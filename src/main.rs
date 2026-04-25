mod cleanup;
mod cli;
mod config;
mod doctor;
mod i18n;
mod init;
mod init_tui;
mod install;
mod latest;
mod output;
mod package_manager;
mod platform;
mod shell;
mod sync;
mod tool_command;
mod upgrade;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use crate::{
    cleanup::build_cleanup_plan,
    cli::{Cli, Commands, ConfigCommands},
    config::DevkitConfig,
    doctor::build_doctor_report,
    i18n::{Language, Messages},
    init::{
        InitInteractionOutcome, InitInteractiveOptions, build_init_draft,
        customize_init_draft_interactively_with_options, render_init_document, write_init_document,
    },
    install::{build_install_plan, execute_install_plan},
    output::{
        print_cleanup_plan, print_config_explain_report, print_config_validation_report,
        print_doctor_report, print_init_cancelled, print_init_document, print_init_result,
        print_install_execution, print_install_plan, print_sync_execution, print_sync_plan,
        print_upgrade_plan,
    },
    sync::{build_sync_plan, execute_sync_plan},
    upgrade::build_upgrade_plan,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let messages = Messages::new(Language::detect(cli.lang));
    let command = cli.command;

    match command {
        Commands::Doctor { json, config } => {
            let config = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&config);
            print_doctor_report(&report, json, &messages)?;
        }
        Commands::Upgrade {
            dry_run,
            json,
            config,
            offline,
        } => {
            let config = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&config);
            let plan = build_upgrade_plan(dry_run, !offline, &report);
            print_upgrade_plan(&plan, json, &messages)?;
        }
        Commands::Init {
            interactive,
            print,
            output,
            force,
        } => {
            run_init_flow(interactive, print, output, force, &messages)?;
        }
        Commands::Tui {
            print,
            output,
            force,
        } => {
            run_init_flow(true, print, output, force, &messages)?;
        }
        Commands::Install {
            tool,
            dry_run,
            json,
            manager,
            version,
            config,
        } => {
            let policy = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&policy);
            let plan = build_install_plan(
                dry_run,
                &tool,
                manager.as_deref(),
                version.as_deref(),
                &policy,
                &report,
            )?;
            if dry_run {
                print_install_plan(&plan, json, &messages)?;
            } else {
                let execution = execute_install_plan(&plan, &policy);
                let succeeded = execution.succeeded;
                print_install_execution(&execution, json, &messages)?;
                if !succeeded {
                    std::process::exit(1);
                }
            }
        }
        Commands::Sync {
            dry_run,
            apply,
            json,
            config,
        } => {
            let dry_run = dry_run || !apply;
            let policy = DevkitConfig::read(&config)?;
            let report = build_doctor_report(&policy);
            let plan = build_sync_plan(dry_run, &config, &policy, &report);
            if dry_run {
                print_sync_plan(&plan, json, &messages)?;
            } else {
                let execution = execute_sync_plan(&plan, &policy);
                let succeeded = execution.succeeded;
                print_sync_execution(&execution, json, &messages)?;
                if !succeeded {
                    std::process::exit(1);
                }
            }
        }
        Commands::Cleanup { dry_run, json } => {
            let plan = build_cleanup_plan(dry_run);
            print_cleanup_plan(&plan, json, &messages)?;
        }
        Commands::Config { command } => match command {
            ConfigCommands::Validate { json, config } => {
                let config = DevkitConfig::read_raw(&config)?;
                let report = config.validate_for_current_platform();
                let has_errors = report.has_errors();
                print_config_validation_report(&report, json, &messages)?;
                if has_errors {
                    std::process::exit(1);
                }
            }
            ConfigCommands::Explain { json, config } => {
                let config = DevkitConfig::read_raw(&config)?;
                let report = config.explain_for_current_platform();
                print_config_explain_report(&report, json, &messages)?;
            }
        },
    }

    Ok(())
}

fn run_init_flow(
    interactive: bool,
    stdout: bool,
    output: PathBuf,
    force: bool,
    messages: &Messages,
) -> Result<()> {
    let report = build_doctor_report(&DevkitConfig::default());
    let mut draft = build_init_draft(&report);
    if interactive {
        match customize_init_draft_interactively_with_options(
            &mut draft,
            InitInteractiveOptions {
                output: output.clone(),
                force,
                stdout,
                language: messages.language(),
            },
        )? {
            InitInteractionOutcome::Continue => {}
            InitInteractionOutcome::Handled => return Ok(()),
            InitInteractionOutcome::Cancelled => {
                print_init_cancelled(messages)?;
                return Ok(());
            }
        }
    }
    let document = render_init_document(&draft);
    if stdout {
        print_init_document(&document.content)?;
    } else {
        let result = write_init_document(&output, force, &document)?;
        print_init_result(&result.path, result.overwritten, messages)?;
    }
    Ok(())
}
