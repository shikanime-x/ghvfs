use crate::{db, doc, gh, model};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::{env, path::Path};

pub async fn sync(repo: &str, root: &Path, pool: &SqlitePool) -> Result<(), String> {
    let (org, name) = model::repo_parts(repo)?;
    for kind in ["issues", "pr", "discussion"] {
        tokio::fs::create_dir_all(root.join(org).join(format!("{name}.{kind}")))
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
        tokio::fs::write(
            dir.join(model::doc_name(record.kind)),
            doc::front_matter(record),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    let entries: Vec<Value> = records.iter().map(doc::entry).collect();
    db::replace(pool, repo, &entries).await?;
    println!(
        "synced {} issues, {} prs, {} discussions",
        records.iter().filter(|r| r.kind == "issues").count(),
        records.iter().filter(|r| r.kind == "pr").count(),
        records.iter().filter(|r| r.kind == "discussion").count()
    );
    Ok(())
}

pub async fn apply(
    root: &Path,
    pool: &SqlitePool,
    commit: bool,
    force: bool,
) -> Result<(), String> {
    let (repo, mut entries) = db::load(pool).await?;
    let mut changed = 0;
    for e in entries.iter_mut() {
        let number = e.get("number").and_then(Value::as_u64).unwrap_or(0);
        let kind = e
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // GraphQL discussions have no supported edit endpoint here.
        if kind == "discussion" {
            continue;
        }
        let old_title = e
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let old_body = e
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let doc_path = model::record_dir(root, &repo, &kind, number)?.join(model::doc_name(&kind));
        let document = tokio::fs::read_to_string(&doc_path)
            .await
            .map_err(|e| e.to_string())?;
        if doc::readonly_lines(&document) != doc::expected_readonly(e) {
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
        if remote != e.get("updated_at").and_then(Value::as_str).unwrap_or("") && !force {
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
        db::update_title(pool, &repo, &kind, number, &title, &new_body, &remote).await?;
    }
    println!("apply: {changed} changed");
    Ok(())
}
