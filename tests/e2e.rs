#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const STUB: &str = r###"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
if "-X" in args and "PATCH" in args:
    payload = open(args[args.index("--input") + 1]).read()
    with open(os.environ["GHVFS_PATCH_LOG"], "a") as log:
        log.write(payload + "\n")
    print("{}")
    sys.exit(0)
jq = args[args.index("--jq") + 1] if "--jq" in args else ""
if jq.startswith(".data.repository"):
    sys.exit(0)
if jq.startswith(".[] | select"):
    print(json.dumps({"number": 1, "title": "Stub issue", "state": "open",
        "type": {"name": "Feature"}, "labels": [{"name": "enhancement"}],
        "assignees": [], "milestone": None, "user": {"login": "shikanime"},
        "created_at": "2026-11-26T00:00:00Z", "updated_at": "2026-11-26T00:00:00Z",
        "html_url": "https://example.test/1", "body": "Issue body"}))
    sys.exit(0)
if jq == ".[]":
    print(json.dumps({"number": 9, "title": "Stub PR", "state": "open",
        "draft": True, "labels": [], "milestone": None,
        "user": {"login": "shikanime"}, "created_at": "2026-11-26T00:00:00Z",
        "updated_at": "2026-11-26T00:00:00Z", "html_url": "https://example.test/9",
        "body": "PR body"}))
    sys.exit(0)
if jq == ".resources.core":
    print(json.dumps({"remaining": 5000, "limit": 5000, "reset": 0}))
    sys.exit(0)
if jq == ".updated_at":
    print("2026-11-26T00:00:00Z")
"###;

struct Fixture {
    root: PathBuf,
    tree: PathBuf,
    stub: PathBuf,
    patches: PathBuf,
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("ghvfs-e2e-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let stub = root.join("fake-gh");
    fs::write(&stub, STUB).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let patches = root.join("patches.log");
    Fixture {
        tree: root.join("tree"),
        root,
        stub,
        patches,
    }
}

fn run(f: &Fixture, args: &[&str]) -> (bool, String) {
    run_with(f, &f.stub, args)
}

fn run_with(f: &Fixture, stub: &PathBuf, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ghvfs"))
        .args(args)
        .env("GHVFS_GH", stub)
        .env("GHVFS_PATCH_LOG", &f.patches)
        .current_dir(&f.root)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn sync(f: &Fixture) {
    let root = f.tree.to_str().unwrap();
    let (ok, out) = run(f, &["sync", "--repo", "acme/demo", "--root", root]);
    assert!(ok, "sync failed: {out}");
    assert!(
        out.contains("synced 1 issues, 1 prs, 0 discussions"),
        "{out}"
    );
}

#[test]
fn sync_tree_and_apply_round_trip() {
    let f = fixture("round-trip");
    sync(&f);
    let issue = f.tree.join("acme/demo.issues/1/issue.md");
    let pr = f.tree.join("acme/demo.pr/9/pr.md");
    assert!(issue.exists() && pr.exists());
    assert!(f.tree.join("acme/demo.discussion").is_dir());
    let doc = fs::read_to_string(&issue).unwrap();
    assert!(doc.contains("kind: issues") && doc.contains("Issue body"));

    let root = f.tree.to_str().unwrap();
    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");

    let edited = doc.replacen("title: Stub issue", "title: Edited stub issue", 1);
    fs::write(&issue, edited).unwrap();
    let (ok, out) = run(&f, &["apply", "--root", root, "--commit"]);
    assert!(ok && out.contains("apply: 1 changed"), "{out}");
    let patches = fs::read_to_string(&f.patches).unwrap();
    assert!(patches.contains("Edited stub issue"), "{patches}");

    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");

    let mutated = fs::read_to_string(&issue)
        .unwrap()
        .replacen("state: open", "state: closed", 1);
    fs::write(&issue, mutated).unwrap();
    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(!ok && out.contains("read-only metadata"), "{out}");
}

#[test]
fn watch_applies_a_saved_document() {
    let f = fixture("watch");
    sync(&f);
    let issue = f.tree.join("acme/demo.issues/1/issue.md");
    let root = f.tree.to_str().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ghvfs"))
        .args(["watch", "--root", root, "--commit"])
        .env("GHVFS_GH", &f.stub)
        .env("GHVFS_PATCH_LOG", &f.patches)
        .current_dir(&f.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(1500));
    let doc = fs::read_to_string(&issue).unwrap();
    fs::write(
        &issue,
        doc.replacen("title: Stub issue", "title: Watched edit", 1),
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(12);
    let mut patches = String::new();
    while Instant::now() < deadline {
        patches = fs::read_to_string(&f.patches).unwrap_or_default();
        if patches.contains("Watched edit") {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    child.kill().unwrap();
    let _ = child.wait();
    assert!(
        patches.contains("Watched edit"),
        "no watch patch: {patches}"
    );
}

#[test]
fn pull_updates_changed_remote_documents() {
    let f = fixture("pull");
    sync(&f);
    let remote = f.root.join("fake-gh-remote");
    let changed = STUB
        .replace("Stub issue", "Remote edit")
        .replace("2026-11-26T00:00:00Z", "2026-11-26T00:01:00Z");
    fs::write(&remote, changed).unwrap();
    fs::set_permissions(&remote, fs::Permissions::from_mode(0o755)).unwrap();

    let root = f.tree.to_str().unwrap();
    let (ok, out) = run_with(
        &f,
        &remote,
        &["pull", "--repo", "acme/demo", "--root", root],
    );
    assert!(ok && out.contains("pull: 2 changed"), "{out}");
    let issue = fs::read_to_string(f.tree.join("acme/demo.issues/1/issue.md")).unwrap();
    assert!(issue.contains("title: Remote edit"), "{issue}");
    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");
}

#[test]
fn unsync_removes_repo_tree() {
    let f = fixture("unsync");
    sync(&f);
    let root = f.tree.to_str().unwrap();
    let (ok, out) = run(&f, &["unsync", "--repo", "acme/demo", "--root", root]);
    assert!(ok && out.contains("unsynced acme/demo"), "{out}");
    assert!(!f.tree.join("acme/demo.issues").exists());
    assert!(!f.tree.join("acme/demo.pr").exists());
    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");
}
