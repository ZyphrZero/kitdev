# devkit

`devkit` is a personal and team development toolchain health CLI. It does not try to replace version managers like `mise`, `fnm`, `rustup`, or `uv`; it checks, reports, and plans fixes across them.

## MVP commands

```bash
cargo run --
cargo run -- check -j
cargo run -- check -c examples/devkit.toml
cargo run -- new -p
cargo run -- new -i -p
cargo run -- init --output /tmp/devkit.toml
cargo run -- add bun
cargo run -- add bun -n
cargo run -- sync -c examples/devkit.toml
cargo run -- sync --apply -c examples/devkit.toml
cargo run -- up -n -c examples/devkit.toml
cargo run -- up -n --offline -c examples/devkit.toml
cargo run -- cleanup --dry-run
```

## Scope

The MVP checks common macOS developer tools:

- Homebrew / fnm / Node / npm / pnpm / Bun / Wrangler
- Yarn / Deno / Python / Poetry / Ruby
- uv / Rust / Cargo / Go
- legacy leftovers such as `/usr/local/go`, `/usr/local/lib/node_modules`, and `~/.nvm`

## Bootstrap planning

Daily command aliases are intentionally short:

- `devkit` or `devkit check`: inspect the local environment
- `devkit new -i -p`: interactively print a starter config
- `devkit add bun`: install one tool
- `devkit sync`: preview the policy repair plan
- `devkit sync --apply`: apply the repair plan
- `devkit up -n`: preview upgrades

The longer names remain available for scripts: `doctor`, `init`, `install`, and `upgrade`.

`add <tool>` / `install <tool>` is the shortest path for a single tool. It runs by default and uses `devkit.toml` when present, while `-n` / `--dry-run` prints the exact command first:

- `devkit add bun` installs Bun with the configured or default manager
- `devkit add deno` installs Deno with Homebrew by default
- `devkit add node -v 24` installs and selects Node through `fnm`
- `devkit add python -m uv -v 3.13` installs a Python runtime through `uv`
- `devkit add gh -c examples/devkit.toml` installs or checks a CLI package listed in policy
- `devkit add gh -m brew` installs an explicit Homebrew CLI package when no policy entry exists

`sync` turns the current machine state plus `devkit.toml` policy into a bootstrap or repair plan. `sync --apply` applies install, align, and managed shell-configuration steps, then verifies the result with `doctor`.

- install prerequisite managers such as Homebrew, `fnm`, `rustup`, and `uv`
- align managed runtimes such as Node, Go, and Rust to policy
- plan shell snippets for `fnm` and Go PATH setup
- include configured Homebrew and npm CLI packages
- execute managed shell snippets idempotently with `devkit` markers
- keep cleanup steps manual even during `sync --yes`
- end with a `doctor` verification step

## Config bootstrap

`init` turns the current machine into a starter `devkit.toml`. The default mode is deterministic; `--interactive` opens a visual TUI where you can trim tools, edit details, and review the live TOML before writing:

- `devkit new -p` prints the generated policy
- `devkit new -i -p` opens the TUI editor, then prints the accepted policy
- `devkit init --output ./devkit.toml` writes a starter file
- `devkit init --force` overwrites an existing file
- generated policy includes a stable channel, current platform, detected core runtimes, and a small set of installed CLI packages

In the TUI, use the left pane to choose a section, the center pane to edit fields, and the right pane to review the generated TOML. Main keys are `Up`/`Down` or `j`/`k` to move, `Left`/`Right` or `Tab` to switch panes, `Space` to enable or disable a tool, `Enter` or `e` to edit a field, `P` to open the full preview, `PageUp`/`PageDown` to scroll the preview, `S` to save, and `Q`/`Esc` to cancel. Editing Node, Go, or Rust version fields opens a version picker backed by remote release lists where available; choose a major selector such as `24.x`, an exact version, or press `C` to type a custom value such as `24`; press `Q` to close the picker. The TUI renders on stderr, so `devkit new -i -p > devkit.toml` still writes only TOML to the file after you save.

## Latest-version providers

`upgrade --dry-run` can query known upstream providers:

- npm registry: `npm`, `pnpm`, `yarn`, `wrangler`
- Homebrew formula metadata: `fnm`, `bun`, `deno`, `python`, `poetry`, `ruby`, `brew`
- fnm remote list: `node`
- Go official endpoint: `go`
- GitHub releases page: `uv`
- rustup: `rustup`, `rustc`, `cargo`

The MVP prints commands instead of applying changes. This keeps it safe for personal and team machines while the policy model evolves.

## Configuration

See `examples/devkit.toml` for a team policy example.
