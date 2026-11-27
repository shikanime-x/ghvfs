#![cfg(unix)]

use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// One JSON HTTP stub: /api/* serves REST fixtures, /api/graphql the GraphQL fixture,
/// PATCHes are appended to PATCH_LOG. python3 stdlib only; STUB_UPDATED env varies the
/// remote timestamp so pull tests can produce real diffs.
const STUB: &str = r###"#!/usr/bin/env python3
import json, os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

UPDATED = os.environ.get("STUB_UPDATED", "2026-11-26T00:00:00Z")
ISSUE = {"number": 1, "title": os.environ.get("STUB_TITLE", "Stub issue"), "state": "open",
    "type": {"name": "Feature"}, "labels": [{"name": "enhancement"}],
    "assignees": [], "milestone": None, "user": {"login": "shikanime"},
    "created_at": "2026-11-26T00:00:00Z", "updated_at": UPDATED,
    "html_url": "https://example.test/1", "body": "Issue body"}
PR = {"number": 9, "title": "Stub PR", "state": "open",
    "draft": True, "labels": [], "milestone": None,
    "user": {"login": "shikanime"}, "created_at": "2026-11-26T00:00:00Z",
    "updated_at": UPDATED,
    "html_url": "https://example.test/9", "body": "PR body"}
GQL = {"repository": {
    "issues": {"nodes": [{"number": 1, "linkedBranches": {"nodes": []}}]},
    "pullRequests": {"nodes": [{"number": 9,
        "closingIssuesReferences": {"nodes": [{"number": 1}]}}]}}}
RATE = {"resources": {"core": {"remaining": 5000, "limit": 5000, "reset": 0}}}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def _send(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path.startswith("/api/rate_limit"):
            self._send(RATE)
        elif "/issues/" in self.path:
            self._send(ISSUE)
        elif "/issues" in self.path:
            self._send([ISSUE])
        elif "/pulls" in self.path:
            self._send([PR])
        else:
            self._send({}, 404)
    def do_PATCH(self):
        n = int(self.headers.get("Content-Length", 0))
        with open(os.environ["PATCH_LOG"], "a") as log:
            log.write(self.rfile.read(n).decode() + "\n")
        self._send({"updated_at": UPDATED})
    def do_POST(self):
        if self.path.endswith("/graphql"):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            self._send({"data": GQL})
        else:
            self._send({}, 404)

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
"###;

struct Fixture {
    root: PathBuf,
    tree: PathBuf,
    api: String,
    patches: PathBuf,
    child: Option<std::process::Child>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn fixture(name: &str, title: &str, updated: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("ghvfs-e2e-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let stub = root.join("stub.py");
    fs::write(&stub, STUB).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let patches = root.join("patches.log");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let child = Command::new("python3")
        .arg(&stub)
        .arg(port.to_string())
        .env("PATCH_LOG", &patches)
        .env("STUB_TITLE", title)
        .env("STUB_UPDATED", updated)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn python stub");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if Instant::now() > deadline {
            panic!("stub never became ready");
        }
        thread::sleep(Duration::from_millis(50));
    }
    Fixture {
        tree: root.join("tree"),
        root,
        api: format!("http://127.0.0.1:{port}/api"),
        patches,
        child: Some(child),
    }
}

fn run(f: &Fixture, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ghvfs"))
        .args(args)
        .env("GH_TOKEN", "stub-token")
        .env("GHVFS_API", &f.api)
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
    let f = fixture("round-trip", "Stub issue", "2026-11-26T00:00:00Z");
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
    let f = fixture("watch", "Stub issue", "2026-11-26T00:00:00Z");
    sync(&f);
    let issue = f.tree.join("acme/demo.issues/1/issue.md");
    let root = f.tree.to_str().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ghvfs"))
        .args(["watch", "--root", root, "--commit"])
        .env("GH_TOKEN", "stub-token")
        .env("GHVFS_API", &f.api)
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
    let f = fixture("pull", "Stub issue", "2026-11-26T00:00:00Z");
    sync(&f);
    // Second stub with a newer timestamp and a different title.
    let f2 = fixture("pull-remote", "Remote edit", "2026-11-26T00:01:00Z");
    let root = f.tree.to_str().unwrap();
    let (ok, out) = run(&f2, &["pull", "--repo", "acme/demo", "--root", root]);
    assert!(ok && out.contains("pull: 2 changed"), "{out}");
    let issue = fs::read_to_string(f.tree.join("acme/demo.issues/1/issue.md")).unwrap();
    assert!(issue.contains("title: Remote edit"), "{issue}");
    let (ok, out) = run(&f2, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");
}

#[test]
fn unsync_removes_repo_tree() {
    let f = fixture("unsync", "Stub issue", "2026-11-26T00:00:00Z");
    sync(&f);
    let root = f.tree.to_str().unwrap();
    let (ok, out) = run(&f, &["unsync", "--repo", "acme/demo", "--root", root]);
    assert!(ok && out.contains("unsynced acme/demo"), "{out}");
    assert!(!f.tree.join("acme/demo.issues").exists());
    assert!(!f.tree.join("acme/demo.pr").exists());
    let (ok, out) = run(&f, &["apply", "--root", root]);
    assert!(ok && out.contains("apply: 0 changed"), "{out}");
}
