use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    config::{DevkitConfig, ToolPolicy},
    platform::{OperatingSystem, current_platform_tag, home_path, path_contains},
    shell::{command_path, command_paths, command_version},
    tool_command::{ToolCommand, command_for_tool_on},
};

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub name: String,
    pub command: String,
    pub path: Option<PathBuf>,
    pub path_candidates: Vec<PathBuf>,
    pub current: Option<String>,
    pub required: Option<String>,
    pub manager: Option<String>,
    pub status: Status,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Ok,
    Missing,
    Mismatch,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub tools: Vec<ToolStatus>,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    pub kind: IssueKind,
    pub severity: IssueSeverity,
    pub path: Option<PathBuf>,
    pub message: String,
    pub evidence: Vec<IssueEvidence>,
    pub fix: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueKind {
    PathConflict,
    LegacyInstall,
    MissingTool,
    PlatformMismatch,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueEvidence {
    pub key: String,
    pub value: String,
}

struct ToolSpec {
    name: &'static str,
    command: &'static str,
    version_args: &'static [&'static str],
    version_prefix: Option<&'static str>,
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "fnm",
        command: "fnm",
        version_args: &["--version"],
        version_prefix: Some("fnm "),
    },
    ToolSpec {
        name: "node",
        command: "node",
        version_args: &["-v"],
        version_prefix: Some("v"),
    },
    ToolSpec {
        name: "npm",
        command: "npm",
        version_args: &["--version"],
        version_prefix: None,
    },
    ToolSpec {
        name: "pnpm",
        command: "pnpm",
        version_args: &["--version"],
        version_prefix: None,
    },
    ToolSpec {
        name: "yarn",
        command: "yarn",
        version_args: &["--version"],
        version_prefix: None,
    },
    ToolSpec {
        name: "bun",
        command: "bun",
        version_args: &["--version"],
        version_prefix: None,
    },
    ToolSpec {
        name: "deno",
        command: "deno",
        version_args: &["--version"],
        version_prefix: Some("deno "),
    },
    ToolSpec {
        name: "wrangler",
        command: "wrangler",
        version_args: &["--version"],
        version_prefix: None,
    },
    ToolSpec {
        name: "uv",
        command: "uv",
        version_args: &["--version"],
        version_prefix: Some("uv "),
    },
    ToolSpec {
        name: "python",
        command: "python3",
        version_args: &["--version"],
        version_prefix: Some("Python "),
    },
    ToolSpec {
        name: "poetry",
        command: "poetry",
        version_args: &["--version"],
        version_prefix: Some("Poetry (version "),
    },
    ToolSpec {
        name: "ruby",
        command: "ruby",
        version_args: &["--version"],
        version_prefix: Some("ruby "),
    },
    ToolSpec {
        name: "rustup",
        command: "rustup",
        version_args: &["--version"],
        version_prefix: Some("rustup "),
    },
    ToolSpec {
        name: "rustc",
        command: "rustc",
        version_args: &["--version"],
        version_prefix: Some("rustc "),
    },
    ToolSpec {
        name: "cargo",
        command: "cargo",
        version_args: &["--version"],
        version_prefix: Some("cargo "),
    },
    ToolSpec {
        name: "go",
        command: "go",
        version_args: &["version"],
        version_prefix: Some("go version go"),
    },
    ToolSpec {
        name: "brew",
        command: "brew",
        version_args: &["--version"],
        version_prefix: Some("Homebrew "),
    },
    ToolSpec {
        name: "winget",
        command: "winget",
        version_args: &["--version"],
        version_prefix: Some("v"),
    },
];

pub fn build_doctor_report(config: &DevkitConfig) -> DoctorReport {
    let platform = OperatingSystem::current();
    let tools = TOOL_SPECS
        .iter()
        .filter(|spec| {
            should_inspect_tool(spec, config, platform, |command| {
                command_path(command).is_some()
            })
        })
        .map(|spec| inspect_tool(spec, config))
        .collect::<Vec<_>>();
    let issues = detect_issues(&tools, config, platform);

    DoctorReport { tools, issues }
}

fn should_inspect_tool(
    spec: &ToolSpec,
    config: &DevkitConfig,
    platform: OperatingSystem,
    command_exists: impl Fn(&str) -> bool,
) -> bool {
    if tool_policy(config, spec.name).is_some()
        || command_exists(spec.command)
        || tool_is_policy_dependency(spec.name, config)
    {
        return true;
    }

    match spec.name {
        "brew" => {
            platform.is_macos()
                || config_references_manager(config, &["brew", "homebrew"])
                || config
                    .homebrew
                    .as_ref()
                    .and_then(|homebrew| homebrew.packages.as_ref())
                    .is_some_and(|packages| !packages.is_empty())
        }
        "winget" => platform.is_windows() || config_references_manager(config, &["winget"]),
        _ => false,
    }
}

fn inspect_tool(spec: &ToolSpec, config: &DevkitConfig) -> ToolStatus {
    let policy = tool_policy(config, spec.name);
    let required = policy.and_then(|policy| policy.version.clone());
    let manager = policy.and_then(|policy| policy.manager.clone());

    let path_candidates = command_paths(spec.command);
    let path = path_candidates.first().cloned();
    let current = path
        .as_ref()
        .and_then(|_| command_version(spec.command, spec.version_args));
    let normalized = current
        .as_deref()
        .map(|version| normalize_version(version, spec.version_prefix));

    let manager_matches = path
        .as_ref()
        .is_none_or(|path| manager_matches_path(manager.as_deref(), path));
    let status = match (&path, &normalized, &required, manager_matches) {
        (None, _, _, _) => Status::Missing,
        (Some(_), _, _, false) => Status::Mismatch,
        (Some(_), Some(current), Some(required), true) if version_satisfies(current, required) => {
            Status::Ok
        }
        (Some(_), Some(_), Some(_), true) => Status::Mismatch,
        (Some(_), Some(_), None, true) => Status::Ok,
        (Some(_), None, _, true) => Status::Unknown,
    };

    let note = match status {
        Status::Missing => Some("command not found in PATH".to_string()),
        Status::Mismatch if !manager_matches => {
            Some("installed path does not match configured manager".to_string())
        }
        Status::Mismatch => Some("installed version does not match configured policy".to_string()),
        Status::Unknown => Some("installed, but version could not be detected".to_string()),
        Status::Ok => None,
    };

    ToolStatus {
        name: spec.name.to_string(),
        command: spec.command.to_string(),
        path,
        path_candidates,
        current: normalized,
        required,
        manager,
        status,
        note,
    }
}

fn tool_policy<'a>(config: &'a DevkitConfig, name: &str) -> Option<&'a ToolPolicy> {
    config.tools.as_ref().and_then(|tools| {
        tools.get(name).or_else(|| match name {
            "rustup" | "rustc" | "cargo" => tools.get("rust"),
            _ => None,
        })
    })
}

fn tool_is_policy_dependency(name: &str, config: &DevkitConfig) -> bool {
    let Some(tools) = config.tools.as_ref() else {
        return false;
    };

    match name {
        "fnm" => tools
            .get("node")
            .and_then(|policy| policy.manager.as_deref())
            .is_some_and(|manager| manager == "fnm"),
        "npm" => {
            tools
                .get("wrangler")
                .and_then(|policy| policy.manager.as_deref())
                .is_some_and(|manager| manager == "npm")
                || tools
                    .get("node")
                    .and_then(|policy| policy.package_managers.as_ref())
                    .is_some_and(|managers| managers.iter().any(|manager| manager == "npm"))
                || config
                    .npm
                    .as_ref()
                    .and_then(|npm| npm.global_packages.as_ref())
                    .is_some_and(|packages| !packages.is_empty())
        }
        "pnpm" | "yarn" | "bun" => tools
            .get("node")
            .and_then(|policy| policy.package_managers.as_ref())
            .is_some_and(|managers| managers.iter().any(|manager| manager == name)),
        "uv" => tools
            .get("python")
            .and_then(|policy| policy.manager.as_deref())
            .is_some_and(|manager| manager == "uv"),
        "rustup" | "rustc" | "cargo" => ["rust", "rustup", "rustc", "cargo"]
            .iter()
            .any(|tool| tools.contains_key(*tool)),
        _ => false,
    }
}

fn manager_matches_path(manager: Option<&str>, path: &Path) -> bool {
    match manager {
        Some("brew" | "homebrew") => OperatingSystem::current().homebrew_path_matches(path),
        Some("fnm") => path_contains(path, &["fnm"]),
        Some("nvm") => path_contains(path, &["/.nvm/", "\\nvm\\", "/nvm/"]),
        _ => true,
    }
}

pub fn normalize_version(raw: &str, prefix: Option<&str>) -> String {
    let mut value = raw.trim().to_string();
    if let Some(prefix) = prefix {
        value = value.trim_start_matches(prefix).trim().to_string();
    }

    value
        .split_whitespace()
        .next()
        .unwrap_or(value.as_str())
        .trim_start_matches('v')
        .trim_end_matches(')')
        .to_string()
}

pub fn version_satisfies(current: &str, required: &str) -> bool {
    let current = current.trim_start_matches('v');
    let required = required.trim_start_matches('v');

    if required.eq_ignore_ascii_case("latest") || required.eq_ignore_ascii_case("stable") {
        return true;
    }

    if let Some(major) = required.strip_suffix(".x") {
        return current.starts_with(&format!("{major}."));
    }

    if required.chars().all(|character| character.is_ascii_digit()) {
        return current.starts_with(&format!("{required}."));
    }

    current == required || current.starts_with(&format!("{required}."))
}

fn detect_issues(
    tools: &[ToolStatus],
    config: &DevkitConfig,
    platform: OperatingSystem,
) -> Vec<Issue> {
    let mut issues = Vec::new();

    if let Some(policy_platform) = config
        .policy
        .as_ref()
        .and_then(|policy| policy.platform.as_deref())
        && !policy_platform_matches_current(policy_platform, platform)
    {
        let current = current_platform_tag();
        issues.push(Issue {
            kind: IssueKind::PlatformMismatch,
            severity: IssueSeverity::Error,
            path: None,
            message: format!("policy targets {policy_platform}, but this machine is {current}"),
            evidence: vec![
                evidence("policy_platform", policy_platform),
                evidence("current_platform", &current),
            ],
            fix: Some("use a platform-specific config or update [policy].platform".to_string()),
        });
    }

    for tool in tools {
        if matches!(tool.status, Status::Missing) {
            issues.push(Issue {
                kind: IssueKind::MissingTool,
                severity: missing_tool_severity(tool),
                path: None,
                message: format!("{} is not available in PATH", tool.name),
                evidence: missing_tool_evidence(tool),
                fix: suggested_install(tool, platform),
            });
        }
    }

    if !platform.is_windows() && path_exists("/usr/local/go") {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            severity: IssueSeverity::Warning,
            path: Some(PathBuf::from("/usr/local/go")),
            message: "legacy official Go install may shadow managed Go versions".to_string(),
            evidence: vec![evidence("path", "/usr/local/go")],
            fix: Some("sudo rm -rf /usr/local/go".to_string()),
        });
    }

    if !platform.is_windows() && path_exists("/usr/local/lib/node_modules") {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            severity: IssueSeverity::Warning,
            path: Some(PathBuf::from("/usr/local/lib/node_modules")),
            message: "legacy global Node modules remain under /usr/local".to_string(),
            evidence: vec![evidence("path", "/usr/local/lib/node_modules")],
            fix: Some("sudo rm -rf /usr/local/lib/node_modules".to_string()),
        });
    }

    let nvm_path = if platform.is_windows() {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_path("AppData/Roaming"))
            .join("nvm")
    } else {
        home_path(".nvm")
    };
    if path_exists(&nvm_path) && !config_references_manager(config, &["nvm"]) {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            severity: IssueSeverity::Warning,
            path: Some(nvm_path),
            message: "nvm directory exists; verify it is not initialized by your shell".to_string(),
            evidence: vec![evidence(
                "path",
                if platform.is_windows() {
                    "%APPDATA%\\nvm"
                } else {
                    "~/.nvm"
                },
            )],
            fix: Some(if platform.is_windows() {
                "remove the nvm directory after confirming it is unused".to_string()
            } else {
                "remove ~/.nvm after confirming it is unused".to_string()
            }),
        });
    }

    detect_path_conflicts(&mut issues, tools);

    issues
}

fn suggested_install(tool: &ToolStatus, platform: OperatingSystem) -> Option<String> {
    if tool.name == "winget" {
        return Some(
            "install App Installer from Microsoft Store or https://aka.ms/getwinget".to_string(),
        );
    }

    let command_tool = match tool.name.as_str() {
        "rustup" | "rustc" | "cargo" => "rust",
        other => other,
    };
    let policy = ToolPolicy {
        version: tool.required.clone(),
        manager: tool.manager.clone(),
        source: tool.manager.clone(),
        ..ToolPolicy::default()
    };

    match command_for_tool_on(command_tool, &policy, true, platform).ok()? {
        ToolCommand::Shell(command) | ToolCommand::Manual(command) => Some(command),
    }
}

fn missing_tool_severity(tool: &ToolStatus) -> IssueSeverity {
    if tool.required.is_some() || tool.manager.is_some() {
        IssueSeverity::Error
    } else {
        IssueSeverity::Info
    }
}

fn missing_tool_evidence(tool: &ToolStatus) -> Vec<IssueEvidence> {
    let mut evidence_items = vec![evidence("command", &tool.command)];
    if let Some(required) = &tool.required {
        evidence_items.push(evidence("required", required));
    }
    if let Some(manager) = &tool.manager {
        evidence_items.push(evidence("manager", manager));
    }
    evidence_items
}

fn evidence(key: &str, value: &str) -> IssueEvidence {
    IssueEvidence {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn detect_path_conflicts(issues: &mut Vec<Issue>, tools: &[ToolStatus]) {
    for tool in tools {
        if tool.path_candidates.len() <= 1 {
            continue;
        }
        let Some(path) = &tool.path else {
            continue;
        };

        issues.push(Issue {
            kind: IssueKind::PathConflict,
            severity: IssueSeverity::Warning,
            path: Some(path.clone()),
            message: format!(
                "{} resolves to the first of {} PATH candidates",
                tool.name,
                tool.path_candidates.len()
            ),
            evidence: vec![
                evidence("active_path", &path.display().to_string()),
                evidence("candidate_count", &tool.path_candidates.len().to_string()),
                evidence("candidates", &format_path_candidates(&tool.path_candidates)),
            ],
            fix: Some(format!(
                "inspect PATH ordering; candidates: {}",
                format_path_candidates(&tool.path_candidates)
            )),
        });
    }
}

fn format_path_candidates(candidates: &[PathBuf]) -> String {
    let mut paths = candidates
        .iter()
        .take(4)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if candidates.len() > paths.len() {
        paths.push(format!("... and {} more", candidates.len() - paths.len()));
    }
    paths.join("; ")
}

fn path_exists(path: impl AsRef<Path>) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn config_references_manager(config: &DevkitConfig, managers: &[&str]) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools.values().any(|policy| {
            [policy.manager.as_deref(), policy.source.as_deref()]
                .into_iter()
                .flatten()
                .any(|manager| managers.contains(&manager))
        })
    })
}

fn policy_platform_matches_current(policy_platform: &str, platform: OperatingSystem) -> bool {
    OperatingSystem::from_platform_tag(policy_platform)
        .is_none_or(|policy_os| policy_os == platform)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        config::{DevkitConfig, ToolPolicy},
        platform::OperatingSystem,
    };

    use super::{
        IssueKind, IssueSeverity, Status, TOOL_SPECS, ToolStatus, detect_path_conflicts,
        manager_matches_path, normalize_version, should_inspect_tool, suggested_install,
        version_satisfies,
    };

    #[test]
    fn normalizes_common_version_outputs() {
        assert_eq!(normalize_version("v24.15.0", Some("v")), "24.15.0");
        assert_eq!(normalize_version("fnm 1.39.0", Some("fnm ")), "1.39.0");
        assert_eq!(
            normalize_version("go version go1.26.2 darwin/arm64", Some("go version go")),
            "1.26.2"
        );
        assert_eq!(
            normalize_version("rustc 1.95.0 (59807616e 2026-04-14)", Some("rustc ")),
            "1.95.0"
        );
        assert_eq!(
            normalize_version("Poetry (version 2.3.1)", Some("Poetry (version ")),
            "2.3.1"
        );
    }

    #[test]
    fn supports_policy_version_matching() {
        assert!(version_satisfies("24.15.0", "24.x"));
        assert!(version_satisfies("1.26.2", "1.26.2"));
        assert!(version_satisfies("11.13.0", "latest"));
        assert!(version_satisfies("1.95.0", "stable"));
        assert!(!version_satisfies("25.9.0", "24.x"));
        assert!(!version_satisfies("1.26.1", "1.26.2"));
    }

    #[test]
    fn matches_nvm_managed_node_paths() {
        assert!(manager_matches_path(
            Some("nvm"),
            std::path::Path::new("/Users/dev/.nvm/versions/node/v24.0.0/bin/node")
        ));
        assert!(!manager_matches_path(
            Some("nvm"),
            std::path::Path::new("/opt/homebrew/bin/node")
        ));
    }

    #[test]
    fn skips_missing_unconfigured_tools() {
        let deno = TOOL_SPECS.iter().find(|spec| spec.name == "deno").unwrap();
        let config = DevkitConfig::default();

        assert!(!should_inspect_tool(
            deno,
            &config,
            OperatingSystem::Macos,
            |_| false
        ));
    }

    #[test]
    fn inspects_configured_or_installed_tools() {
        let deno = TOOL_SPECS.iter().find(|spec| spec.name == "deno").unwrap();
        let config = DevkitConfig {
            tools: Some(
                std::iter::once((
                    "deno".to_string(),
                    ToolPolicy {
                        version: Some("latest".to_string()),
                        manager: Some("brew".to_string()),
                        ..ToolPolicy::default()
                    },
                ))
                .collect(),
            ),
            ..DevkitConfig::default()
        };

        assert!(should_inspect_tool(
            deno,
            &config,
            OperatingSystem::Macos,
            |_| false
        ));

        let config = DevkitConfig::default();
        assert!(should_inspect_tool(
            deno,
            &config,
            OperatingSystem::Macos,
            |command| command == "deno"
        ));
    }

    #[test]
    fn suggests_platform_specific_install_commands() {
        assert_eq!(
            suggested_install(
                &missing_tool("deno", Some("standalone")),
                OperatingSystem::Linux
            )
            .as_deref(),
            Some("curl -fsSL https://deno.land/install.sh | sh")
        );
        assert_eq!(
            suggested_install(
                &missing_tool("deno", Some("winget")),
                OperatingSystem::Windows
            )
            .as_deref(),
            Some("winget install --exact --id DenoLand.Deno")
        );
        assert_eq!(
            suggested_install(&missing_tool("python", None), OperatingSystem::Windows).as_deref(),
            Some("uv python install")
        );
    }

    #[test]
    fn reports_path_conflict_from_candidate_count() {
        let tool = ToolStatus {
            name: "node".to_string(),
            command: "node".to_string(),
            path: Some(PathBuf::from("/a/node")),
            path_candidates: vec![PathBuf::from("/a/node"), PathBuf::from("/b/node")],
            current: Some("24.0.0".to_string()),
            required: None,
            manager: None,
            status: Status::Ok,
            note: None,
        };
        let mut issues = Vec::new();

        detect_path_conflicts(&mut issues, &[tool]);

        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0].kind, IssueKind::PathConflict));
        assert_eq!(issues[0].severity, IssueSeverity::Warning);
        assert!(issues[0].message.contains("first of 2 PATH candidates"));
        assert!(
            issues[0]
                .evidence
                .iter()
                .any(|evidence| { evidence.key == "active_path" && evidence.value == "/a/node" })
        );
        assert!(
            issues[0]
                .fix
                .as_deref()
                .is_some_and(|fix| fix.contains("/a/node; /b/node"))
        );
    }

    fn missing_tool(name: &str, manager: Option<&str>) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            command: name.to_string(),
            path: None,
            path_candidates: Vec::new(),
            current: None,
            required: Some("latest".to_string()),
            manager: manager.map(ToString::to_string),
            status: Status::Missing,
            note: Some("command not found in PATH".to_string()),
        }
    }
}
