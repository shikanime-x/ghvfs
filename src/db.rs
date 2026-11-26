use crate::model::{Kind, Snapshot};
use sqlx::{FromRow, SqlitePool, sqlite::SqliteConnectOptions};
use std::path::{Path, PathBuf};

/// Rows per transaction. ponytail: bounds memory and WAL churn on large repos; raise if profiling shows a bottleneck.
const BATCH: usize = 50;

#[derive(Debug, FromRow)]
struct Row {
    repo: String,
    kind: String,
    number: i64,
    title: String,
    body: String,
    updated_at: String,
    readonly: String,
}

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
    Ok(pool)
}

const INSERT: &str = "INSERT INTO records (repo, kind, number, title, body, updated_at, readonly)
     VALUES (?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (repo, kind, number) DO UPDATE SET
         title = excluded.title,
         body = excluded.body,
         updated_at = excluded.updated_at,
         readonly = excluded.readonly";

async fn write_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    repo: &str,
    records: &[Snapshot],
) -> Result<(), String> {
    for chunk in records.chunks(BATCH) {
        for s in chunk {
            sqlx::query(INSERT)
                .bind(repo)
                .bind(s.kind.as_str())
                .bind(s.number as i64)
                .bind(&s.title)
                .bind(&s.body)
                .bind(&s.updated_at)
                .bind(serde_json::to_string(&s.readonly).map_err(|e| e.to_string())?)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub async fn replace(pool: &SqlitePool, repo: &str, records: &[Snapshot]) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM records WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    write_rows(&mut tx, repo, records).await?;
    tx.commit().await.map_err(|e| e.to_string())
}

pub async fn upsert(pool: &SqlitePool, repo: &str, records: &[Snapshot]) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    write_rows(&mut tx, repo, records).await?;
    tx.commit().await.map_err(|e| e.to_string())
}

pub async fn remove_repo(pool: &SqlitePool, repo: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM records WHERE repo = ?")
        .bind(repo)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn update_title(
    pool: &SqlitePool,
    repo: &str,
    kind: Kind,
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
    .bind(kind.as_str())
    .bind(number as i64)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn load(pool: &SqlitePool) -> Result<(String, Vec<Snapshot>), String> {
    let rows = sqlx::query_as::<_, Row>(
        "SELECT repo, kind, number, title, body, updated_at, readonly FROM records",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut repo = String::new();
    let mut entries = Vec::new();
    for row in rows {
        if repo.is_empty() {
            repo = row.repo;
        }
        entries.push(Snapshot {
            number: row.number as u64,
            kind: Kind::parse(&row.kind)?,
            title: row.title,
            body: row.body,
            updated_at: row.updated_at,
            readonly: serde_json::from_str(&row.readonly).map_err(|e| e.to_string())?,
        });
    }
    Ok((repo, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        doc,
        model::{ToSnapshot, sample},
    };

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ghvfs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[tokio::test]
    async fn sqlite_state_round_trips() {
        let root = temp_root("sqlite");
        let pool = open(&root).await.unwrap();
        let records = vec![sample().to_snapshot()];
        replace(&pool, "acme/demo", &records).await.unwrap();
        assert!(state_path(&root).exists());
        let (repo, rows) = load(&pool).await.unwrap();
        assert_eq!(repo, "acme/demo");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Fix login flow");
        assert!(doc::matches_readonly(
            &doc::front_matter(&sample()),
            &rows[0]
        ));
        std::fs::remove_dir_all(&root).ok();
    }
}
