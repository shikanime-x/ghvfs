use octocrab::Octocrab;
use serde_json::Value;
use std::sync::OnceLock;

static CLIENT: OnceLock<Octocrab> = OnceLock::new();

fn token() -> Result<String, String> {
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .map(|t| t.trim().to_string())
        .ok()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "set GH_TOKEN (or GITHUB_TOKEN) to a GitHub token".to_string())
}

pub fn client() -> Result<&'static Octocrab, String> {
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let t = token()?;
    let mut b = Octocrab::builder().personal_token(t);
    let api = std::env::var("GHVFS_API").unwrap_or_default();
    if !api.is_empty() {
        b = b.base_uri(api).map_err(|e| format!("GHVFS_API: {e}"))?;
    }
    let c = b.build().map_err(|e| format!("octocrab init: {e}"))?;
    let _ = CLIENT.set(c);
    Ok(CLIENT.get().unwrap())
}

/// Raw REST GET returning the parsed JSON body. `repo` names are `org/name`.
pub async fn get(repo: &str, path: &str) -> Result<Value, String> {
    let c = client()?;
    c.get::<Value, _, _>(format!("/repos/{repo}/{path}"), None::<&str>)
        .await
        .map_err(|e| e.to_string())
}

/// Raw GraphQL call, returning the `data` payload as JSON.
pub async fn graphql(query: &str, vars: Value) -> Result<Value, String> {
    let c = client()?;
    let body = serde_json::json!({"query": query, "variables": vars});
    let resp: Value = c
        .post("/graphql", Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(errors) = resp.get("errors") {
        return Err(format!("graphql: {errors}"));
    }
    Ok(resp.get("data").cloned().unwrap_or(Value::Null))
}

/// REST PATCH with a JSON body.
pub async fn patch(repo: &str, path: &str, body: Value) -> Result<(), String> {
    let c = client()?;
    c.patch::<Value, _, _>(format!("/repos/{repo}/{path}"), Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Core rate-limit snapshot as (remaining, limit).
pub async fn rate_core() -> Result<(u64, u64), String> {
    let c = client()?;
    let rl = c.ratelimit().get().await.map_err(|e| e.to_string())?;
    Ok((
        rl.resources.core.remaining as u64,
        rl.resources.core.limit as u64,
    ))
}

/// Paginated REST GET collecting every page of a list endpoint.
pub async fn get_all(repo: &str, path: &str) -> Result<Vec<Value>, String> {
    let c = client()?;
    let mut out = Vec::new();
    let mut page = 1u32;
    let sep = if path.contains('?') { '&' } else { '?' };
    loop {
        let batch: Vec<Value> = c
            .get(
                format!("/repos/{repo}/{path}{sep}per_page=100&page={page}"),
                None::<&str>,
            )
            .await
            .map_err(|e| e.to_string())?;
        let n = batch.len();
        out.extend(batch);
        if n < 100 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// Issue list including PRs (the issues API returns both); caller filters.
pub async fn issues_all(repo: &str, since: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut page = 1u32;
    loop {
        let c = client()?;
        let batch: Vec<Value> = c
            .get(
                format!("/repos/{repo}/issues"),
                Some(&[
                    ("state", "all"),
                    ("since", since),
                    ("per_page", "100"),
                    ("page", &page.to_string()),
                ]),
            )
            .await
            .map_err(|e| e.to_string())?;
        let n = batch.len();
        out.extend(batch);
        if n < 100 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_errors_are_actionable() {
        // No GH_TOKEN in test env -> client() must fail with the hint, not panic.
        if std::env::var("GH_TOKEN").is_err() && std::env::var("GITHUB_TOKEN").is_err() {
            assert!(client().is_err());
        }
    }
}
