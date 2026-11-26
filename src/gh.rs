use serde_json::Value;
use std::env;

pub async fn gh(args: &[&str]) -> Result<String, String> {
    let program = env::var("GHVFS_GH").unwrap_or_else(|_| "gh".to_string());
    let out = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("spawn gh: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

pub fn json_lines(raw: &str) -> Result<Vec<Value>, String> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
