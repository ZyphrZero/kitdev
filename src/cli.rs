use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

use crate::i18n::Language;

#[derive(Parser, Debug)]
#[command(name = "devkit")]
#[command(
    version,
    about = "Keep personal and team development toolchains healthy"
)]
pub struct Cli {
    /// Output language. Defaults to DEVKIT_LANG or the current locale.
    #[arg(long, global = true, value_enum)]
    pub lang: Option<Language>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect installed tools, versions, PATH, and common conflicts.
    Doctor {
        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read expected tool versions from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Show upgrade candidates without changing anything.
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
        #[arg(long)]
        offline: bool,
    },

    /// Generate a starter devkit policy from the current machine state.
    Init {
        /// Open a visual TUI editor before generating the config.
        #[arg(short, long)]
        interactive: bool,

        /// Print the generated TOML to stdout instead of writing a file.
        #[arg(short = 'p', long)]
        print: bool,

        /// Output path for the generated TOML file.
        #[arg(short, long, default_value = "devkit.toml")]
        output: PathBuf,

        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },

    /// Open the visual TUI policy editor.
    Tui {
        /// Print the generated TOML to stdout instead of writing a file.
        #[arg(short = 'p', long)]
        print: bool,

        /// Output path for the generated TOML file.
        #[arg(short, long, default_value = "devkit.toml")]
        output: PathBuf,

        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },

    /// Install one supported tool directly.
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
        #[arg(short = 'n', long, action = ArgAction::SetTrue, conflicts_with = "apply")]
        dry_run: bool,

        /// Apply install, align, and managed shell-configuration steps.
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,

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

    /// Inspect and explain the devkit configuration file.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Validate the effective single-file configuration.
    Validate {
        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Show which base and platform override values produced this machine's config.
    Explain {
        /// Print machine-readable JSON.
        #[arg(short, long)]
        json: bool,

        /// Read a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands, ConfigCommands};

    #[test]
    fn command_is_required_for_a_single_clear_path() {
        let error = Cli::try_parse_from(["devkit"]).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn parses_canonical_commands() {
        let cli = Cli::try_parse_from(["devkit", "doctor", "-j", "-c", "team.toml"]).unwrap();
        match cli.command {
            Commands::Doctor { json, config } => {
                assert!(json);
                assert_eq!(config.to_string_lossy(), "team.toml");
            }
            command => panic!("unexpected command: {command:?}"),
        }

        let cli = Cli::try_parse_from(["devkit", "init", "-i", "-p"]).unwrap();
        match cli.command {
            Commands::Init {
                interactive, print, ..
            } => {
                assert!(interactive);
                assert!(print);
            }
            command => panic!("unexpected command: {command:?}"),
        }

        let cli = Cli::try_parse_from(["devkit", "tui", "-p", "-o", "team.toml"]).unwrap();
        match cli.command {
            Commands::Tui { print, output, .. } => {
                assert!(print);
                assert_eq!(output.to_string_lossy(), "team.toml");
            }
            command => panic!("unexpected command: {command:?}"),
        }

        let cli = Cli::try_parse_from(["devkit", "install", "node", "-n", "-m", "fnm", "-v", "24"])
            .unwrap();
        match cli.command {
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
    fn parses_apply_for_sync() {
        let cli = Cli::try_parse_from(["devkit", "sync", "--apply"]).unwrap();

        match cli.command {
            Commands::Sync { apply, .. } => assert!(apply),
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn rejects_removed_compatibility_commands() {
        assert!(Cli::try_parse_from(["devkit", "add", "node"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "check"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "new"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "up"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "sync", "--yes"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "init", "--stdout"]).is_err());
        assert!(Cli::try_parse_from(["devkit", "upgrade", "--skip-latest"]).is_err());
    }

    #[test]
    fn parses_offline_upgrade() {
        let cli = Cli::try_parse_from(["devkit", "upgrade", "--offline"]).unwrap();

        match cli.command {
            Commands::Upgrade { offline, .. } => assert!(offline),
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_config_subcommands() {
        let cli =
            Cli::try_parse_from(["devkit", "config", "validate", "-j", "-c", "team.toml"]).unwrap();

        match cli.command {
            Commands::Config {
                command: ConfigCommands::Validate { json, config },
            } => {
                assert!(json);
                assert_eq!(config.to_string_lossy(), "team.toml");
            }
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_global_language() {
        let cli = Cli::try_parse_from(["devkit", "--lang", "zh", "doctor"]).unwrap();

        assert_eq!(cli.lang, Some(crate::i18n::Language::Zh));
    }
}
