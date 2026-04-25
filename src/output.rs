use anyhow::Result;
use std::path::Path;

use crate::{
    cleanup::CleanupPlan,
    config::{ConfigExplainReport, ConfigValidationLevel, ConfigValidationReport},
    doctor::{DoctorReport, IssueSeverity, Status},
    install::{InstallExecution, InstallPlan, InstallStatus},
    sync::{SyncExecution, SyncPlan},
    upgrade::UpgradePlan,
};

pub fn print_config_validation_report(report: &ConfigValidationReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    if report.items.is_empty() {
        println!("Config validation: ok");
        return Ok(());
    }

    println!(
        "Config validation: {}",
        if report.has_errors() {
            "errors found"
        } else {
            "warnings only"
        }
    );
    for item in &report.items {
        println!(
            "- {} {}: {}",
            format_config_validation_level(item.level),
            item.path,
            item.message
        );
    }

    Ok(())
}

pub fn print_config_explain_report(report: &ConfigExplainReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("Config explain:");
    println!("platform: {}", report.platform);
    if report.applied_overrides.is_empty() {
        println!("applied overrides: none");
    } else {
        println!("applied overrides: {}", report.applied_overrides.join(", "));
    }

    if report.entries.is_empty() {
        println!("entries: none");
        return Ok(());
    }

    println!("entries:");
    for entry in &report.entries {
        println!("- {} = {} ({})", entry.path, entry.value, entry.source);
    }

    Ok(())
}

pub fn print_doctor_report(report: &DoctorReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!("Tool       Current        Required       Status      Path");
    println!("---------  -------------  -------------  ----------  ----");
    for tool in &report.tools {
        println!(
            "{:<9}  {:<13}  {:<13}  {:<10}  {}",
            tool.name,
            tool.current.as_deref().unwrap_or("-"),
            tool.required.as_deref().unwrap_or("-"),
            format_status(&tool.status),
            tool.path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        if tool.path_candidates.len() > 1 {
            println!(
                "           PATH candidates: {}",
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
        println!("Issues:");
        for issue in &report.issues {
            println!(
                "- [{}] {}",
                format_issue_severity(issue.severity),
                issue.message
            );
            if let Some(path) = &issue.path {
                println!("  path: {}", path.display());
            }
            for evidence in &issue.evidence {
                println!("  {}: {}", evidence.key, evidence.value);
            }
            if let Some(fix) = &issue.fix {
                println!("  fix: {fix}");
            }
        }
    }

    Ok(())
}

pub fn print_upgrade_plan(plan: &UpgradePlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "Upgrade plan{}{}:",
        if plan.dry_run { " (dry-run)" } else { "" },
        if plan.checked_latest {
            " with latest checks"
        } else {
            ""
        }
    );
    for candidate in &plan.candidates {
        println!("- {}: {}", candidate.tool, candidate.note);
        if let Some(current) = &candidate.current {
            println!("  current: {current}");
        }
        if let Some(required) = &candidate.required {
            println!("  required: {required}");
        }
        if let Some(latest) = &candidate.latest {
            println!("  latest: {latest}");
        }
        if let Some(source) = &candidate.latest_source {
            println!("  source: {source}");
        }
        if let Some(command) = &candidate.command {
            println!("  command: {command}");
        }
    }

    Ok(())
}

pub fn print_cleanup_plan(plan: &CleanupPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "Cleanup plan{}:",
        if plan.dry_run { " (dry-run)" } else { "" }
    );
    if plan.items.is_empty() {
        println!("- no known legacy toolchain leftovers found");
        return Ok(());
    }

    for item in &plan.items {
        println!("- {}", item.reason);
        println!("  path: {}", item.path.display());
        println!("  command: {}", item.command);
        if item.requires_sudo {
            println!("  requires sudo: yes");
        }
    }

    Ok(())
}

pub fn print_init_document(content: &str) -> Result<()> {
    print!("{content}");
    Ok(())
}

pub fn print_init_result(path: &Path, overwritten: bool) -> Result<()> {
    if overwritten {
        println!("Overwrote {}", path.display());
    } else {
        println!("Wrote {}", path.display());
    }
    Ok(())
}

pub fn print_init_cancelled() -> Result<()> {
    println!("Cancelled");
    Ok(())
}

pub fn print_install_plan(plan: &InstallPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!(
        "Install plan{}:",
        if plan.dry_run { " (dry-run)" } else { "" }
    );
    println!("target tool: {}", plan.tool);
    for step in &plan.steps {
        println!(
            "- {} {}: {}",
            format_kind(&step.kind),
            step.target,
            step.reason
        );
        print_blockers(&step.blocked_by);
        print_step_command(step.command.as_deref(), step.manual);
        if step.requires_sudo {
            println!("  requires sudo: yes");
        }
    }

    Ok(())
}

pub fn print_install_execution(execution: &InstallExecution, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(execution)?);
        return Ok(());
    }

    println!("Install execution:");
    println!("target tool: {}", execution.tool);
    for step in &execution.steps {
        println!(
            "- {} {}: {}",
            format_execution_status(&step.status),
            step.target,
            step.detail
        );
        print_blockers(&step.blocked_by);
        print_step_command(step.command.as_deref(), step.manual);
    }
    print_install_status(&execution.status);
    let applied = execution
        .steps
        .iter()
        .any(|step| matches!(step.status, crate::sync::SyncStepExecutionStatus::Applied));
    if execution.succeeded && applied {
        println!("result: install command completed");
    } else if execution.succeeded {
        println!("result: no install command needed");
    } else {
        println!("result: install command failed");
    }

    Ok(())
}

pub fn print_sync_plan(plan: &SyncPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    println!("Sync plan{}:", if plan.dry_run { " (dry-run)" } else { "" });
    if let Some(channel) = &plan.policy_channel {
        println!("target channel: {channel}");
    }
    if let Some(platform) = &plan.platform {
        println!("target platform: {platform}");
    }
    if plan.auto_fix {
        println!("policy auto-fix: enabled");
    }

    let ready_steps = plan
        .graph
        .ready
        .iter()
        .filter_map(|id| plan.steps.iter().find(|step| step.id == *id))
        .collect::<Vec<_>>();
    if !ready_steps.is_empty() {
        println!("ready:");
        for step in ready_steps {
            print_sync_plan_step(step);
        }
    }

    if !plan.graph.blocked.is_empty() {
        println!("blocked:");
        for blocked in &plan.graph.blocked {
            if let Some(step) = plan.steps.iter().find(|step| step.id == blocked.id) {
                print_sync_plan_step(step);
            }
        }
    }

    let verify_steps = plan
        .steps
        .iter()
        .filter(|step| matches!(step.kind, crate::sync::SyncStepKind::Verify))
        .collect::<Vec<_>>();
    if !verify_steps.is_empty() {
        println!("verify:");
        for step in verify_steps {
            print_sync_plan_step(step);
        }
    }

    Ok(())
}

pub fn print_sync_execution(execution: &SyncExecution, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(execution)?);
        return Ok(());
    }

    println!("Sync execution:");
    if let Some(channel) = &execution.policy_channel {
        println!("target channel: {channel}");
    }
    if let Some(platform) = &execution.platform {
        println!("target platform: {platform}");
    }
    if execution.auto_fix {
        println!("policy auto-fix: enabled");
    }
    for step in &execution.steps {
        println!(
            "- {} {}: {}",
            format_execution_status(&step.status),
            step.target,
            step.detail
        );
        print_blockers(&step.blocked_by);
        print_step_command(step.command.as_deref(), step.manual);
        if let Some(file) = &step.file {
            println!("  file: {}", file.display());
        }
    }
    if execution.succeeded {
        println!("result: environment matches policy");
    } else {
        println!("result: sync stopped before reaching the configured policy");
    }

    Ok(())
}

fn print_install_status(status: &InstallStatus) {
    match status {
        InstallStatus::KnownTool(tool) => {
            println!(
                "detected: {} {} ({})",
                tool.name,
                tool.current.as_deref().unwrap_or("-"),
                format_status(&tool.status)
            );
            if let Some(path) = &tool.path {
                println!("path: {}", path.display());
            }
        }
        InstallStatus::CommandVisible { command, path } => {
            println!("detected: {command}");
            println!("path: {}", path.display());
        }
        InstallStatus::CommandNotVisible { command } => {
            println!("detected: {command} is not visible in PATH yet");
        }
    }
}

fn print_step_command(command: Option<&str>, manual: bool) {
    if let Some(command) = command {
        if manual {
            println!("  instruction: {command}");
        } else {
            println!("  command: {command}");
        }
    }
}

fn print_sync_plan_step(step: &crate::sync::SyncStep) {
    println!(
        "- {} {}: {}",
        format_kind(&step.kind),
        step.target,
        step.reason
    );
    print_blockers(&step.blocked_by);
    print_step_command(step.command.as_deref(), step.manual);
    if let Some(file) = &step.file {
        println!("  file: {}", file.display());
    }
    if let Some(snippet) = &step.snippet {
        println!("  snippet: {}", snippet.replace('\n', "\n           "));
    }
    if step.requires_sudo {
        println!("  requires sudo: yes");
    }
}

fn print_blockers(blocked_by: &[String]) {
    if !blocked_by.is_empty() {
        println!("  blocked by: {}", blocked_by.join(", "));
    }
}

fn format_status(status: &Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Missing => "missing",
        Status::Mismatch => "mismatch",
        Status::Unknown => "unknown",
    }
}

fn format_config_validation_level(level: ConfigValidationLevel) -> &'static str {
    match level {
        ConfigValidationLevel::Error => "error",
        ConfigValidationLevel::Warning => "warning",
    }
}

fn format_issue_severity(severity: IssueSeverity) -> &'static str {
    match severity {
        IssueSeverity::Error => "error",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Info => "info",
    }
}

fn format_kind(kind: &crate::sync::SyncStepKind) -> &'static str {
    match kind {
        crate::sync::SyncStepKind::Install => "install",
        crate::sync::SyncStepKind::Align => "align",
        crate::sync::SyncStepKind::Configure => "configure",
        crate::sync::SyncStepKind::Cleanup => "cleanup",
        crate::sync::SyncStepKind::Verify => "verify",
        crate::sync::SyncStepKind::Info => "info",
    }
}

fn format_execution_status(status: &crate::sync::SyncStepExecutionStatus) -> &'static str {
    match status {
        crate::sync::SyncStepExecutionStatus::Applied => "applied",
        crate::sync::SyncStepExecutionStatus::Unchanged => "unchanged",
        crate::sync::SyncStepExecutionStatus::Skipped => "skipped",
        crate::sync::SyncStepExecutionStatus::Failed => "failed",
        crate::sync::SyncStepExecutionStatus::Verified => "verified",
    }
}
