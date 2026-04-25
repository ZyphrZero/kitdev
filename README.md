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
cargo run -- config validate -c examples/devkit.toml
cargo run -- config explain -c examples/devkit.toml
cargo run -- add bun
cargo run -- add bun -n
cargo run -- sync -c examples/devkit.toml
cargo run -- sync --apply -c examples/devkit.toml
cargo run -- up -n -c examples/devkit.toml
cargo run -- up -n --offline -c examples/devkit.toml
cargo run -- cleanup --dry-run
```

## Scope

`devkit` checks common developer tools on macOS, Linux, and Windows:

- Homebrew / fnm / nvm / Node / npm / pnpm / Bun / Wrangler
- Yarn / Deno / Python / Poetry / Ruby
- uv / Rust / Cargo / Go
- platform package managers such as Homebrew and Windows Package Manager when policy requires them
- legacy leftovers such as `/usr/local/go`, `/usr/local/lib/node_modules`, `~/.nvm` when nvm is not configured, and Windows npm/nvm paths
- PATH candidate counts for tools that resolve to more than one executable

Platform defaults are intentionally conservative:

- macOS keeps the existing Homebrew-oriented defaults
- Linux prefers standalone installers, `rustup`, official Go, and `uv` for Python runtimes; CLI packages use the first detected package manager from `apt`, `dnf`, `pacman`, `zypper`, `apk`, or Linuxbrew when no manager is configured
- Windows prefers `winget` where there is a stable package ID, PowerShell installers for standalone tools, `rustup`, and `uv` for Python runtimes; CLI packages use the first detected package manager from `winget`, `scoop`, or `choco` when no manager is configured

## Bootstrap planning

Daily command aliases are intentionally short:

- `devkit` or `devkit check`: inspect the local environment
- `devkit new -i -p`: interactively print a starter config
- `devkit config validate`: validate the effective single-file policy
- `devkit config explain`: show which base and platform override values were applied
- `devkit add bun`: install one tool
- `devkit sync`: preview the policy repair plan
- `devkit sync --apply`: apply the repair plan
- `devkit up -n`: preview upgrades

The longer names remain available for scripts: `doctor`, `init`, `install`, and `upgrade`.

`add <tool>` / `install <tool>` is the shortest path for a single tool. It runs by default and uses `devkit.toml` when present, while `-n` / `--dry-run` prints the exact command first:

- `devkit add bun` installs Bun with the configured or platform default manager
- `devkit add deno` installs Deno with Homebrew on macOS, standalone installers on Linux, or `winget` on Windows
- `devkit add node -v 24` installs and selects Node through `fnm`
- `devkit add node -m nvm -v 24` installs and selects Node through `nvm`
- `devkit add python -m uv -v 3.13` installs a Python runtime through `uv`
- `devkit add gh -c examples/devkit.toml` installs or checks a CLI package listed in policy
- `devkit add gh -m brew` installs an explicit Homebrew CLI package when no policy entry exists
- `devkit add GitHub.cli -m winget` installs an explicit Windows Package Manager package ID
- `devkit add gh -c team.toml` can infer the `[tools.cli]` manager from the current machine when the policy lists the package but omits `manager`

`sync` turns the current machine state plus `devkit.toml` policy into a bootstrap or repair plan. `sync --apply` applies install, align, and managed shell-configuration steps, then verifies the result with `doctor`.

- install prerequisite managers such as Homebrew, Windows Package Manager, `fnm`, `nvm`, `rustup`, and `uv`
- align managed runtimes such as Node, Go, and Rust to policy
- group dry-run plans into `ready` and `blocked` graph sections so dependent steps do not run before prerequisites
- plan platform-appropriate shell snippets for `fnm` and Go PATH setup
- plan shell initialization for `nvm` when Node is configured with `manager = "nvm"`
- report duplicate PATH candidates from the actual environment instead of relying on platform-specific suspicious path lists
- show step blockers such as `tool:fnm`, `tool:npm`, or `tool:winget` before attempting dependent commands
- include configured Homebrew, npm, winget, and explicit Linux package-manager CLI packages
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

In the TUI, use the left pane to choose a section, the center pane to edit fields, and the right pane to review the generated TOML. Main keys are `Up`/`Down` or `j`/`k` to move, `Left`/`Right` or `Tab` to switch panes, `Space` to enable or disable a tool, `Enter` or `e` to edit a field, `P` to open the full preview, `PageUp`/`PageDown` to scroll the preview, `S` to save, and `Q` to cancel. `Esc` is ignored as a destructive close shortcut so terminals that emit arrow-key prefixes as `Esc` do not accidentally exit. Editing Node, Go, or Rust version fields opens a version picker backed by remote release lists where available; choose a major selector such as `24.x`, an exact version, or press `C` to type a custom value such as `24`; press `Q` to close the picker. The TUI renders on stderr, so `devkit new -i -p > devkit.toml` still writes only TOML to the file after you save.

## Latest-version providers

`upgrade --dry-run` can query known upstream providers:

- npm registry: `npm`, `pnpm`, `yarn`, `wrangler`
- Homebrew formula metadata when Homebrew is the configured manager: `fnm`, `bun`, `deno`, `python`, `poetry`, `ruby`, `brew`
- Node remote releases when `node` is managed by `fnm` or `nvm`
- Go official endpoint when `go` is managed from the official source
- GitHub releases page: `uv`
- rustup: `rustup`, `rustc`, `cargo`
- unsupported manager-specific checks, such as `winget`, `scoop`, `choco`, and distro package managers, are reported as unavailable instead of falling back to the wrong provider

The MVP prints commands instead of applying changes. This keeps it safe for personal and team machines while the policy model evolves.

## Configuration

See `examples/devkit.toml` for a single team policy that carries shared defaults plus `[platform.macos]`, `[platform.linux]`, and `[platform.windows]` overrides. `DevkitConfig::read` resolves the effective policy for the current machine automatically; OS-level overrides are applied first, then exact platform tags such as `[platform.macos-arm64]` can refine them.

`devkit config validate` checks both the current effective policy and explicit platform override sections for unsupported tools, invalid manager/source choices, unknown platform keys, empty package entries, and runtime package-manager gaps. `devkit config explain` prints the resolved value source for each field, for example `base`, `platform.macos`, or `platform.macos-arm64`.
