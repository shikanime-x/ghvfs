# ghvfs

Expose a GitHub repository's issues as a local tree of markdown files with metadata.

## Shell

The flake consumes `shikanime-labs/devlib`. The `rust` profile supplies cargo, clippy and rustfmt. Run `direnv allow` or `nix develop`, then use `cargo`.

## Commands

```sh
cargo run -- sync --repo shikanime-x/ghvfs --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs --commit
cargo run -- watch --root /tmp/ghvfs --commit
```

`sync` is read-only on GitHub. It materializes issues, pull requests and discussions, and writes `.ghvfs/state.json` for the apply path. `apply` is a dry run unless `--commit` is present. `--force` overrides the stale-remote guard on `updated_at`.

## Layout

```text
<root>/
  .ghvfs/state.json
  <org>/
    <repo>.issues/<number>/issue.md
    <repo>.pr/<number>/pr.md
    <repo>.discussion/<number>/discussion.md
```

All three kind folders exist after a sync even when empty. Each document carries YAML front matter with `number`, `kind`, `title`, `state`, `type`, `labels`, `assignees`, `milestone`, `author`, `created_at`, `updated_at` and `url`, followed by the body. `apply` writes title and body for issues and PRs; discussions are read-only because the edit path is not implemented. Editing other front matter fields is rejected.

`watch` observes the tree with `notify` and runs `apply` after document saves, using a 500 ms debounce. It is a dry run unless `--commit` is present. Backends are selected per target: kqueue on macOS, inotify on Linux and Android, ReadDirectoryChangesW on Windows. On macOS outside the devenv shell, set `LIBRARY_PATH="$(xcrun --show-sdk-path)/usr/lib"` for the kqueue backend to link.

## Check

```sh
cargo test
```

The integration tests use `GHVFS_GH` to point the binary at a stub `gh`, so they need no network or credentials. On Windows the stub-based tests are skipped; verify that target with `cargo check --target x86_64-pc-windows-gnu`.
