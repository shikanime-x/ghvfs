use crate::model::Record;
use serde_json::{Value, json};

pub const READONLY_KEYS: [&str; 11] = [
    "number",
    "kind",
    "state",
    "type",
    "labels",
    "assignees",
    "milestone",
    "author",
    "created_at",
    "updated_at",
    "url",
];

fn yq(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn yq_list(values: &[String]) -> String {
    format!(
        "[{}]",
        values.iter().map(|s| yq(s)).collect::<Vec<_>>().join(", ")
    )
}

pub fn front_matter(r: &Record) -> String {
    // ponytail: flat scalars and lists only; add a YAML crate for nested fields.
    format!(
        "---\nnumber: {}\nkind: {}\ntitle: {}\nstate: {}\ntype: {}\nlabels: {}\nassignees: {}\nmilestone: {}\nauthor: {}\ncreated_at: {}\nupdated_at: {}\nurl: {}\n---\n{}\n",
        r.number,
        yq(r.kind),
        yq(&r.title),
        yq(&r.state),
        yq(&r.type_name),
        yq_list(&r.labels),
        yq_list(&r.assignees),
        yq(&r.milestone),
        yq(&r.author),
        yq(&r.created_at),
        yq(&r.updated_at),
        yq(&r.url),
        r.body
    )
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_doc(doc: &str) -> Option<(String, String)> {
    let (head, body) = doc.split_once("\n---\n")?;
    let title = head
        .lines()
        .find_map(|l| l.strip_prefix("title: ").map(unquote))?;
    Some((title, body.to_string()))
}

pub fn readonly_lines(doc: &str) -> Vec<String> {
    let head = doc.split_once("\n---\n").map(|x| x.0).unwrap_or("");
    head.lines()
        .filter(|l| {
            READONLY_KEYS
                .iter()
                .any(|k| l.starts_with(&format!("{k}:")))
        })
        .map(str::to_string)
        .collect()
}

pub fn entry(r: &Record) -> Value {
    let ro = json!({
        "number": json!(r.number.to_string()),
        "kind": yq(r.kind),
        "state": yq(&r.state),
        "type": yq(&r.type_name),
        "labels": yq_list(&r.labels),
        "assignees": yq_list(&r.assignees),
        "milestone": yq(&r.milestone),
        "author": yq(&r.author),
        "created_at": yq(&r.created_at),
        "updated_at": yq(&r.updated_at),
        "url": yq(&r.url)
    });
    json!({
        "number": r.number,
        "kind": r.kind,
        "title": r.title,
        "body": r.body,
        "updated_at": r.updated_at,
        "readonly": ro
    })
}

pub fn expected_readonly(e: &Value) -> Vec<String> {
    let ro = e.get("readonly").cloned().unwrap_or(json!({}));
    READONLY_KEYS
        .iter()
        .filter_map(|k| {
            ro.get(*k)
                .and_then(Value::as_str)
                .map(|v| format!("{k}: {v}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    #[test]
    fn front_matter_round_trips_title_and_body() {
        let doc = front_matter(&sample());
        for key in [
            "number:",
            "kind:",
            "title:",
            "state:",
            "type:",
            "labels:",
            "assignees:",
            "milestone:",
            "author:",
            "created_at:",
            "updated_at:",
            "url:",
        ] {
            assert!(doc.contains(key), "missing {key}");
        }
        let (title, body) = parse_doc(&doc).unwrap();
        assert_eq!(title, "Fix login flow");
        assert_eq!(body, "Body text\n");
    }

    #[test]
    fn change_detection_and_readonly_guard() {
        let mut doc = front_matter(&sample());
        let e = entry(&sample());
        assert_eq!(readonly_lines(&doc), expected_readonly(&e));
        doc = doc.replacen(
            "title: \"Fix login flow\"",
            "title: \"Fixed login flow\"",
            1,
        );
        assert_eq!(parse_doc(&doc).unwrap().0, "Fixed login flow");
        let mutated = doc.replacen("state: \"open\"", "state: \"closed\"", 1);
        assert_ne!(readonly_lines(&mutated), expected_readonly(&e));
    }
}
