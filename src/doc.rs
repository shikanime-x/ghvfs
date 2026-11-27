use crate::model::{Kind, Record, Snapshot, ToSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Front {
    number: u64,
    kind: Kind,
    title: String,
    state: String,
    #[serde(rename = "type")]
    type_name: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    milestone: String,
    author: String,
    created_at: String,
    updated_at: String,
    url: String,
    /// Development section: linked numbers. ponytail: bare numbers, not URLs — switch to
    /// objects if titles are ever needed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    linked: Vec<u64>,
    /// Projects V2 boards: project title -> {field: value}. ponytail: values stay raw
    /// serde_json (text/number/date/single-select all differ); normalize if cross-project
    /// diffs matter.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    projects: BTreeMap<String, BTreeMap<String, Value>>,
}

fn front(record: &Record) -> Front {
    Front {
        number: record.number,
        kind: record.kind,
        title: record.title.clone(),
        state: record.state.clone(),
        type_name: record.type_name.clone(),
        labels: record.labels.clone(),
        assignees: record.assignees.clone(),
        milestone: record.milestone.clone(),
        author: record.author.clone(),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        url: record.url.clone(),
        linked: record.linked.clone(),
        projects: record
            .projects
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter().map(|(fk, fv)| (fk.clone(), fv.clone())).collect(),
                )
            })
            .collect(),
    }
}

fn front_value(f: &Front) -> Value {
    let mut v = serde_json::to_value(f).expect("serialize front matter");
    if let Some(map) = v.as_object_mut() {
        map.remove("title");
    }
    v
}

pub fn front_matter(record: &Record) -> String {
    let yaml = serde_yaml::to_string(&front(record)).expect("serialize front matter");
    format!("---\n{yaml}---\n{}\n", record.body)
}

pub fn parse_doc(doc: &str) -> Option<(String, String)> {
    let (head, body) = doc.split_once("\n---\n")?;
    let f: Front = serde_yaml::from_str(head).ok()?;
    Some((f.title, body.to_string()))
}

pub fn matches_readonly(doc: &str, snapshot: &Snapshot) -> bool {
    let Some((head, _)) = doc.split_once("\n---\n") else {
        return false;
    };
    let Ok(f) = serde_yaml::from_str::<Front>(head) else {
        return false;
    };
    front_value(&f) == snapshot.readonly
}

impl ToSnapshot for Record {
    fn to_snapshot(&self) -> Snapshot {
        Snapshot {
            number: self.number,
            kind: self.kind,
            title: self.title.clone(),
            body: self.body.clone(),
            updated_at: self.updated_at.clone(),
            readonly: front_value(&front(self)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    #[test]
    fn front_matter_round_trips_title_and_body() {
        let doc = front_matter(&sample());
        let (title, body) = parse_doc(&doc).unwrap();
        assert_eq!(title, "Fix login flow");
        assert_eq!(body, "Body text\n");
    }

    #[test]
    fn change_detection_and_readonly_guard() {
        let mut doc = front_matter(&sample());
        let s = sample().to_snapshot();
        assert!(matches_readonly(&doc, &s));
        doc = doc.replacen("title: Fix login flow", "title: Fixed login flow", 1);
        assert_eq!(parse_doc(&doc).unwrap().0, "Fixed login flow");
        let mutated = doc.replacen("state: open", "state: closed", 1);
        assert!(!matches_readonly(&mutated, &s));
    }

    #[test]
    fn projects_and_linked_render() {
        let mut r = sample();
        r.linked = vec![9];
        r.projects.insert(
            "Skills".into(),
            [(
                "Status".to_string(),
                serde_json::Value::String("Ready".into()),
            )]
            .into(),
        );
        let doc = front_matter(&r);
        assert!(doc.contains("linked:\n- 9"), "{doc}");
        assert!(
            doc.contains("projects:\n  Skills:\n    Status: Ready"),
            "{doc}"
        );
        // The snapshot render still matches as unmodified front matter.
        assert!(matches_readonly(&doc, &r.to_snapshot()));
    }
}
