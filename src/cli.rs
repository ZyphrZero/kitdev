use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "devkit")]
#[command(
    version,
    about = "Keep personal and team development toolchains healthy"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect installed tools, versions, PATH, and common conflicts.
    #[command(visible_alias = "check")]
    Doctor {
        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read expected tool versions from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Show upgrade candidates without changing anything.
    #[command(visible_alias = "up")]
    Upgrade {
        /// Preview actions only. The MVP currently only supports dry-run.
        #[arg(short = 'n', long, default_value_t = true)]
        dry_run: bool,

        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read expected tool versions from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,

        /// Skip package-manager and official latest-version queries.
        #[arg(long, visible_alias = "offline")]
        skip_latest: bool,
    },

    /// Generate a starter devkit policy from the current machine state.
    #[command(visible_alias = "new")]
    Init {
        /// Open a visual TUI editor before generating the config.
        #[arg(short, long)]
        interactive: bool,

        /// Print the generated TOML to stdout instead of writing a file.
        #[arg(short = 'p', long, visible_alias = "print")]
        stdout: bool,

        /// Output path for the generated TOML file.
        #[arg(short, long, default_value = "devkit.toml")]
        output: PathBuf,

        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },

    /// Install one supported tool directly.
    #[command(visible_alias = "add")]
    Install {
        /// Tool name, for example bun, node, uv, rust, or go.
        tool: String,

        /// Preview the install command without running it.
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Override the configured or default manager.
        #[arg(short, long)]
        manager: Option<String>,

        /// Override the configured or default version selector.
        #[arg(short, long)]
        version: Option<String>,

        /// Read install preferences from a TOML config file when present.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Plan how to bootstrap or repair a machine to match the configured toolchain.
    Sync {
        /// Preview actions only.
        #[arg(short = 'n', long, action = ArgAction::SetTrue, conflicts_with = "yes")]
        dry_run: bool,

        /// Apply install, align, and managed shell-configuration steps.
        #[arg(long, visible_alias = "apply", conflicts_with = "dry_run")]
        yes: bool,

        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read desired toolchain state from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Find old or conflicting toolchain leftovers.
    Cleanup {
        /// Preview actions only. The MVP currently only supports dry-run.
        #[arg(short = 'n', long, default_value_t = true)]
        dry_run: bool,

        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands};

    #[test]
    fn command_can_be_omitted_for_default_doctor() {
        let cli = Cli::try_parse_from(["devkit"]).unwrap();

        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_short_command_aliases() {
        let cli = Cli::try_parse_from(["devkit", "check", "-j", "-c", "team.toml"]).unwrap();
        match cli.command.unwrap() {
            Commands::Doctor { json, config } => {
                assert!(json);
                assert_eq!(config.to_string_lossy(), "team.toml");
            }
            command => panic!("unexpected command: {command:?}"),
        }

        let cli = Cli::try_parse_from(["devkit", "new", "-i", "-p"]).unwrap();
        match cli.command.unwrap() {
            Commands::Init {
                interactive,
                stdout,
                ..
            } => {
                assert!(interactive);
                assert!(stdout);
            }
            command => panic!("unexpected command: {command:?}"),
        }

        let cli =
            Cli::try_parse_from(["devkit", "add", "node", "-n", "-m", "fnm", "-v", "24"]).unwrap();
        match cli.command.unwrap() {
            Commands::Install {
                tool,
                dry_run,
                manager,
                version,
                ..
            } => {
                assert_eq!(tool, "node");
                assert!(dry_run);
                assert_eq!(manager.as_deref(), Some("fnm"));
                assert_eq!(version.as_deref(), Some("24"));
            }
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_apply_alias_for_sync_yes() {
        let cli = Cli::try_parse_from(["devkit", "sync", "--apply"]).unwrap();

        match cli.command.unwrap() {
            Commands::Sync { yes, .. } => assert!(yes),
            command => panic!("unexpected command: {command:?}"),
        }
    }
}
