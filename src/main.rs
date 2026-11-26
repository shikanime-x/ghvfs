use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const READONLY_KEYS: [&str; 8] = [
    "state",
    "type",
    "labels",
    "assignees",
    "milestone",
    "author",
    "created_at",
    "url",
];

fn gh(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("spawn gh: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8(out.stdout).map_err(|e| e.to_string())?)
}

fn field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
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

struct Issue {
    number: u64,
    title: String,
    state: String,
    kind: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    milestone: String,
    author: String,
    created_at: String,
    updated_at: String,
    url: String,
    body: String,
}

fn parse_issue(v: &Value) -> Issue {
    Issue {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: field(v, "title"),
        state: field(v, "state"),
        kind: v
            .get("type")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        labels: names(v, "labels", "name"),
        assignees: names(v, "assignees", "login"),
        milestone: v
            .get("milestone")
            .and_then(|m| m.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        author: v
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        created_at: field(v, "created_at"),
        updated_at: field(v, "updated_at"),
        url: field(v, "html_url"),
        body: field(v, "body"),
    }
}

fn slug(title: &str) -> String {
    let folded: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut out = String::new();
    for c in folded.chars() {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    // ponytail: ASCII-only fold keeps truncate on a char boundary; Unicode titles lose non-ASCII chars.
    let out = out.chars().take(60).collect::<String>();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "issue".to_string()
    } else {
        out
    }
}

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

fn front_matter(i: &Issue) -> String {
    format!(
        "---\nnumber: {}\ntitle: {}\nstate: {}\ntype: {}\nlabels: {}\nassignees: {}\nmilestone: {}\nauthor: {}\ncreated_at: {}\nupdated_at: {}\nurl: {}\n---\n{}\n",
        i.number,
        yq(&i.title),
        yq(&i.state),
        yq(&i.kind),
        yq_list(&i.labels),
        yq_list(&i.assignees),
        yq(&i.milestone),
        yq(&i.author),
        yq(&i.created_at),
        yq(&i.updated_at),
        yq(&i.url),
        i.body
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

fn parse_doc(doc: &str) -> Option<(String, String)> {
    let (head, body) = doc.split_once("\n---\n")?;
    let title = head
        .lines()
        .find_map(|l| l.strip_prefix("title: ").map(unquote))?;
    Some((title, body.to_string()))
}

fn readonly_lines(doc: &str) -> Vec<String> {
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

fn state_path(root: &Path) -> PathBuf {
    root.join(".ghvfs").join("state.json")
}

fn entry(i: &Issue) -> Value {
    json!({
        "number": i.number,
        "title": i.title,
        "body": i.body,
        "updated_at": i.updated_at,
        "readonly": {
            "state": yq(&i.state),
            "type": yq(&i.kind),
            "labels": yq_list(&i.labels),
            "assignees": yq_list(&i.assignees),
            "milestone": yq(&i.milestone),
            "author": yq(&i.author),
            "created_at": yq(&i.created_at),
            "url": yq(&i.url)
        }
    })
}

fn write_state(root: &Path, repo: &str, issues: &[Value]) -> Result<(), String> {
    fs::create_dir_all(root.join(".ghvfs")).map_err(|e| e.to_string())?;
    let payload = json!({ "repo": repo, "issues": issues });
    fs::write(
        state_path(root),
        serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn load_state(root: &Path) -> Result<(String, Vec<Value>), String> {
    let raw = fs::read_to_string(state_path(root)).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let repo = field(&v, "repo");
    let issues = v
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((repo, issues))
}

fn sync(repo: &str, root: &Path) -> Result<(), String> {
    let raw = gh(&[
        "api",
        &format!("repos/{repo}/issues?per_page=100"),
        "--paginate",
        "--jq",
        ".[] | select(.pull_request == null)",
    ])?;
    let issues: Vec<Issue> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(parse_issue)
        .collect();

    fs::create_dir_all(root.join("issues")).map_err(|e| e.to_string())?;
    let mut index = String::from("# Issues\n\n");
    for issue in &issues {
        let dir = root
            .join("issues")
            .join(format!("{:04}-{}", issue.number, slug(&issue.title)));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("issue.md"), front_matter(issue)).map_err(|e| e.to_string())?;
        index.push_str(&format!("- #{} {}\n", issue.number, issue.title));
    }
    fs::write(root.join("INDEX.md"), index).map_err(|e| e.to_string())?;
    let entries: Vec<Value> = issues.iter().map(entry).collect();
    write_state(root, repo, &entries)?;
    println!("synced {} issues", issues.len());
    Ok(())
}

fn issue_dir(root: &Path, number: u64) -> Result<PathBuf, String> {
    let prefix = format!("{number:04}-");
    let dirs = fs::read_dir(root.join("issues")).map_err(|e| e.to_string())?;
    for d in dirs.flatten() {
        if d.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && d.file_name().to_string_lossy().starts_with(&prefix)
        {
            return Ok(d.path());
        }
    }
    Err(format!("no folder for issue #{number}"))
}

fn expected_readonly(e: &Value) -> Vec<String> {
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

fn apply(root: &Path, commit: bool, force: bool) -> Result<(), String> {
    let (repo, mut entries) = load_state(root)?;
    let mut changed = 0;
    for e in entries.iter_mut() {
        let number = e.get("number").and_then(Value::as_u64).unwrap_or(0);
        let old_title = field(e, "title");
        let old_body = field(e, "body");
        let doc = fs::read_to_string(issue_dir(root, number)?.join("issue.md"))
            .map_err(|e| e.to_string())?;
        if readonly_lines(&doc) != expected_readonly(e) {
            return Err(format!(
                "#{number} changes read-only metadata; only title and body are writable"
            ));
        }
        let (title, body) =
            parse_doc(&doc).ok_or_else(|| format!("#{number} has malformed front matter"))?;
        let body = body.trim_end().to_string();
        if title == old_title && body == old_body.trim_end() {
            continue;
        }
        changed += 1;
        let remote = gh(&[
            "api",
            &format!("repos/{repo}/issues/{number}"),
            "--jq",
            ".updated_at",
        ])?
        .trim()
        .to_string();
        if remote != field(e, "updated_at") && !force {
            return Err(format!(
                "#{number} changed remotely; pass --force to overwrite"
            ));
        }
        if !commit {
            println!("#{number}: would PATCH title and body");
            continue;
        }
        let mut patch = json!({});
        if title != old_title {
            patch["title"] = json!(title.clone());
        }
        if body != old_body.trim_end() {
            patch["body"] = json!(format!("{body}\n"));
        }
        let path = env::temp_dir().join(format!("ghvfs-{number}.json"));
        fs::write(&path, patch.to_string()).map_err(|e| e.to_string())?;
        gh(&[
            "api",
            "-X",
            "PATCH",
            &format!("repos/{repo}/issues/{number}"),
            "--input",
            path.to_str().ok_or("bad temp path")?,
        ])?;
        e["title"] = json!(title);
        e["body"] = json!(format!("{body}\n"));
        e["updated_at"] = json!(remote);
    }
    if commit {
        write_state(root, &repo, &entries)?;
    }
    println!("apply: {changed} changed");
    Ok(())
}

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("sync") => sync(
            &arg(&args, "--repo").ok_or("sync needs --repo <org/name>")?,
            &Path::new(&arg(&args, "--root").ok_or("sync needs --root <dir>")?).to_path_buf(),
        ),
        Some("apply") => apply(
            Path::new(&arg(&args, "--root").ok_or("apply needs --root <dir>")?),
            args.iter().any(|a| a == "--commit"),
            args.iter().any(|a| a == "--force"),
        ),
        _ => Err("usage: ghvfs sync --repo <org/name> --root <dir> | ghvfs apply --root <dir> [--commit] [--force]".into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ghvfs: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Issue {
        Issue {
            number: 7,
            title: "Fix login flow".into(),
            state: "open".into(),
            kind: "Feature".into(),
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

    #[test]
    fn slug_folds_and_truncates() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("---"), "issue");
        assert!(slug(&"a".repeat(100)).len() == 60);
    }

    #[test]
    fn front_matter_round_trips_title_and_body() {
        let doc = front_matter(&sample());
        for key in [
            "number:",
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
        assert_eq!(parse_doc(&doc).unwrap().0, "Fix login flow");
        doc = doc.replacen(
            "title: \"Fix login flow\"",
            "title: \"Fixed login flow\"",
            1,
        );
        let e = entry(&sample());
        assert_eq!(readonly_lines(&doc), expected_readonly(&e));
        assert_eq!(parse_doc(&doc).unwrap().0, "Fixed login flow");
        let mutated = doc.replacen("state: \"open\"", "state: \"closed\"", 1);
        assert_ne!(readonly_lines(&mutated), expected_readonly(&e));
    }
}
