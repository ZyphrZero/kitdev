# devkit

`devkit` is a personal and team development toolchain health CLI. It does not try to replace version managers like `mise`, `fnm`, `rustup`, or `uv`; it checks, reports, and plans fixes across them.

## MVP commands

```bash
cargo run -- doctor
cargo run -- doctor --json
cargo run -- doctor --config examples/devkit.toml
cargo run -- upgrade --dry-run --config examples/devkit.toml
cargo run -- upgrade --dry-run --skip-latest --config examples/devkit.toml
cargo run -- cleanup --dry-run
```

## Scope

The MVP checks common macOS developer tools:

- Homebrew / fnm / Node / npm / pnpm / Bun / Wrangler
- uv / Rust / Cargo / Go
- legacy leftovers such as `/usr/local/go`, `/usr/local/lib/node_modules`, and `~/.nvm`

## Latest-version providers

`upgrade --dry-run` can query known upstream providers:

- npm registry: `npm`, `pnpm`, `wrangler`
- Homebrew formula metadata: `fnm`, `bun`, `brew`
- fnm remote list: `node`
- Go official endpoint: `go`
- GitHub releases page: `uv`
- rustup: `rustup`, `rustc`, `cargo`

The MVP prints commands instead of applying changes. This keeps it safe for personal and team machines while the policy model evolves.

## Configuration

See `examples/devkit.toml` for a team policy example.
