mod cleanup;
mod cli;
mod config;
mod doctor;
mod latest;
mod output;
mod shell;
mod sync;
mod upgrade;

use anyhow::Result;
use clap::Parser;

use crate::{
    cleanup::build_cleanup_plan,
    cli::{Cli, Commands},
    config::DevkitConfig,
    doctor::build_doctor_report,
    output::{
        print_cleanup_plan, print_doctor_report, print_sync_execution, print_sync_plan,
        print_upgrade_plan,
    },
    sync::{build_sync_plan, execute_sync_plan},
    upgrade::build_upgrade_plan,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor { json, config } => {
            let config = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&config);
            print_doctor_report(&report, json)?;
        }
        Commands::Upgrade {
            dry_run,
            json,
            config,
            skip_latest,
        } => {
            let config = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&config);
            let plan = build_upgrade_plan(dry_run, !skip_latest, &report);
            print_upgrade_plan(&plan, json)?;
        }
        Commands::Sync {
            dry_run,
            yes,
            json,
            config,
        } => {
            let dry_run = dry_run || !yes;
            let policy = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&policy);
            let plan = build_sync_plan(dry_run, &config, &policy, &report);
            if dry_run {
                print_sync_plan(&plan, json)?;
            } else {
                let execution = execute_sync_plan(&plan, &policy);
                let succeeded = execution.succeeded;
                print_sync_execution(&execution, json)?;
                if !succeeded {
                    std::process::exit(1);
                }
            }
        }
        Commands::Cleanup { dry_run, json } => {
            let plan = build_cleanup_plan(dry_run);
            print_cleanup_plan(&plan, json)?;
        }
    }

    Ok(())
}
