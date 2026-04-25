# devkit

`devkit` is a personal and team development toolchain health CLI. It does not try to replace version managers like `mise`, `fnm`, `rustup`, or `uv`; it checks, reports, and plans fixes across them.

Chinese documentation: [README_ZH.md](README_ZH.md)

## Product model

`devkit` is a cross-platform development toolchain policy controller. It manages the intended development environment for a machine or team, not the package-manager commands themselves. Commands are an implementation detail used to reconcile the current platform with the policy.

The core model is:

- `devkit.toml` is the policy: it describes the desired toolchain state.
- `doctor` is the diagnostic engine: it inspects the local machine, reports drift, and explains evidence such as active paths, managers, versions, and PATH candidates.
- `sync` is the policy execution planner: it turns policy drift into a dependency-aware ready/blocked plan, and only applies executable steps when requested.
- `init` and the TUI are policy editors: they help generate and refine one policy file while showing the effective TOML before writing or applying it.

This means users enable capabilities, not raw internal fields. A tool section should answer two questions:

- what capability should exist, such as Node, Go, Rust, Python, or a CLI package set
- how that capability should be managed on this platform, such as `fnm`, `nvm`, `rustup`, `uv`, Homebrew, Windows Package Manager, or a standalone installer

Node follows a nested workflow model:

- `[tools.node]` enables the Node.js runtime workflow.
- `tools.node.manager` selects the Node runtime manager, such as `fnm`, `nvm`, or a platform package manager.
- `tools.node.package_managers` is the source of truth for enabled Node package-manager workflows.
- `[tools.npm]`, `[tools.pnpm]`, `[tools.yarn]`, and `[tools.bun]` describe how each enabled package manager is checked, installed, or aligned.

For example:

```toml
[tools.node]
version = "24.x"
manager = "fnm"
package_managers = ["npm", "pnpm", "yarn", "bun"]

[tools.pnpm]
version = "latest"
manager = "corepack"

[tools.yarn]
version = "stable"
manager = "corepack"

[tools.bun]
version = "latest"
manager = "brew"
```

In the TUI, the `node` switch controls the Node runtime workflow. The `npm`, `pnpm`, `yarn`, and `bun` switches represent Node package-manager workflows and must stay synchronized with `node.package_managers`. Turning one of those package managers off removes it from `node.package_managers` and removes its tool section from the generated policy. Turning one on adds it back and creates the corresponding tool section. Turning Node off disables the package-manager workflows in the draft because they no longer have an owning runtime workflow.

`devkit` should not manage or install a tool just because it is present on the machine. The policy is the source of intent. Unconfigured tools may be reported as environmental context when useful, but `doctor` and `sync` should focus on configured tools and dependencies required by configured tools.

## MVP commands

```bash
cargo run -- doctor
cargo run -- doctor -j
cargo run -- doctor -c examples/devkit.toml
cargo run -- init -p
cargo run -- init -i -p
cargo run -- tui
cargo run -- tui -p
cargo run -- init --output /tmp/devkit.toml
cargo run -- config validate -c examples/devkit.toml
cargo run -- config explain -c examples/devkit.toml
cargo run -- install bun
cargo run -- install bun -n
cargo run -- sync -c examples/devkit.toml
cargo run -- sync --apply -c examples/devkit.toml
cargo run -- upgrade -n -c examples/devkit.toml
cargo run -- upgrade -n --offline -c examples/devkit.toml
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

Daily commands use one canonical path:

- `devkit doctor`: inspect the local environment
- `devkit tui`: open the visual policy editor
- `devkit init -i -p`: interactively print a starter config
- `devkit config validate`: validate the effective single-file policy
- `devkit config explain`: show which base and platform override values were applied
- `devkit install bun`: install one tool
- `devkit sync`: preview the policy repair plan
- `devkit sync --apply`: apply the repair plan
- `devkit upgrade -n`: preview upgrades

`install <tool>` is the path for a single tool. It runs by default and uses `devkit.toml` when present, while `-n` / `--dry-run` prints the exact command first:

- `devkit install bun` installs Bun with the configured or platform default manager
- `devkit install deno` installs Deno with Homebrew on macOS, standalone installers on Linux, or `winget` on Windows
- `devkit install node -v 24` installs and selects Node through `fnm`
- `devkit install node -m nvm -v 24` installs and selects Node through `nvm`
- `devkit install python -m uv -v 3.13` installs a Python runtime through `uv`
- `devkit install gh -c examples/devkit.toml` installs or checks a CLI package listed in policy
- `devkit install gh -m brew` installs an explicit Homebrew CLI package when no policy entry exists
- `devkit install GitHub.cli -m winget` installs an explicit Windows Package Manager package ID
- `devkit install gh -c team.toml` can infer the `[tools.cli]` manager from the current machine when the policy lists the package but omits `manager`

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
- keep cleanup steps manual even during `sync --apply`
- end with a `doctor` verification step

## Config bootstrap

`init` turns the current machine into a starter `devkit.toml`. The default mode is deterministic; `--interactive` opens a visual TUI where you can trim tools, edit details, and review the live TOML before writing:

- `devkit init -p` prints the generated policy
- `devkit tui` opens the TUI editor and writes the accepted policy to `devkit.toml`
- `devkit tui -p` opens the TUI editor, then prints the accepted policy
- `devkit init -i -p` opens the TUI editor, then prints the accepted policy
- `devkit init --output ./devkit.toml` writes a starter file
- `devkit init --force` overwrites an existing file
- generated policy includes a stable channel, current platform, detected core runtimes, and a small set of installed CLI packages

In the TUI, use the left pane to choose a section, the center pane to edit fields, and the right pane to review the generated TOML. Main keys are `Up`/`Down` or `j`/`k` to move, `Left`/`Right` or `Tab` to switch panes, `Space` to enable or disable a tool, `Enter` or `e` to edit a field, `P` to open the full preview, `PageUp`/`PageDown` to scroll the preview, `S` to save, and `Q` to cancel. `Esc` is ignored as a destructive close shortcut so terminals that emit arrow-key prefixes as `Esc` do not accidentally exit. Editing Node, Go, or Rust version fields opens a version picker backed by remote release lists where available; choose a major selector such as `24.x`, an exact version, or press `C` to type a custom value such as `24`; press `Q` to close the picker. The TUI renders on stderr, so `devkit tui -p > devkit.toml` still writes only TOML to the file after you save.

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
