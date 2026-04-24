use anyhow::Result;

use crate::{
    cleanup::CleanupPlan,
    doctor::{DoctorReport, Status},
    upgrade::UpgradePlan,
};

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
    }

    if !report.issues.is_empty() {
        println!();
        println!("Issues:");
        for issue in &report.issues {
            println!("- {}", issue.message);
            if let Some(path) = &issue.path {
                println!("  path: {}", path.display());
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

fn format_status(status: &Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Missing => "missing",
        Status::Mismatch => "mismatch",
        Status::Unknown => "unknown",
    }
}
