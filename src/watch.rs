use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

fn is_doc(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("issue.md" | "pr.md" | "discussion.md")
    )
}

fn watch_blocking(root: PathBuf, handle: Handle, commit: bool, force: bool) -> Result<(), String> {
    let pool = handle.block_on(crate::db::open(&root))?;
    let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    println!("watching {}", root.display());
    // ponytail: fixed 500 ms debounce absorbs editor save bursts; raise it if a writer emits slower duplicate events.
    let mut last = Instant::now() - Duration::from_millis(500);
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(event))
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
                    && event.paths.iter().any(|p| is_doc(p))
                    && last.elapsed() >= Duration::from_millis(500) =>
            {
                last = Instant::now();
                if let Err(e) = handle.block_on(crate::ops::apply(&root, &pool, commit, force)) {
                    eprintln!("ghvfs: apply failed: {e}");
                }
            }
            Ok(Err(e)) => eprintln!("ghvfs: watch event error: {e}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    Ok(())
}

pub async fn watch(root: PathBuf, commit: bool, force: bool) -> Result<(), String> {
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || watch_blocking(root, handle, commit, force))
        .await
        .map_err(|e| e.to_string())?
}

struct RepoState {
    since: String,
    interval: u64,
    next: Instant,
}

async fn rate_budget() -> Result<(u64, u64), String> {
    let raw = crate::gh::gh(&["api", "rate_limit", "--jq", ".resources.core"]).await?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let remaining = v
        .get("remaining")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5000);
    let limit = v
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5000);
    Ok((remaining, limit))
}

fn budget_factor(remaining: u64, limit: u64) -> u64 {
    if remaining == 0 {
        16
    } else if remaining * 4 < limit {
        4
    } else if remaining * 2 < limit {
        2
    } else {
        1
    }
}

pub async fn watch_pull(repos: &[String], root: &Path, base: u64) -> Result<(), String> {
    if repos.is_empty() {
        return Err("watch --pull needs --repo <org/name>".into());
    }
    let pool = crate::db::open(root).await?;
    let (_, rows) = crate::db::load(&pool).await?;
    let initial = rows
        .iter()
        .map(|s| s.updated_at.as_str())
        .max()
        .unwrap_or("1970-01-01T00:00:00Z")
        .to_string();
    let mut states: HashMap<String, RepoState> = repos
        .iter()
        .map(|repo| {
            (
                repo.clone(),
                RepoState {
                    since: initial.clone(),
                    interval: base,
                    next: Instant::now(),
                },
            )
        })
        .collect();
    let mut budget = (Instant::now() - Duration::from_secs(3600), 1u64);
    println!("watching {} repos, {base}s base interval", repos.len());
    loop {
        if budget.0.elapsed() >= Duration::from_secs(60) {
            match rate_budget().await {
                Ok((remaining, limit)) => {
                    budget = (Instant::now(), budget_factor(remaining, limit))
                }
                Err(e) => eprintln!("ghvfs: rate limit check failed: {e}"),
            }
        }
        let now = Instant::now();
        for (repo, state) in &mut states {
            if now < state.next {
                continue;
            }
            match crate::ops::pull(repo, root, &pool, &state.since, &[]).await {
                Ok((next, changed)) => {
                    state.since = next;
                    state.interval = if changed > 0 {
                        (state.interval / 2).max(5)
                    } else {
                        (state.interval * 2).min(base * 8)
                    };
                }
                Err(e) => {
                    eprintln!("ghvfs: {repo} pull failed: {e}");
                    state.interval = (state.interval * 2).min(base * 8);
                }
            }
            state.next = Instant::now() + Duration::from_secs(state.interval * budget.1);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_filters_doc_edits() {
        assert!(is_doc(Path::new("/r/org/repo.issues/1/issue.md")));
        assert!(is_doc(Path::new("/r/org/repo.pr/6/pr.md")));
        assert!(!is_doc(Path::new("/r/.ghvfs/state.sqlite3")));
    }

    #[test]
    fn budget_factor_scales_with_remaining_requests() {
        assert_eq!(budget_factor(5000, 5000), 1);
        assert_eq!(budget_factor(4000, 5000), 1);
        assert_eq!(budget_factor(2000, 5000), 2);
        assert_eq!(budget_factor(1000, 5000), 4);
        assert_eq!(budget_factor(0, 5000), 16);
    }
}
