# ghvfs

Expose a GitHub repository's issues as a local tree of markdown files with metadata.

## Auth

Set `GH_TOKEN` (or `GITHUB_TOKEN`) to a GitHub token with repo access, or pass the global flag `--gh-user <account>` to resolve the token from the `gh` CLI keyring via `gh auth token --user` — the named account wins over the environment, so concurrent use of different identities cannot drift. The API client is octocrab; `gh` is only consulted when `--gh-user` is present.

## Shell

The flake consumes `shikanime-labs/devlib`. The `rust` profile supplies cargo, clippy and rustfmt. Run `direnv allow` or `nix develop`, then use `cargo`.

## Commands

```sh
cargo run -- sync --repo shikanime-x/ghvfs --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs --commit
cargo run -- watch --root /tmp/ghvfs --commit
```

`sync` is read-only on GitHub. It materializes issues, pull requests and discussions, and writes `.ghvfs/state.sqlite3` through SQLx. Writes are committed in batches of 50 rows. `apply` is a dry run unless `--commit` is present. `--force` overrides the stale-remote guard on `updated_at`. The CLI uses clap and the runtime is Tokio.

## Layout

```text
<root>/
  .ghvfs/state.sqlite3
  <org>/
    <repo>.issues/<number>/issue.md
    <repo>.pr/<number>/pr.md
    <repo>.discussion/<number>/discussion.md
```

All three kind folders exist after a sync even when empty. Front matter is serialized from a derived serde struct through `serde_yaml`; `Kind` and `Snapshot` are derived types, and `ToSnapshot` converts a record to its stored snapshot. `apply` compares typed front-matter values against the snapshot, so quoting changes alone are not edits. SQL rows are read through a derived `sqlx::FromRow` struct. `apply` writes title and body for issues and PRs; discussions are read-only because the edit path is not implemented.

`pull --repo <org/name> --root <dir>` fetches issues and PRs updated since the newest snapshot timestamp, rewrites changed documents and upserts their snapshots. Documents with unsent local edits are skipped. `unsync --repo <org/name> --root <dir>` removes a repo's documents and rows.

`sync` and `pull` attach read-only Development and board metadata. `linked:` lists Development-section numbers (linked PRs on issues, closing issues on PRs). `--project <owner>/<number>` (repeatable) reads Projects V2 boards into a `projects:` map of field values; pass it again on `pull`. Both sections are snapshot-guarded like other read-only metadata.

`watch --pull --repo <org/name> [--repo <org/name> ...] --root <dir> [--interval 30]` polls those repos. A repo with changes is polled more often, an idle repo less often, bounded by 5 s and eight times the base interval. Every 60 s the loop reads `/rate_limit`; low remaining budget multiplies the interval by 2, 4 or 16, and zero backs off by 16.

`watch` observes the tree with `notify` and runs `apply` after document saves, using a 500 ms debounce. It is a dry run unless `--commit` is present. Backends are selected per target: kqueue on macOS, inotify on Linux and Android, ReadDirectoryChangesW on Windows. On macOS outside the devenv shell, set `LIBRARY_PATH="$(xcrun --show-sdk-path)/usr/lib"` for the kqueue backend to link.

## Check

```sh
cargo test
```

The integration tests spin up a stub HTTP API on localhost (`GHVFS_API`) backed by a Python stdlib script, so they need no network or credentials. On Windows the stub-based tests are skipped; verify that target with a cross C toolchain:

```sh
nix-shell -p pkgsCross.musl64.buildPackages.gcc pkgsCross.mingwW64.buildPackages.gcc --run '
  export CC_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-gcc
  export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
  cargo check --tests --target x86_64-unknown-linux-musl
  cargo check --tests --target x86_64-pc-windows-gnu
'
```

SQLite is bundled through `libsqlite3-sys`, so cross-target checks need a C compiler for the target.
