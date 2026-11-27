use octocrab::Octocrab;
use serde_json::Value;
use std::sync::OnceLock;

static CLIENT: OnceLock<Octocrab> = OnceLock::new();

/// Where the GitHub token comes from. `Env` reads GH_TOKEN/GITHUB_TOKEN;
/// `Gh` resolves a named account from the `gh` CLI keyring (`gh auth token
/// --user <u>`) and overrides env when set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Auth {
    Env,
    Gh(&'static str),
}

impl Auth {
    pub fn resolve(self) -> Result<String, String> {
        match self {
            Auth::Env => std::env::var("GH_TOKEN")
                .or_else(|_| std::env::var("GITHUB_TOKEN"))
                .map(|t| t.trim().to_string())
                .ok()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| "set GH_TOKEN (or GITHUB_TOKEN) to a GitHub token".to_string()),
            Auth::Gh(user) => std::process::Command::new("gh")
                .args(["auth", "token", "--user", user])
                .output()
                .map_err(|e| format!("gh keyring lookup: {e}"))
                .and_then(|o| {
                    if o.status.success() {
                        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if t.is_empty() {
                            Err(format!("gh auth token --user {user}: empty token"))
                        } else {
                            Ok(t)
                        }
                    } else {
                        Err(format!(
                            "gh auth token --user {user}: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        ))
                    }
                }),
        }
    }
}

/// Auth selection: --gh-user <account> wins, else env vars.
pub fn init(auth: Auth) -> Result<(), String> {
    let t = auth.resolve()?;
    let mut b = Octocrab::builder().personal_token(t);
    let api = std::env::var("GHVFS_API").unwrap_or_default();
    if !api.is_empty() {
        b = b.base_uri(api).map_err(|e| format!("GHVFS_API: {e}"))?;
    }
    let c = b.build().map_err(|e| format!("octocrab init: {e}"))?;
    CLIENT
        .set(c)
        .map_err(|_| "github client already initialised".to_string())
}

/// Raw REST GET returning the parsed JSON body. `repo` names are `org/name`.
pub async fn get(repo: &str, path: &str) -> Result<Value, String> {
    let c = CLIENT.get().ok_or("github client not initialised")?;
    c.get::<Value, _, _>(format!("/repos/{repo}/{path}"), None::<&str>)
        .await
        .map_err(|e| e.to_string())
}

/// Raw GraphQL call, returning the `data` payload as JSON.
pub async fn graphql(query: &str, vars: Value) -> Result<Value, String> {
    let c = CLIENT.get().ok_or("github client not initialised")?;
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
    let c = CLIENT.get().ok_or("github client not initialised")?;
    c.patch::<Value, _, _>(format!("/repos/{repo}/{path}"), Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Core rate-limit snapshot as (remaining, limit).
pub async fn rate_core() -> Result<(u64, u64), String> {
    let c = CLIENT.get().ok_or("github client not initialised")?;
    let rl = c.ratelimit().get().await.map_err(|e| e.to_string())?;
    Ok((
        rl.resources.core.remaining as u64,
        rl.resources.core.limit as u64,
    ))
}

/// Paginated REST GET collecting every page of a list endpoint.
pub async fn get_all(repo: &str, path: &str) -> Result<Vec<Value>, String> {
    let c = CLIENT.get().ok_or("github client not initialised")?;
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
        let c = CLIENT.get().ok_or("github client not initialised")?;
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
        // No GH_TOKEN in test env -> env auth must fail with the hint, not panic.
        if std::env::var("GH_TOKEN").is_err() && std::env::var("GITHUB_TOKEN").is_err() {
            assert_eq!(
                Auth::Env.resolve().unwrap_err(),
                "set GH_TOKEN (or GITHUB_TOKEN) to a GitHub token"
            );
        }
    }

    #[test]
    fn gh_strategy_surfaces_keyring_failure() {
        // A nonexistent keyring account must produce a named error, not fall back to env.
        if std::process::Command::new("gh").output().is_err() {
            return; // no gh binary in this environment
        }
        let err = Auth::Gh("ghvfs-no-such-account-3f9d")
            .resolve()
            .unwrap_err();
        assert!(err.contains("ghvfs-no-such-account-3f9d"), "{err}");
    }
}
