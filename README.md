# ghvfs

Expose a GitHub repository's issues as a local tree of markdown files with metadata.

## Shell

The flake consumes `shikanime-labs/devlib`. The `rust` profile supplies cargo, clippy and rustfmt. Run `direnv allow` or `nix develop`, then use `cargo`.

## Commands

```sh
cargo run -- sync --repo shikanime-x/ghvfs --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs
cargo run -- apply --root /tmp/ghvfs --commit
```

`sync` is read-only. It writes one folder per non-PR issue under `issues/`, an `INDEX.md`, and `.ghvfs/state.json`. `apply` is a dry run unless `--commit` is present. `--force` overrides the stale-remote guard on `updated_at`.

## Layout

```
<root>/
  INDEX.md
  issues/<number>-<slug>/issue.md
  .ghvfs/state.json
```

`issue.md` carries YAML front matter with `number`, `title`, `state`, `type`, `labels`, `assignees`, `milestone`, `author`, `created_at`, `updated_at` and `url`, followed by the issue body. Only `title` and `body` are writable; editing other fields is rejected.

## Check

```sh
cargo test
```
