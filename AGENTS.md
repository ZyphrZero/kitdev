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

## Commit & Pull Request Guidelines

The current history uses short, descriptive commit messages such as `Initial devkit MVP`; continue with concise imperative or summary-style messages that explain the change. Keep unrelated cleanup out of feature commits.

Pull requests should include a clear description, the commands run for validation, and notes for behavior that affects local machines, PATH ordering, shell config, network calls, or package managers. Include sample text or JSON output when changing user-facing CLI output.

## Security & Configuration Tips

The MVP should report and plan changes before applying them. Preserve dry-run behavior unless a task explicitly introduces execution. Do not commit machine-specific secrets, private paths beyond examples, or generated artifacts from `target/`.
