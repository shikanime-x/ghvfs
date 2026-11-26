use serde_json::Value;
use std::path::PathBuf;

pub struct Record {
    pub number: u64,
    pub kind: &'static str,
    pub title: String,
    pub state: String,
    pub type_name: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub url: String,
    pub body: String,
}

fn field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn nested(v: &Value, a: &str, b: &str) -> String {
    v.get(a)
        .and_then(|x| x.get(b))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn names(v: &Value, key: &str, sub: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    if sub.is_empty() {
                        x.as_str().map(str::to_string)
                    } else {
                        x.get(sub).and_then(Value::as_str).map(str::to_string)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn number(v: &Value) -> u64 {
    v.get("number").and_then(Value::as_u64).unwrap_or(0)
}

pub fn parse_issue(v: &Value) -> Record {
    Record {
        number: number(v),
        kind: "issues",
        title: field(v, "title"),
        state: field(v, "state"),
        type_name: nested(v, "type", "name"),
        labels: names(v, "labels", "name"),
        assignees: names(v, "assignees", "login"),
        milestone: nested(v, "milestone", "title"),
        author: nested(v, "user", "login"),
        created_at: field(v, "created_at"),
        updated_at: field(v, "updated_at"),
        url: field(v, "html_url"),
        body: field(v, "body"),
    }
}

pub fn parse_pr(v: &Value) -> Record {
    Record {
        number: number(v),
        kind: "pr",
        title: field(v, "title"),
        state: field(v, "state"),
        type_name: if v.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            "draft".into()
        } else {
            "ready".into()
        },
        labels: names(v, "labels", "name"),
        assignees: names(v, "assignee", "login"),
        milestone: nested(v, "milestone", "title"),
        author: nested(v, "user", "login"),
        created_at: field(v, "created_at"),
        updated_at: field(v, "updated_at"),
        url: field(v, "html_url"),
        body: field(v, "body"),
    }
}

pub fn parse_discussion(v: &Value) -> Record {
    Record {
        number: number(v),
        kind: "discussion",
        title: field(v, "title"),
        state: String::new(),
        type_name: String::new(),
        labels: Vec::new(),
        assignees: Vec::new(),
        milestone: String::new(),
        author: nested(v, "author", "login"),
        created_at: field(v, "createdAt"),
        updated_at: field(v, "updatedAt"),
        url: field(v, "url"),
        body: field(v, "body"),
    }
}

pub fn repo_parts(repo: &str) -> Result<(&str, &str), String> {
    repo.split_once('/')
        .ok_or_else(|| "repo must be <org>/<name>".into())
}

pub fn record_dir(
    root: &std::path::Path,
    repo: &str,
    kind: &str,
    number: u64,
) -> Result<PathBuf, String> {
    let (org, name) = repo_parts(repo)?;
    Ok(root
        .join(org)
        .join(format!("{name}.{kind}"))
        .join(number.to_string()))
}

pub fn doc_name(kind: &str) -> &'static str {
    match kind {
        "pr" => "pr.md",
        "discussion" => "discussion.md",
        _ => "issue.md",
    }
}

#[cfg(test)]
pub fn sample() -> Record {
    Record {
        number: 7,
        kind: "issues",
        title: "Fix login flow".into(),
        state: "open".into(),
        type_name: "Feature".into(),
        labels: vec!["bug".into()],
        assignees: vec![],
        milestone: String::new(),
        author: "shikanime".into(),
        created_at: "2026-11-26T00:00:00Z".into(),
        updated_at: "2026-11-26T00:00:00Z".into(),
        url: "https://example.test/7".into(),
        body: "Body text".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_paths_are_namespace_scoped() {
        let p = record_dir(std::path::Path::new("/r"), "shikanime-x/ghvfs", "pr", 7).unwrap();
        assert_eq!(p, std::path::Path::new("/r/shikanime-x/ghvfs.pr/7"));
        assert_eq!(doc_name("discussion"), "discussion.md");
    }
}
