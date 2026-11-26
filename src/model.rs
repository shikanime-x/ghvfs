use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Issues,
    Pr,
    Discussion,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Issues => "issues",
            Kind::Pr => "pr",
            Kind::Discussion => "discussion",
        }
    }

    pub fn doc_name(self) -> &'static str {
        match self {
            Kind::Pr => "pr.md",
            Kind::Discussion => "discussion.md",
            Kind::Issues => "issue.md",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "issues" => Ok(Kind::Issues),
            "pr" => Ok(Kind::Pr),
            "discussion" => Ok(Kind::Discussion),
            other => Err(format!("unknown kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub number: u64,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub updated_at: String,
    pub readonly: Value,
}

pub trait ToSnapshot {
    fn to_snapshot(&self) -> Snapshot;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub number: u64,
    pub kind: Kind,
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
    /// Development-section links (linked PRs for issues, closing issues for PRs).
    pub linked: Vec<u64>,
    /// Projects V2 board data: project title -> {field: value}.
    pub projects: BTreeMap<String, BTreeMap<String, Value>>,
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
        kind: Kind::Issues,
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
        linked: Vec::new(),
        projects: BTreeMap::new(),
    }
}

pub fn parse_pr(v: &Value) -> Record {
    Record {
        number: number(v),
        kind: Kind::Pr,
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
        linked: Vec::new(),
        projects: BTreeMap::new(),
    }
}

pub fn parse_discussion(v: &Value) -> Record {
    Record {
        number: number(v),
        kind: Kind::Discussion,
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
        linked: Vec::new(),
        projects: BTreeMap::new(),
    }
}

pub fn repo_parts(repo: &str) -> Result<(&str, &str), String> {
    repo.split_once('/')
        .ok_or_else(|| "repo must be <org>/<name>".into())
}

pub fn record_dir(
    root: &std::path::Path,
    repo: &str,
    kind: Kind,
    number: u64,
) -> Result<PathBuf, String> {
    let (org, name) = repo_parts(repo)?;
    Ok(root
        .join(org)
        .join(format!("{name}.{}", kind.as_str()))
        .join(number.to_string()))
}

#[cfg(test)]
pub fn sample() -> Record {
    Record {
        number: 7,
        kind: Kind::Issues,
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
        linked: Vec::new(),
        projects: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_paths_are_namespace_scoped() {
        let p = record_dir(std::path::Path::new("/r"), "shikanime-x/ghvfs", Kind::Pr, 7).unwrap();
        assert_eq!(p, std::path::Path::new("/r/shikanime-x/ghvfs.pr/7"));
        assert_eq!(Kind::Discussion.doc_name(), "discussion.md");
    }

    #[test]
    fn kind_round_trips_through_serde() {
        let k: Kind = serde_json::from_str("\"pr\"").unwrap();
        assert_eq!(k, Kind::Pr);
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"pr\"");
    }
}
