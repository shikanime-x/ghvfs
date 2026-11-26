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

`sync` is read-only on GitHub. It materializes issues, pull requests and discussions, and writes `.ghvfs/state.sqlite3` through SQLx. Writes are committed in batches of 50 rows. An existing `state.json` is imported once and removed. `apply` is a dry run unless `--commit` is present. `--force` overrides the stale-remote guard on `updated_at`. The CLI uses clap and the runtime is Tokio.

## Layout

```text
<root>/
  .ghvfs/state.sqlite3
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

The integration tests use `GHVFS_GH` to point the binary at a stub `gh`, so they need no network or credentials. On Windows the stub-based tests are skipped; verify that target with a cross C toolchain:

```sh
nix-shell -p pkgsCross.musl64.buildPackages.gcc pkgsCross.mingwW64.buildPackages.gcc --run '
  export CC_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-gcc
  export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
  cargo check --tests --target x86_64-unknown-linux-musl
  cargo check --tests --target x86_64-pc-windows-gnu
'
```

SQLite is bundled through `libsqlite3-sys`, so cross-target checks need a C compiler for the target.
