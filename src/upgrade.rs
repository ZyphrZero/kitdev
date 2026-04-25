use serde::Serialize;

use crate::{
    config::ToolPolicy,
    doctor::{DoctorReport, Status, ToolStatus, version_satisfies},
    latest::{LatestVersion, lookup_latest_for},
    tool_command::{ToolCommand, command_for_tool},
};

#[derive(Debug, Serialize)]
pub struct UpgradePlan {
    pub dry_run: bool,
    pub checked_latest: bool,
    pub candidates: Vec<UpgradeCandidate>,
}

#[derive(Debug, Serialize)]
pub struct UpgradeCandidate {
    pub tool: String,
    pub current: Option<String>,
    pub required: Option<String>,
    pub latest: Option<String>,
    pub latest_source: Option<String>,
    pub command: Option<String>,
    pub note: String,
}

pub fn build_upgrade_plan(dry_run: bool, check_latest: bool, report: &DoctorReport) -> UpgradePlan {
    let mut candidates = Vec::new();

    for tool in &report.tools {
        let latest = check_latest.then(|| lookup_latest_for(&tool.name, tool.manager.as_deref()));
        let target = upgrade_target(tool, latest.as_ref());
        let should_upgrade = should_upgrade(tool, target.as_deref());

        if should_upgrade {
            candidates.push(UpgradeCandidate {
                tool: tool.name.clone(),
                current: tool.current.clone(),
                required: tool.required.clone(),
                latest: latest.as_ref().and_then(|latest| latest.version.clone()),
                latest_source: latest.as_ref().map(|latest| latest.source.to_string()),
                command: upgrade_command(tool, target.as_deref()),
                note: upgrade_note(tool, latest.as_ref(), target.as_deref()),
            });
        }
    }

    if candidates.is_empty() {
        candidates.push(UpgradeCandidate {
            tool: "all".to_string(),
            current: None,
            required: None,
            latest: None,
            latest_source: None,
            command: None,
            note: "no missing tools, configured mismatches, or latest-version upgrades detected"
                .to_string(),
        });
    }

    UpgradePlan {
        dry_run,
        checked_latest: check_latest,
        candidates,
    }
}

fn upgrade_target(tool: &ToolStatus, latest: Option<&LatestVersion>) -> Option<String> {
    match tool.required.as_deref() {
        Some("latest") | Some("stable") => latest.and_then(|latest| latest.version.clone()),
        Some(required) => Some(required.to_string()),
        None => latest.and_then(|latest| latest.version.clone()),
    }
}

fn should_upgrade(tool: &ToolStatus, target: Option<&str>) -> bool {
    if matches!(tool.status, Status::Missing | Status::Mismatch) {
        return true;
    }

    let Some(current) = &tool.current else {
        return false;
    };
    let Some(target) = target else {
        return false;
    };

    !version_satisfies(current, target)
}

fn upgrade_note(tool: &ToolStatus, latest: Option<&LatestVersion>, target: Option<&str>) -> String {
    if matches!(tool.status, Status::Missing) {
        return "tool is missing".to_string();
    }

    if matches!(tool.status, Status::Mismatch) {
        return "tool does not match configured policy".to_string();
    }

    if let Some(note) = latest.and_then(|latest| latest.note.clone()) {
        return format!("newer version available ({note})");
    }

    if target.is_some() {
        "newer version available".to_string()
    } else {
        "latest version could not be determined".to_string()
    }
}

fn upgrade_command(tool: &ToolStatus, target: Option<&str>) -> Option<String> {
    if matches!(tool.name.as_str(), "rustup" | "rustc" | "cargo") {
        if matches!(tool.status, Status::Missing) {
            let policy = ToolPolicy {
                version: target.map(ToString::to_string),
                manager: tool.manager.clone().or_else(|| Some("rustup".to_string())),
                source: tool.manager.clone().or_else(|| Some("rustup".to_string())),
                ..ToolPolicy::default()
            };
            return command_for_tool("rust", &policy, true)
                .ok()
                .map(|command| match command {
                    ToolCommand::Shell(command) | ToolCommand::Manual(command) => command,
                });
        }
        return Some("rustup self update && rustup update stable".to_string());
    }

    let install = matches!(tool.status, Status::Missing)
        || tool.note.as_deref() == Some("installed path does not match configured manager");
    let policy = ToolPolicy {
        version: target.map(ToString::to_string),
        manager: tool.manager.clone(),
        source: tool.manager.clone(),
        ..ToolPolicy::default()
    };

    command_for_tool(&tool.name, &policy, install)
        .ok()
        .map(|command| match command {
            ToolCommand::Shell(command) | ToolCommand::Manual(command) => command,
        })
}

#[cfg(test)]
mod tests {
    use crate::doctor::{DoctorReport, Status, ToolStatus};

    use super::build_upgrade_plan;

    #[test]
    fn missing_tools_use_install_commands() {
        let report = DoctorReport {
            tools: vec![ToolStatus {
                name: "deno".to_string(),
                command: "deno".to_string(),
                path: None,
                path_candidates: Vec::new(),
                current: None,
                required: Some("latest".to_string()),
                manager: Some("brew".to_string()),
                status: Status::Missing,
                note: Some("command not found in PATH".to_string()),
            }],
            issues: Vec::new(),
        };

        let plan = build_upgrade_plan(true, false, &report);

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(
            plan.candidates[0].command.as_deref(),
            Some("brew install deno")
        );
    }
}
