use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::{
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_filters_doc_edits() {
        assert!(is_doc(Path::new("/r/org/repo.issues/1/issue.md")));
        assert!(is_doc(Path::new("/r/org/repo.pr/6/pr.md")));
        assert!(!is_doc(Path::new("/r/.ghvfs/state.sqlite3")));
    }
}
