use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const READONLY_KEYS: [&str; 11] = [
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

struct Record {
    number: u64,
    kind: &'static str,
    title: String,
    state: String,
    type_name: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    milestone: String,
    author: String,
    created_at: String,
    updated_at: String,
    url: String,
    body: String,
}

fn parse_issue(v: &Value) -> Record {
    Record {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
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

fn parse_pr(v: &Value) -> Record {
    Record {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
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

fn parse_discussion(v: &Value) -> Record {
    Record {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
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

fn front_matter(r: &Record) -> String {
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

fn entry(r: &Record) -> Value {
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

fn write_state(root: &Path, repo: &str, records: &[Value]) -> Result<(), String> {
    fs::create_dir_all(root.join(".ghvfs")).map_err(|e| e.to_string())?;
    let payload = json!({ "repo": repo, "records": records });
    fs::write(
        state_path(root),
        serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn load_state(root: &Path) -> Result<(String, Vec<Value>), String> {
    let raw = fs::read_to_string(state_path(root)).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let records = v
        .get("records")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((field(&v, "repo"), records))
}

fn repo_parts(repo: &str) -> Result<(&str, &str), String> {
    repo.split_once('/')
        .ok_or_else(|| "repo must be <org>/<name>".into())
}

fn record_dir(root: &Path, repo: &str, kind: &str, number: u64) -> Result<PathBuf, String> {
    let (org, name) = repo_parts(repo)?;
    Ok(root
        .join(org)
        .join(format!("{name}.{kind}"))
        .join(number.to_string()))
}

fn doc_name(kind: &str) -> &'static str {
    match kind {
        "pr" => "pr.md",
        "discussion" => "discussion.md",
        _ => "issue.md",
    }
}

fn json_lines(raw: &str) -> Result<Vec<Value>, String> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn sync(repo: &str, root: &Path) -> Result<(), String> {
    let (org, name) = repo_parts(repo)?;
    for kind in ["issues", "pr", "discussion"] {
        fs::create_dir_all(root.join(org).join(format!("{name}.{kind}")))
            .map_err(|e| e.to_string())?;
    }
    let mut records = Vec::new();

    let raw = gh(&[
        "api",
        &format!("repos/{repo}/issues?per_page=100"),
        "--paginate",
        "--jq",
        ".[] | select(.pull_request == null)",
    ])?;
    records.extend(json_lines(&raw)?.iter().map(parse_issue));

    let raw = gh(&[
        "api",
        &format!("repos/{repo}/pulls?per_page=100"),
        "--paginate",
        "--jq",
        ".[]",
    ])?;
    records.extend(json_lines(&raw)?.iter().map(parse_pr));

    let query = "query($owner:String!,$name:String!){repository(owner:$owner,name:$name){discussions(first:100){nodes{number title body createdAt updatedAt url author{login}}}}}";
    let raw = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={org}"),
        "-f",
        &format!("name={name}"),
        "--jq",
        ".data.repository.discussions.nodes[]?",
    ])?;
    records.extend(json_lines(&raw)?.iter().map(parse_discussion));

    for record in &records {
        let dir = record_dir(root, repo, record.kind, record.number)?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join(doc_name(record.kind)), front_matter(record))
            .map_err(|e| e.to_string())?;
    }
    let entries: Vec<Value> = records.iter().map(entry).collect();
    write_state(root, repo, &entries)?;
    println!(
        "synced {} issues, {} prs, {} discussions",
        records.iter().filter(|r| r.kind == "issues").count(),
        records.iter().filter(|r| r.kind == "pr").count(),
        records.iter().filter(|r| r.kind == "discussion").count()
    );
    Ok(())
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
        let kind = field(e, "kind");
        // GraphQL discussions have no supported edit endpoint here.
        if kind == "discussion" {
            continue;
        }
        let old_title = field(e, "title");
        let old_body = field(e, "body");
        let doc = fs::read_to_string(record_dir(root, &repo, &kind, number)?.join(doc_name(&kind)))
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

fn is_doc(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("issue.md" | "pr.md" | "discussion.md")
    )
}

fn watch(root: &Path, commit: bool, force: bool) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    println!("watching {}", root.display());
    // ponytail: fixed 500 ms debounce absorbs editor save bursts; raise it if a writer emits slower duplicate events.
    let mut last = Instant::now() - Duration::from_millis(500);
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(event))
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
                    && event.paths.iter().any(|p| is_doc(p)) =>
            {
                if last.elapsed() >= Duration::from_millis(500) {
                    last = Instant::now();
                    if let Err(e) = apply(root, commit, force) {
                        eprintln!("ghvfs: apply failed: {e}");
                    }
                }
            }
            Ok(Err(e)) => eprintln!("ghvfs: watch event error: {e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
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
            Path::new(&arg(&args, "--root").ok_or("sync needs --root <dir>")?),
        ),
        Some("apply") => apply(
            Path::new(&arg(&args, "--root").ok_or("apply needs --root <dir>")?),
            args.iter().any(|a| a == "--commit"),
            args.iter().any(|a| a == "--force"),
        ),
        Some("watch") => watch(
            Path::new(&arg(&args, "--root").ok_or("watch needs --root <dir>")?),
            args.iter().any(|a| a == "--commit"),
            args.iter().any(|a| a == "--force"),
        ),
        _ => Err("usage: ghvfs sync --repo <org/name> --root <dir> | ghvfs apply --root <dir> [--commit] [--force] | ghvfs watch --root <dir> [--commit] [--force]".into()),
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

    fn sample() -> Record {
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

    #[test]
    fn hierarchy_paths_are_namespace_scoped() {
        let p = record_dir(Path::new("/r"), "shikanime-x/ghvfs", "pr", 7).unwrap();
        assert_eq!(p, Path::new("/r/shikanime-x/ghvfs.pr/7"));
        assert_eq!(doc_name("discussion"), "discussion.md");
    }

    #[test]
    fn watch_filters_doc_edits() {
        assert!(is_doc(Path::new("/r/org/repo.issues/1/issue.md")));
        assert!(is_doc(Path::new("/r/org/repo.pr/6/pr.md")));
        assert!(!is_doc(Path::new("/r/.ghvfs/state.json")));
    }

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
