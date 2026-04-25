use serde::Serialize;

use crate::{
    doctor::{DoctorReport, Status, ToolStatus, version_satisfies},
    latest::{LatestVersion, lookup_latest},
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
        let latest = check_latest.then(|| lookup_latest(&tool.name));
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
    match tool.name.as_str() {
        "fnm" | "bun" | "deno" | "python" | "poetry" | "ruby" | "brew" => {
            Some(format!("brew upgrade {}", tool.name))
        }
        "node" => target.map(|version| format!("fnm install {version} && fnm default {version}")),
        "npm" => Some("npm install -g npm@latest".to_string()),
        "pnpm" => Some("corepack prepare pnpm@latest --activate".to_string()),
        "yarn" => Some("corepack prepare yarn@stable --activate".to_string()),
        "wrangler" => Some("npm install -g wrangler@latest".to_string()),
        "uv" => Some("uv self update".to_string()),
        "rustup" | "rustc" | "cargo" => {
            Some("rustup self update && rustup update stable".to_string())
        }
        "go" => target.map(|version| format!("install Go {version} from https://go.dev/dl/")),
        _ => None,
    }
}
