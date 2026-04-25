# Repository Guidelines

## Project Structure & Module Organization

`devkit` is a Rust 2024 CLI crate. Source lives in `src/`, with one module per responsibility: `cli.rs` defines Clap arguments, `config.rs` loads TOML policy, `doctor.rs` inspects local tools, `latest.rs` checks upstream versions, `upgrade.rs` builds upgrade plans, `cleanup.rs` finds leftovers, `output.rs` renders text/JSON, and `shell.rs` handles command lookup. `src/main.rs` wires the command flow together.

Use `examples/devkit.toml` as the reference team policy file. Session notes and design context belong under `docs/`. Build output stays in `target/` and should not be committed.

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

When changing TUI navigation, version pickers, action popups, or background tasks, add regression tests that prove subviews do not exit on `Esc` or arrow-key input and that long-running actions return results to the TUI instead of dropping back to the CLI. Manually smoke-test at least `cargo run -- new -i -p`, opening a subview and using arrow keys before exiting with `Q` or the finish action.

## Commit & Pull Request Guidelines

The current history uses short, descriptive commit messages such as `Initial devkit MVP`; continue with concise imperative or summary-style messages that explain the change. Keep unrelated cleanup out of feature commits.

Pull requests should include a clear description, the commands run for validation, and notes for behavior that affects local machines, PATH ordering, shell config, network calls, or package managers. Include sample text or JSON output when changing user-facing CLI output.

## Security & Configuration Tips

The MVP should report and plan changes before applying them. Preserve dry-run behavior unless a task explicitly introduces execution. Do not commit machine-specific secrets, private paths beyond examples, or generated artifacts from `target/`.
