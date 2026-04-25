# Repository Guidelines

## Project Structure & Module Organization

`devkit` is a Rust CLI crate. Source lives in `src/`, with one module per responsibility: `cli.rs` defines Clap arguments, `config.rs` loads TOML policy, `doctor.rs` inspects local tools, `latest.rs` checks upstream versions, `upgrade.rs` builds upgrade plans, `cleanup.rs` finds leftovers, `output.rs` renders text/JSON, and `shell.rs` handles command lookup. `src/main.rs` wires the command flow together.

Use `examples/devkit.toml` as the reference team policy file. Session notes and design context belong under `docs/`. Build output stays in `target/` and should not be committed.

## Product Definition & Domain Semantics

Treat `devkit` as a cross-platform development toolchain policy controller. It manages the intended development environment for a machine or team; package-manager commands are implementation details used to reconcile the current platform with the policy.

Keep these domain boundaries explicit in code, docs, tests, and TUI copy:

- `devkit.toml` is the source of policy intent. Do not manage, install, or align a tool merely because it is present on the machine.
- `doctor` is the diagnostic engine. It should inspect the local machine, report drift, and explain evidence such as active paths, managers, versions, and PATH candidates.
- `sync` is the policy execution planner. It should turn policy drift into a dependency-aware ready/blocked plan and only execute applicable steps when the user requests apply behavior.
- `init` and the TUI are policy editors. They should help generate and refine one policy file while showing the effective TOML before writing or applying it.

Users enable capabilities, not raw internal TOML fields. A tool policy should describe what capability should exist and how that capability should be managed on the current platform.

Node has a nested workflow model:

- `[tools.node]` enables the Node.js runtime workflow.
- `tools.node.manager` selects the runtime manager, such as `fnm`, `nvm`, or a platform package manager.
- `tools.node.package_managers` is the source of truth for enabled Node package-manager workflows.
- `[tools.npm]`, `[tools.pnpm]`, `[tools.yarn]`, and `[tools.bun]` describe how each enabled Node package manager is checked, installed, or aligned.

When changing the TUI or init flow, keep the `node`, `npm`, `pnpm`, `yarn`, and `bun` semantics synchronized. Turning a Node package manager on should add it to `node.package_managers` and create its tool section. Turning it off should remove it from `node.package_managers` and remove its tool section. Editing `node.package_managers` manually should update the corresponding package-manager switches. Turning `node` off should disable those package-manager workflows in the draft because they no longer have an owning runtime workflow.

## Build, Test, and Development Commands

- `cargo run -- doctor --config examples/devkit.toml`: run the main health check with the example policy.
- `cargo run -- doctor --json --config examples/devkit.toml`: verify automation-friendly JSON output.
- `cargo run -- upgrade --dry-run --config examples/devkit.toml`: preview upgrade recommendations.
- `cargo run -- cleanup --dry-run`: preview cleanup findings for legacy toolchain paths.
- `cargo fmt --check`: confirm standard Rust formatting.
- `cargo test`: run unit tests.
- `cargo clippy --all-targets -- -D warnings`: catch lint issues before review.

## Coding Style & Naming Conventions

Follow `rustfmt` defaults: 4-space indentation, conventional import grouping, and trailing commas where rustfmt adds them. Keep modules focused and named by domain behavior. Use `snake_case` for functions, variables, files, and test names; `PascalCase` for types and enum variants. Prefer `anyhow::Result` and contextual errors at I/O boundaries, as in config loading.

## Testing Guidelines

Keep fast unit tests near the code they cover using `#[cfg(test)] mod tests`, as shown in `src/doctor.rs`. Name tests after expected behavior, for example `supports_policy_version_matching`. Add integration tests under `tests/` when a change needs to exercise CLI behavior, output formats, or multiple modules together. Run `cargo test` before opening a PR.

## TUI Development Guidelines

Interactive TUI flows must stay inside the alternate screen until the user explicitly exits with `Q`, `Ctrl-C`, or a clearly labeled finish action. Do not use `Esc` as a destructive close, cancel, or quit shortcut in the main screen, preview screen, popups, or pickers, because some terminals can deliver arrow-key prefixes as `Esc`. Treat `Esc` as ignored or as a non-destructive hint unless a future input layer can reliably disambiguate it.

Any command launched while the TUI is active must not read from the terminal. Prefer direct `Command` execution over an interactive shell. When a shell fallback is necessary, run it non-interactively, set `stdin(Stdio::null())`, and suppress or capture output so background work cannot suspend the process with tty input.

When changing TUI navigation, version pickers, action popups, or background tasks, add regression tests that prove subviews do not exit on `Esc` or arrow-key input and that long-running actions return results to the TUI instead of dropping back to the CLI. Manually smoke-test at least `cargo run -- tui -p`, opening a subview and using arrow keys before exiting with `Q` or the finish action.

## Commit & Pull Request Guidelines

The current history uses short, descriptive commit messages such as `Initial devkit MVP`; continue with concise imperative or summary-style messages that explain the change. Keep unrelated cleanup out of feature commits.

Pull requests should include a clear description, the commands run for validation, and notes for behavior that affects local machines, PATH ordering, shell config, network calls, or package managers. Include sample text or JSON output when changing user-facing CLI output.

## Security & Configuration Tips

The MVP should report and plan changes before applying them. Preserve dry-run behavior unless a task explicitly introduces execution. Do not commit machine-specific secrets, private paths beyond examples, or generated artifacts from `target/`.
