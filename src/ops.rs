use crate::{
    db, doc, gh,
    model::{self, Kind, Snapshot, ToSnapshot},
};
use serde_json::json;
use sqlx::SqlitePool;
use std::{collections::HashMap, env, path::Path};

pub async fn sync(repo: &str, root: &Path, pool: &SqlitePool) -> Result<(), String> {
    let (org, name) = model::repo_parts(repo)?;
    for kind in [Kind::Issues, Kind::Pr, Kind::Discussion] {
        tokio::fs::create_dir_all(root.join(org).join(format!("{name}.{}", kind.as_str())))
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut records = Vec::new();

    let raw = gh::gh(&[
        "api",
        &format!("repos/{repo}/issues?per_page=100"),
        "--paginate",
        "--jq",
        ".[] | select(.pull_request == null)",
    ])
    .await?;
    records.extend(gh::json_lines(&raw)?.iter().map(model::parse_issue));

    let raw = gh::gh(&[
        "api",
        &format!("repos/{repo}/pulls?per_page=100"),
        "--paginate",
        "--jq",
        ".[]",
    ])
    .await?;
    records.extend(gh::json_lines(&raw)?.iter().map(model::parse_pr));

    let query = "query($owner:String!,$name:String!){repository(owner:$owner,name:$name){discussions(first:100){nodes{number title body createdAt updatedAt url author{login}}}}}";
    let raw = gh::gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={org}"),
        "-f",
        &format!("name={name}"),
        "--jq",
        ".data.repository.discussions.nodes[]?",
    ])
    .await?;
    records.extend(gh::json_lines(&raw)?.iter().map(model::parse_discussion));

    for record in &records {
        let dir = model::record_dir(root, repo, record.kind, record.number)?;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(dir.join(record.kind.doc_name()), doc::front_matter(record))
            .await
            .map_err(|e| e.to_string())?;
    }
    let entries: Vec<Snapshot> = records.iter().map(ToSnapshot::to_snapshot).collect();
    db::replace(pool, repo, &entries).await?;
    println!(
        "synced {} issues, {} prs, {} discussions",
        records.iter().filter(|r| r.kind == Kind::Issues).count(),
        records.iter().filter(|r| r.kind == Kind::Pr).count(),
        records
            .iter()
            .filter(|r| r.kind == Kind::Discussion)
            .count()
    );
    Ok(())
}

pub async fn apply(
    root: &Path,
    pool: &SqlitePool,
    commit: bool,
    force: bool,
) -> Result<(), String> {
    let (repo, entries) = db::load(pool).await?;
    let mut changed = 0;
    for e in &entries {
        let number = e.number;
        let kind = e.kind;
        // GraphQL discussions have no supported edit endpoint here.
        if kind == Kind::Discussion {
            continue;
        }
        let old_title = e.title.as_str();
        let old_body = e.body.as_str();
        let doc_path = model::record_dir(root, &repo, kind, number)?.join(kind.doc_name());
        let document = tokio::fs::read_to_string(&doc_path)
            .await
            .map_err(|e| e.to_string())?;
        if !doc::matches_readonly(&document, e) {
            return Err(format!(
                "#{number} changes read-only metadata; only title and body are writable"
            ));
        }
        let (title, body) = doc::parse_doc(&document)
            .ok_or_else(|| format!("#{number} has malformed front matter"))?;
        let body = body.trim_end().to_string();
        if title == old_title && body == old_body.trim_end() {
            continue;
        }
        changed += 1;
        let remote = gh::gh(&[
            "api",
            &format!("repos/{repo}/issues/{number}"),
            "--jq",
            ".updated_at",
        ])
        .await?
        .trim()
        .to_string();
        if remote != e.updated_at && !force {
            return Err(format!(
                "#{number} changed remotely; pass --force to overwrite"
            ));
        }
        if !commit {
            println!("#{number}: would PATCH title and body");
            continue;
        }
        let mut patch = json!({});
        if title != old_title {
            patch["title"] = json!(title.clone());
        }
        if body != old_body.trim_end() {
            patch["body"] = json!(format!("{body}\n"));
        }
        let path = env::temp_dir().join(format!("ghvfs-{number}.json"));
        tokio::fs::write(&path, patch.to_string())
            .await
            .map_err(|e| e.to_string())?;
        gh::gh(&[
            "api",
            "-X",
            "PATCH",
            &format!("repos/{repo}/issues/{number}"),
            "--input",
            path.to_str().ok_or("bad temp path")?,
        ])
        .await?;
        let new_body = format!("{body}\n");
        db::update_title(pool, &repo, kind, number, &title, &new_body, &remote).await?;
    }
    println!("apply: {changed} changed");
    Ok(())
}

pub async fn pull(
    repo: &str,
    root: &Path,
    pool: &SqlitePool,
    since: &str,
) -> Result<(String, usize), String> {
    let raw = gh::gh(&[
        "api",
        &format!("repos/{repo}/issues?per_page=100&state=all&since={since}"),
        "--paginate",
        "--jq",
        ".[] | select(.pull_request == null)",
    ])
    .await?;
    let mut records: Vec<_> = gh::json_lines(&raw)?
        .iter()
        .map(model::parse_issue)
        .collect();
    let raw = gh::gh(&[
        "api",
        &format!("repos/{repo}/pulls?per_page=100&state=all&since={since}"),
        "--paginate",
        "--jq",
        ".[]",
    ])
    .await?;
    records.extend(gh::json_lines(&raw)?.iter().map(model::parse_pr));

    let (_, snapshots) = db::load(pool).await?;
    let known: HashMap<(Kind, u64), &Snapshot> =
        snapshots.iter().map(|s| ((s.kind, s.number), s)).collect();
    let mut next = since.to_string();
    let mut changed = Vec::new();
    for r in &records {
        if r.updated_at > next {
            next = r.updated_at.clone();
        }
        let path = model::record_dir(root, repo, r.kind, r.number)?.join(r.kind.doc_name());
        if let Some(old) = known.get(&(r.kind, r.number)) {
            if old.updated_at == r.updated_at {
                continue;
            }
            if let Ok(document) = tokio::fs::read_to_string(&path).await {
                let locally_dirty = doc::parse_doc(&document).is_none_or(|(title, body)| {
                    title != old.title
                        || body.trim_end() != old.body.trim_end()
                        || !doc::matches_readonly(&document, old)
                });
                if locally_dirty {
                    eprintln!("#{} has local edits; skipping remote update", r.number);
                    continue;
                }
            }
        }
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&path, doc::front_matter(r))
            .await
            .map_err(|e| e.to_string())?;
        changed.push(r.to_snapshot());
    }
    db::upsert(pool, repo, &changed).await?;
    println!("pull: {} changed", changed.len());
    Ok((next, changed.len()))
}

pub async fn unsync(repo: &str, root: &Path, pool: &SqlitePool) -> Result<(), String> {
    let (org, name) = model::repo_parts(repo)?;
    db::remove_repo(pool, repo).await?;
    for kind in [Kind::Issues, Kind::Pr, Kind::Discussion] {
        let dir = root.join(org).join(format!("{name}.{}", kind.as_str()));
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    println!("unsynced {repo}");
    Ok(())
}
