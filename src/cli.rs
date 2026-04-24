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
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect installed tools, versions, PATH, and common conflicts.
    Doctor {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,

        /// Read expected tool versions from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Show upgrade candidates without changing anything.
    Upgrade {
        /// Preview actions only. The MVP currently only supports dry-run.
        #[arg(long, default_value_t = true)]
        dry_run: bool,

        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,

        /// Read expected tool versions from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,

        /// Skip package-manager and official latest-version queries.
        #[arg(long)]
        skip_latest: bool,
    },

    /// Plan how to bootstrap or repair a machine to match the configured toolchain.
    Sync {
        /// Preview actions only.
        #[arg(long, action = ArgAction::SetTrue, conflicts_with = "yes")]
        dry_run: bool,

        /// Apply install, align, and managed shell-configuration steps.
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,

        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,

        /// Read desired toolchain state from a TOML config file.
        #[arg(short, long, default_value = "devkit.toml")]
        config: PathBuf,
    },

    /// Find old or conflicting toolchain leftovers.
    Cleanup {
        /// Preview actions only. The MVP currently only supports dry-run.
        #[arg(long, default_value_t = true)]
        dry_run: bool,

        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}
