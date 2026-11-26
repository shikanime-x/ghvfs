use serde_json::{Value, json};
use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Rows per transaction. ponytail: bounds memory and WAL churn on large repos; raise if profiling shows a bottleneck.
const BATCH: usize = 50;

pub fn state_path(root: &Path) -> PathBuf {
    root.join(".ghvfs").join("state.sqlite3")
}

pub async fn open(root: &Path) -> Result<SqlitePool, String> {
    tokio::fs::create_dir_all(root.join(".ghvfs"))
        .await
        .map_err(|e| e.to_string())?;
    let opts = SqliteConnectOptions::new()
        .filename(state_path(root))
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    migrate_legacy(&pool, root).await?;
    Ok(pool)
}

fn text(e: &Value, key: &str) -> String {
    e.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

async fn migrate_legacy(pool: &SqlitePool, root: &Path) -> Result<(), String> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok(());
    }
    let legacy = root.join(".ghvfs").join("state.json");
    if !legacy.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&legacy).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let repo = text(&v, "repo");
    let records = v
        .get("records")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    replace(pool, &repo, &records).await?;
    fs::remove_file(legacy).ok();
    Ok(())
}

pub async fn replace(pool: &SqlitePool, repo: &str, records: &[Value]) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM records WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    for chunk in records.chunks(BATCH) {
        for e in chunk {
            sqlx::query(
                "INSERT INTO records (repo, kind, number, title, body, updated_at, readonly)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(repo)
            .bind(text(e, "kind"))
            .bind(e.get("number").and_then(Value::as_i64).unwrap_or(0))
            .bind(text(e, "title"))
            .bind(text(e, "body"))
            .bind(text(e, "updated_at"))
            .bind(e.get("readonly").map(Value::to_string).unwrap_or_default())
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().await.map_err(|e| e.to_string())
}

pub async fn update_title(
    pool: &SqlitePool,
    repo: &str,
    kind: &str,
    number: u64,
    title: &str,
    body: &str,
    updated_at: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE records SET title = ?, body = ?, updated_at = ?
         WHERE repo = ? AND kind = ? AND number = ?",
    )
    .bind(title)
    .bind(body)
    .bind(updated_at)
    .bind(repo)
    .bind(kind)
    .bind(number as i64)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn load(pool: &SqlitePool) -> Result<(String, Vec<Value>), String> {
    let rows =
        sqlx::query("SELECT repo, kind, number, title, body, updated_at, readonly FROM records")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    let mut repo = String::new();
    let mut entries = Vec::new();
    for row in rows {
        let r: String = row.try_get(0).map_err(|e| e.to_string())?;
        let kind: String = row.try_get(1).map_err(|e| e.to_string())?;
        let number: i64 = row.try_get(2).map_err(|e| e.to_string())?;
        let title: String = row.try_get(3).map_err(|e| e.to_string())?;
        let body: String = row.try_get(4).map_err(|e| e.to_string())?;
        let updated_at: String = row.try_get(5).map_err(|e| e.to_string())?;
        let readonly: String = row.try_get(6).map_err(|e| e.to_string())?;
        let ro = serde_json::from_str(&readonly).unwrap_or_else(|_| json!({}));
        if repo.is_empty() {
            repo = r;
        }
        entries.push(json!({
            "number": number,
            "kind": kind,
            "title": title,
            "body": body,
            "updated_at": updated_at,
            "readonly": ro
        }));
    }
    Ok((repo, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{doc, model::sample};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ghvfs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[tokio::test]
    async fn sqlite_state_round_trips() {
        let root = temp_root("sqlite");
        let pool = open(&root).await.unwrap();
        let records = vec![doc::entry(&sample())];
        replace(&pool, "acme/demo", &records).await.unwrap();
        assert!(state_path(&root).exists());
        let (repo, rows) = load(&pool).await.unwrap();
        assert_eq!(repo, "acme/demo");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["title"], "Fix login flow");
        assert_eq!(
            doc::readonly_lines(&doc::front_matter(&sample())),
            doc::expected_readonly(&rows[0])
        );
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn legacy_json_state_is_migrated() {
        let root = temp_root("legacy");
        tokio::fs::create_dir_all(root.join(".ghvfs"))
            .await
            .unwrap();
        let legacy = json!({ "repo": "acme/demo", "records": [doc::entry(&sample())] });
        tokio::fs::write(root.join(".ghvfs/state.json"), legacy.to_string())
            .await
            .unwrap();
        let pool = open(&root).await.unwrap();
        let (repo, rows) = load(&pool).await.unwrap();
        assert_eq!(repo, "acme/demo");
        assert_eq!(rows.len(), 1);
        assert!(!root.join(".ghvfs/state.json").exists());
        fs::remove_dir_all(&root).ok();
    }
}
