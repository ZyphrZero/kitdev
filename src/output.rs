use anyhow::Result;
use std::path::Path;

use crate::{
    cleanup::CleanupPlan,
    config::{ConfigExplainReport, ConfigValidationLevel, ConfigValidationReport},
    doctor::{DoctorReport, IssueSeverity, Status},
    i18n::{Label, Messages, Text},
    install::{InstallExecution, InstallPlan, InstallStatus},
    sync::{SyncExecution, SyncPlan},
    upgrade::UpgradePlan,
};

pub fn print_config_validation_report(
    report: &ConfigValidationReport,
    json: bool,
    messages: &Messages,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    if report.items.is_empty() {
        println!("{}", messages.text(Text::ConfigValidationOk));
        return Ok(());
    }

    println!(
        "{}: {}",
        messages.text(Text::ConfigValidation),
        if report.has_errors() {
            messages.text(Text::ErrorsFound)
        } else {
            messages.text(Text::WarningsOnly)
        }
    );
    for item in &report.items {
        println!(
            "- {} {}: {}",
            format_config_validation_level(item.level, messages),
            item.path,
            item.message
        );
    }

    Ok(())
}

pub fn print_config_explain_report(
    report: &ConfigExplainReport,
    json: bool,
    messages: &Messages,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("{}:", messages.text(Text::ConfigExplain));
    println!("{}: {}", messages.text(Text::Platform), report.platform);
    if report.applied_overrides.is_empty() {
        println!(
            "{}: {}",
            messages.text(Text::AppliedOverrides),
            messages.text(Text::None)
        );
    } else {
        println!(
            "{}: {}",
            messages.text(Text::AppliedOverrides),
            report.applied_overrides.join(", ")
        );
    }

    if report.entries.is_empty() {
        println!(
            "{}: {}",
            messages.text(Text::Entries),
            messages.text(Text::None)
        );
        return Ok(());
    }

    println!("{}:", messages.text(Text::Entries));
    for entry in &report.entries {
        println!("- {} = {} ({})", entry.path, entry.value, entry.source);
    }

    Ok(())
}

pub fn print_doctor_report(report: &DoctorReport, json: bool, messages: &Messages) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "{:<9}  {:<13}  {:<13}  {:<10}  {}",
        messages.text(Text::Tool),
        messages.text(Text::Current),
        messages.text(Text::Required),
        messages.text(Text::Status),
        messages.text(Text::Path)
    );
    println!("---------  -------------  -------------  ----------  ----");
    for tool in &report.tools {
        println!(
            "{:<9}  {:<13}  {:<13}  {:<10}  {}",
            tool.name,
            tool.current.as_deref().unwrap_or("-"),
            tool.required.as_deref().unwrap_or("-"),
            format_status(&tool.status, messages),
            tool.path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        if tool.path_candidates.len() > 1 {
            println!(
                "           {}: {}",
                messages.text(Text::PathCandidates),
                tool.path_candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
    }

    if !report.issues.is_empty() {
        println!();
        println!("{}:", messages.text(Text::Issues));
        for issue in &report.issues {
            println!(
                "- [{}] {}",
                format_issue_severity(issue.severity, messages),
                issue.message
            );
            if let Some(path) = &issue.path {
                println!("  {}: {}", messages.text(Text::Path), path.display());
            }
            for evidence in &issue.evidence {
                println!("  {}: {}", evidence.key, evidence.value);
            }
            if let Some(fix) = &issue.fix {
                println!("  {}: {fix}", messages.text(Text::Fix));
            }
        }
    }

    Ok(())
}

pub fn print_upgrade_plan(plan: &UpgradePlan, json: bool, messages: &Messages) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "{}{}{}:",
        messages.text(Text::UpgradePlan),
        messages.dry_run_suffix(plan.dry_run),
        if plan.checked_latest {
            messages.text(Text::WithLatestChecks)
        } else {
            ""
        }
    );
    for candidate in &plan.candidates {
        println!("- {}: {}", candidate.tool, candidate.note);
        if let Some(current) = &candidate.current {
            println!("  {}: {current}", messages.text(Text::Current));
        }
        if let Some(required) = &candidate.required {
            println!("  {}: {required}", messages.text(Text::Required));
        }
        if let Some(latest) = &candidate.latest {
            println!("  latest: {latest}");
        }
        if let Some(source) = &candidate.latest_source {
            println!("  {}: {source}", messages.text(Text::Source));
        }
        if let Some(command) = &candidate.command {
            println!("  {}: {command}", messages.text(Text::Command));
        }
    }

    Ok(())
}

pub fn print_cleanup_plan(plan: &CleanupPlan, json: bool, messages: &Messages) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "{}{}:",
        messages.text(Text::CleanupPlan),
        messages.dry_run_suffix(plan.dry_run)
    );
    if plan.items.is_empty() {
        println!("- {}", messages.text(Text::NoLegacyLeftovers));
        return Ok(());
    }

    for item in &plan.items {
        println!("- {}", item.reason);
        println!("  {}: {}", messages.text(Text::Path), item.path.display());
        println!("  {}: {}", messages.text(Text::Command), item.command);
        if item.requires_sudo {
            println!("  {}", messages.text(Text::RequiresSudoYes));
        }
    }

    Ok(())
}

pub fn print_init_document(content: &str) -> Result<()> {
    print!("{content}");
    Ok(())
}

pub fn print_init_result(path: &Path, overwritten: bool, messages: &Messages) -> Result<()> {
    if overwritten {
        println!("{} {}", messages.text(Text::Overwrote), path.display());
    } else {
        println!("{} {}", messages.text(Text::Wrote), path.display());
    }
    Ok(())
}

pub fn print_init_cancelled(messages: &Messages) -> Result<()> {
    println!("{}", messages.text(Text::Cancelled));
    Ok(())
}

pub fn print_install_plan(plan: &InstallPlan, json: bool, messages: &Messages) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "{}{}:",
        messages.text(Text::InstallPlan),
        messages.dry_run_suffix(plan.dry_run)
    );
    println!("{}: {}", messages.text(Text::TargetTool), plan.tool);
    for step in &plan.steps {
        println!(
            "- {} {}: {}",
            format_kind(&step.kind, messages),
            step.target,
            step.reason
        );
        print_blockers(&step.blocked_by, messages);
        print_step_command(step.command.as_deref(), step.manual, messages);
        if step.requires_sudo {
            println!("  {}", messages.text(Text::RequiresSudoYes));
        }
    }

    Ok(())
}

pub fn print_install_execution(
    execution: &InstallExecution,
    json: bool,
    messages: &Messages,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(execution)?);
        return Ok(());
    }

    println!("{}:", messages.text(Text::InstallExecution));
    println!("{}: {}", messages.text(Text::TargetTool), execution.tool);
    for step in &execution.steps {
        println!(
            "- {} {}: {}",
            format_execution_status(&step.status, messages),
            step.target,
            step.detail
        );
        print_blockers(&step.blocked_by, messages);
        print_step_command(step.command.as_deref(), step.manual, messages);
    }
    print_install_status(&execution.status, messages);
    let applied = execution
        .steps
        .iter()
        .any(|step| matches!(step.status, crate::sync::SyncStepExecutionStatus::Applied));
    if execution.succeeded && applied {
        println!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultInstallCompleted)
        );
    } else if execution.succeeded {
        println!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultInstallNotNeeded)
        );
    } else {
        println!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultInstallFailed)
        );
    }

    Ok(())
}

pub fn print_sync_plan(plan: &SyncPlan, json: bool, messages: &Messages) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "{}{}:",
        messages.text(Text::SyncPlan),
        messages.dry_run_suffix(plan.dry_run)
    );
    if let Some(channel) = &plan.policy_channel {
        println!("{}: {channel}", messages.text(Text::TargetChannel));
    }
    if let Some(platform) = &plan.platform {
        println!("{}: {platform}", messages.text(Text::TargetPlatform));
    }
    if plan.auto_fix {
        println!("{}", messages.text(Text::PolicyAutoFixEnabled));
    }

    let ready_steps = plan
        .graph
        .ready
        .iter()
        .filter_map(|id| plan.steps.iter().find(|step| step.id == *id))
        .collect::<Vec<_>>();
    if !ready_steps.is_empty() {
        println!("{}:", messages.text(Text::Ready));
        for step in ready_steps {
            print_sync_plan_step(step, messages);
        }
    }

    if !plan.graph.blocked.is_empty() {
        println!("{}:", messages.text(Text::Blocked));
        for blocked in &plan.graph.blocked {
            if let Some(step) = plan.steps.iter().find(|step| step.id == blocked.id) {
                print_sync_plan_step(step, messages);
            }
        }
    }

    let verify_steps = plan
        .steps
        .iter()
        .filter(|step| matches!(step.kind, crate::sync::SyncStepKind::Verify))
        .collect::<Vec<_>>();
    if !verify_steps.is_empty() {
        println!("{}:", messages.text(Text::Verify));
        for step in verify_steps {
            print_sync_plan_step(step, messages);
        }
    }

    Ok(())
}

pub fn print_sync_execution(
    execution: &SyncExecution,
    json: bool,
    messages: &Messages,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(execution)?);
        return Ok(());
    }

    println!("{}:", messages.text(Text::SyncExecution));
    if let Some(channel) = &execution.policy_channel {
        println!("{}: {channel}", messages.text(Text::TargetChannel));
    }
    if let Some(platform) = &execution.platform {
        println!("{}: {platform}", messages.text(Text::TargetPlatform));
    }
    if execution.auto_fix {
        println!("{}", messages.text(Text::PolicyAutoFixEnabled));
    }
    for step in &execution.steps {
        println!(
            "- {} {}: {}",
            format_execution_status(&step.status, messages),
            step.target,
            step.detail
        );
        print_blockers(&step.blocked_by, messages);
        print_step_command(step.command.as_deref(), step.manual, messages);
        if let Some(file) = &step.file {
            println!("  {}: {}", messages.text(Text::File), file.display());
        }
    }
    if execution.succeeded {
        println!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultEnvironmentMatchesPolicy)
        );
    } else {
        println!(
            "{}: {}",
            messages.text(Text::Result),
            messages.text(Text::ResultSyncStopped)
        );
    }

    Ok(())
}

fn print_install_status(status: &InstallStatus, messages: &Messages) {
    match status {
        InstallStatus::KnownTool(tool) => {
            println!(
                "{}: {} {} ({})",
                messages.text(Text::Detected),
                tool.name,
                tool.current.as_deref().unwrap_or("-"),
                format_status(&tool.status, messages)
            );
            if let Some(path) = &tool.path {
                println!("{}: {}", messages.text(Text::Path), path.display());
            }
        }
        InstallStatus::CommandVisible { command, path } => {
            println!("{}: {command}", messages.text(Text::Detected));
            println!("{}: {}", messages.text(Text::Path), path.display());
        }
        InstallStatus::CommandNotVisible { command } => {
            println!(
                "{}: {command} {}",
                messages.text(Text::Detected),
                messages.text(Text::DetectedNotVisibleInPath)
            );
        }
    }
}

fn print_step_command(command: Option<&str>, manual: bool, messages: &Messages) {
    if let Some(command) = command {
        if manual {
            println!("  {}: {command}", messages.text(Text::Instruction));
        } else {
            println!("  {}: {command}", messages.text(Text::Command));
        }
    }
}

fn print_sync_plan_step(step: &crate::sync::SyncStep, messages: &Messages) {
    println!(
        "- {} {}: {}",
        format_kind(&step.kind, messages),
        step.target,
        step.reason
    );
    print_blockers(&step.blocked_by, messages);
    print_step_command(step.command.as_deref(), step.manual, messages);
    if let Some(file) = &step.file {
        println!("  {}: {}", messages.text(Text::File), file.display());
    }
    if let Some(snippet) = &step.snippet {
        println!(
            "  {}: {}",
            messages.text(Text::Snippet),
            snippet.replace('\n', "\n           ")
        );
    }
    if step.requires_sudo {
        println!("  {}", messages.text(Text::RequiresSudoYes));
    }
}

fn print_blockers(blocked_by: &[String], messages: &Messages) {
    if !blocked_by.is_empty() {
        println!(
            "  {}: {}",
            messages.text(Text::BlockedBy),
            blocked_by.join(", ")
        );
    }
}

fn format_status(status: &Status, messages: &Messages) -> &'static str {
    match status {
        Status::Ok => messages.label(Label::Ok),
        Status::Missing => messages.label(Label::Missing),
        Status::Mismatch => messages.label(Label::Mismatch),
        Status::Unknown => messages.label(Label::Unknown),
    }
}

fn format_config_validation_level(
    level: ConfigValidationLevel,
    messages: &Messages,
) -> &'static str {
    match level {
        ConfigValidationLevel::Error => messages.label(Label::Error),
        ConfigValidationLevel::Warning => messages.label(Label::Warning),
    }
}

fn format_issue_severity(severity: IssueSeverity, messages: &Messages) -> &'static str {
    match severity {
        IssueSeverity::Error => messages.label(Label::Error),
        IssueSeverity::Warning => messages.label(Label::Warning),
        IssueSeverity::Info => messages.label(Label::Info),
    }
}

fn format_kind(kind: &crate::sync::SyncStepKind, messages: &Messages) -> &'static str {
    match kind {
        crate::sync::SyncStepKind::Install => messages.label(Label::Install),
        crate::sync::SyncStepKind::Align => messages.label(Label::Align),
        crate::sync::SyncStepKind::Configure => messages.label(Label::Configure),
        crate::sync::SyncStepKind::Cleanup => messages.label(Label::Cleanup),
        crate::sync::SyncStepKind::Verify => messages.label(Label::Verify),
        crate::sync::SyncStepKind::Info => messages.label(Label::Info),
    }
}

fn format_execution_status(
    status: &crate::sync::SyncStepExecutionStatus,
    messages: &Messages,
) -> &'static str {
    match status {
        crate::sync::SyncStepExecutionStatus::Applied => messages.label(Label::Applied),
        crate::sync::SyncStepExecutionStatus::Unchanged => messages.label(Label::Unchanged),
        crate::sync::SyncStepExecutionStatus::Skipped => messages.label(Label::Skipped),
        crate::sync::SyncStepExecutionStatus::Failed => messages.label(Label::Failed),
        crate::sync::SyncStepExecutionStatus::Verified => messages.label(Label::Verified),
    }
}
