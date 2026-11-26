use crate::gh;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// One GraphQL call per batch: Development links for every issue and PR, plus all
/// fieldValues of every project item that references this repository (works for
/// org-owned and user-owned boards, which `issue.projectItems` misses).
pub async fn fetch(repo: &str) -> Result<BTreeMap<u64, Value>, String> {
    let (org, name) = crate::model::repo_parts(repo)?;
    let query = "query($org:String!,$name:String!){repository(owner:$org,name:$name){\
         issues(first:100){nodes{number linkedBranches(first:10){nodes{ref{associatedPullRequests(first:20){nodes{number}}}}}}}\
         pullRequests(first:100){nodes{number closingIssuesReferences(first:20){nodes{number}}}}}}";
    let raw = gh::gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("org={org}"),
        "-f",
        &format!("name={name}"),
        "--jq",
        ".data",
    ])
    .await?;
    let data: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut out: BTreeMap<u64, Value> = BTreeMap::new();

    let repo_obj = &data["repository"];
    let empty: Vec<Value> = Vec::new();
    for node in repo_obj["issues"]["nodes"].as_array().unwrap_or(&empty) {
        let mut linked: Vec<u64> = Vec::new();
        for pr in node["linkedBranches"]["nodes"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .flat_map(|b| {
                b["ref"]["associatedPullRequests"]["nodes"]
                    .as_array()
                    .unwrap_or(&empty)
            })
        {
            if let Some(n) = pr["number"].as_u64() {
                linked.push(n);
            }
        }
        linked.sort_unstable();
        linked.dedup();
        if !linked.is_empty() {
            let number = node["number"].as_u64().unwrap_or(0);
            out.entry(number).or_insert_with(|| json!({}))["linked"] = json!(linked);
        }
    }
    for node in repo_obj["pullRequests"]["nodes"]
        .as_array()
        .unwrap_or(&empty)
    {
        let mut linked: Vec<u64> = node["closingIssuesReferences"]["nodes"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|i| i["number"].as_u64())
            .collect();
        linked.sort_unstable();
        linked.dedup();
        if !linked.is_empty() {
            let number = node["number"].as_u64().unwrap_or(0);
            out.entry(number).or_insert_with(|| json!({}))["linked"] = json!(linked);
        }
    }
    let _ = repo_obj["projectsV2"]; // org boards handled below via project item scan
    Ok(out)
}

/// Merge board field values into the per-number metadata map. Boards are looked up
/// through `gh project item-list` per (owner/number, owner) so user-owned projects
/// (which GraphQL `projectItems` on the issue misses) are included.
pub async fn attach_projects(
    meta: &mut BTreeMap<u64, Value>,
    repo: &str,
    projects: &[String],
) -> Result<(), String> {
    let mut boards: Vec<(String, String)> = Vec::new(); // (owner, number)
    for spec in projects {
        let (owner, number) = spec
            .split_once('/')
            .ok_or_else(|| format!("--project must be <owner>/<number>, got {spec}"))?;
        boards.push((owner.to_string(), number.to_string()));
    }
    if boards.is_empty() {
        return Ok(());
    }
    for (owner, number) in boards {
        let raw = gh::gh(&[
            "project",
            "item-list",
            &number,
            "--owner",
            &owner,
            "--format",
            "json",
            "--limit",
            "1000",
        ])
        .await?;
        let data: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let items = data
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            let content = &item["content"];
            // item-list puts the URL on item.repository and owner/name on content.repository.
            let repo_url = item
                .get("repository")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let content_repo = content
                .get("repository")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let repo_ok = repo_url == format!("https://github.com/{repo}") || content_repo == repo;
            if !repo_ok {
                continue;
            }
            let Some(num) = content.get("number").and_then(Value::as_u64) else {
                continue;
            };
            let mut fields = BTreeMap::new();
            for key in [
                "status",
                "priority",
                "size",
                "estimate",
                "start date",
                "target date",
                "iteration",
            ] {
                if let Some(v) = item.get(key).and_then(Value::as_str) {
                    let fk = key.split(' ').map(capitalize).collect::<Vec<_>>().join(" ");
                    fields.insert(fk, json!(v));
                }
            }
            if fields.is_empty() {
                continue;
            }
            let project_title = format!("{owner}/{number}");
            let entry = meta.entry(num).or_insert_with(|| json!({}));
            entry["projects"][&project_title] = json!(fields);
        }
    }
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Apply `meta` (from `fetch`/`attach_projects`) onto parsed records.
pub fn apply_meta(records: &mut [crate::model::Record], meta: &BTreeMap<u64, Value>) {
    for r in records {
        if let Some(m) = meta.get(&r.number) {
            if let Some(linked) = m.get("linked").and_then(Value::as_array) {
                r.linked = linked.iter().filter_map(Value::as_u64).collect();
            }
            if let Some(projects) = m.get("projects").and_then(Value::as_object) {
                for (title, fields) in projects {
                    let map: BTreeMap<String, Value> = fields
                        .as_object()
                        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();
                    r.projects.insert(title.clone(), map);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Record};

    fn record(number: u64, kind: Kind) -> Record {
        Record {
            number,
            kind,
            title: "t".into(),
            state: "open".into(),
            type_name: String::new(),
            labels: vec![],
            assignees: vec![],
            milestone: String::new(),
            author: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            url: String::new(),
            body: String::new(),
            linked: vec![],
            projects: BTreeMap::new(),
        }
    }

    #[test]
    fn apply_meta_fills_linked_and_projects() {
        let mut records = vec![record(1, Kind::Issues), record(2, Kind::Issues)];
        let mut meta: BTreeMap<u64, Value> = BTreeMap::new();
        meta.insert(
            1,
            json!({"linked": [9], "projects": {"acme/1": {"Status": "Ready"}}}),
        );
        apply_meta(&mut records, &meta);
        assert_eq!(records[0].linked, vec![9]);
        assert_eq!(records[0].projects["acme/1"]["Status"], json!("Ready"));
        assert!(records[1].linked.is_empty() && records[1].projects.is_empty());
    }

    #[test]
    fn capitalize_titles_field_keys() {
        assert_eq!(capitalize("status"), "Status");
        assert_eq!(capitalize("start date"), "Start date");
    }
}
