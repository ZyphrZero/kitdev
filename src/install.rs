use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::{
    config::{DevkitConfig, ToolPolicy},
    doctor::{DoctorReport, Status, ToolStatus, build_doctor_report, version_satisfies},
    shell::{command_path, run_shell_command},
    sync::{SyncStep, SyncStepExecution, SyncStepExecutionStatus, SyncStepKind},
    tool_command::{ToolCommand, command_for_package, command_for_tool, homebrew_install_command},
};

#[derive(Debug, Serialize)]
pub struct InstallPlan {
    pub dry_run: bool,
    pub tool: String,
    pub check_command: String,
    pub effective_policy: ToolPolicy,
    pub steps: Vec<SyncStep>,
}

#[derive(Debug, Serialize)]
pub struct InstallExecution {
    pub tool: String,
    pub succeeded: bool,
    pub steps: Vec<SyncStepExecution>,
    pub status: InstallStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallStatus {
    KnownTool(ToolStatus),
    CommandVisible { command: String, path: PathBuf },
    CommandNotVisible { command: String },
}

const KNOWN_TOOLS: &[&str] = &[
    "brew", "fnm", "node", "npm", "pnpm", "yarn", "bun", "deno", "wrangler", "uv", "python",
    "poetry", "ruby", "rust", "rustup", "rustc", "cargo", "go",
];

pub fn build_install_plan(
    dry_run: bool,
    tool: &str,
    manager: Option<&str>,
    version: Option<&str>,
    config: &DevkitConfig,
    report: &DoctorReport,
) -> Result<InstallPlan> {
    let request = normalize_request(tool);
    let known_tool = canonical_known_tool(&request);
    let tool = known_tool.unwrap_or_else(|| request.clone());
    let mut policy = if is_known_tool(&tool) {
        policy_for_tool(&tool, config)
    } else {
        package_policy_for_tool(&request, config)
    };

    if !is_known_tool(&tool)
        && manager.is_none()
        && policy.manager.is_none()
        && policy.source.is_none()
    {
        bail!(
            "unsupported tool `{}`; supported tools: {}; for CLI packages use --manager brew or --manager npm, or list it under [tools.cli], [homebrew], or [npm]",
            tool,
            KNOWN_TOOLS.join(", ")
        );
    }

    let check_command = check_command_for(&tool).to_string();
    if policy.manager.is_none() {
        policy.manager = policy.source.clone();
    }
    if let Some(manager) = manager {
        policy.manager = Some(normalize_manager(manager));
        policy.source = policy.manager.clone();
    }
    if let Some(version) = version {
        policy.version = Some(version.to_string());
    }

    let mut steps = Vec::new();
    let mut seen = BTreeSet::new();
    add_prerequisite_steps(&tool, &policy, report, &mut steps, &mut seen)?;
    add_install_step(&tool, &policy, report, &mut steps, &mut seen)?;

    if steps.is_empty() {
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: format!("info:{tool}:installed"),
                kind: SyncStepKind::Info,
                target: tool.clone(),
                reason: "already installed and visible in PATH".to_string(),
                command: None,
                file: None,
                snippet: None,
                manual: false,
                requires_sudo: false,
            },
        );
    }

    Ok(InstallPlan {
        dry_run,
        tool,
        check_command,
        effective_policy: policy,
        steps,
    })
}

pub fn execute_install_plan(plan: &InstallPlan, _config: &DevkitConfig) -> InstallExecution {
    let mut steps = Vec::new();
    let mut succeeded = true;

    for step in &plan.steps {
        let result = match step.kind {
            SyncStepKind::Install | SyncStepKind::Align if step.manual => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail: "manual instruction not applied by install".to_string(),
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            }),
            SyncStepKind::Install | SyncStepKind::Align => execute_command_step(step),
            SyncStepKind::Info => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Unchanged,
                detail: step.reason.clone(),
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            }),
            SyncStepKind::Configure | SyncStepKind::Cleanup | SyncStepKind::Verify => {
                Ok(SyncStepExecution {
                    id: step.id.clone(),
                    kind: step.kind,
                    target: step.target.clone(),
                    status: SyncStepExecutionStatus::Skipped,
                    detail: "install command does not apply this step type".to_string(),
                    command: step.command.clone(),
                    file: step.file.clone(),
                    manual: step.manual,
                })
            }
        };

        match result {
            Ok(result) => steps.push(result),
            Err(error) => {
                succeeded = false;
                steps.push(SyncStepExecution {
                    id: step.id.clone(),
                    kind: step.kind,
                    target: step.target.clone(),
                    status: SyncStepExecutionStatus::Failed,
                    detail: error.to_string(),
                    command: step.command.clone(),
                    file: step.file.clone(),
                    manual: step.manual,
                });
                break;
            }
        }
    }

    InstallExecution {
        tool: plan.tool.clone(),
        succeeded,
        steps,
        status: detect_install_status(plan),
    }
}

fn add_prerequisite_steps(
    tool: &str,
    policy: &ToolPolicy,
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    match tool {
        "fnm" | "bun" => add_brew_prerequisite(report, steps, seen),
        "node" => {
            if manager_is(policy, "fnm") {
                add_prerequisite_steps("fnm", &default_policy_for("fnm"), report, steps, seen)?;
                add_install_step("fnm", &default_policy_for("fnm"), report, steps, seen)?;
            } else if manager_is(policy, "brew") {
                add_brew_prerequisite(report, steps, seen);
            }
        }
        "npm" | "pnpm" | "yarn" | "wrangler" if command_path("node").is_none() => {
            add_prerequisite_steps("node", &default_policy_for("node"), report, steps, seen)?;
            add_install_step("node", &default_policy_for("node"), report, steps, seen)?;
        }
        "deno" | "python" | "poetry" | "ruby" if manager_is(policy, "brew") => {
            add_brew_prerequisite(report, steps, seen);
        }
        "poetry" if manager_is(policy, "official") && command_path("python3").is_none() => {
            add_prerequisite_steps("python", &default_policy_for("python"), report, steps, seen)?;
            add_install_step("python", &default_policy_for("python"), report, steps, seen)?;
        }
        "python" if manager_is(policy, "uv") && command_path("uv").is_none() => {
            add_install_step("uv", &default_policy_for("uv"), report, steps, seen)?;
        }
        "go" if manager_is(policy, "brew") => {
            add_brew_prerequisite(report, steps, seen);
        }
        _ if !is_known_tool(tool) => match policy.manager.as_deref() {
            Some("brew" | "homebrew") => add_brew_prerequisite(report, steps, seen),
            Some("npm") if command_path("npm").is_none() => {
                add_prerequisite_steps("node", &default_policy_for("node"), report, steps, seen)?;
                add_install_step("node", &default_policy_for("node"), report, steps, seen)?;
            }
            _ => {}
        },
        _ => {}
    }

    Ok(())
}

fn add_brew_prerequisite(
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) {
    if !tool_is_missing(report, "brew") {
        return;
    }

    push_step(
        steps,
        seen,
        SyncStep {
            id: "tool:brew".to_string(),
            kind: SyncStepKind::Install,
            target: "Homebrew".to_string(),
            reason: "install the package manager required for this tool".to_string(),
            command: Some(homebrew_install_command()),
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        },
    );
}

fn add_install_step(
    tool: &str,
    policy: &ToolPolicy,
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    if !is_known_tool(tool) {
        return add_package_install_step(tool, policy, steps, seen);
    }

    if tool == "rust" {
        return add_rust_install_step(policy, report, steps, seen);
    }

    let missing = tool_is_missing(report, tool);
    let manager_mismatch = tool_status(report, tool).is_some_and(|tool_status| {
        policy
            .manager
            .as_deref()
            .is_some_and(|manager| !manager_matches_tool_path(tool, manager, tool_status))
    });
    let install = missing || manager_mismatch;
    let needs_install = tool_needs_install(report, tool, policy);
    if !needs_install {
        return Ok(());
    }

    let command = match command_for_tool(tool, policy, install)? {
        ToolCommand::Shell(command) => command,
        ToolCommand::Manual(instruction) => {
            bail!("`devkit install {tool}` does not automate this install path yet; {instruction}")
        }
    };
    push_step(
        steps,
        seen,
        SyncStep {
            id: format!("tool:{tool}"),
            kind: if install {
                SyncStepKind::Install
            } else {
                SyncStepKind::Align
            },
            target: target_name(tool),
            reason: install_reason(tool, missing, manager_mismatch),
            command: Some(command),
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        },
    );

    Ok(())
}

fn add_rust_install_step(
    policy: &ToolPolicy,
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    let missing = tool_is_missing(report, "rustup")
        || tool_is_missing(report, "rustc")
        || tool_is_missing(report, "cargo");
    let needs_install = missing
        || tool_needs_install(report, "rustup", policy)
        || tool_needs_install(report, "rustc", policy)
        || tool_needs_install(report, "cargo", policy);

    if !needs_install {
        return Ok(());
    }

    let channel = policy
        .channel
        .clone()
        .or_else(|| policy.version.clone())
        .unwrap_or_else(|| "stable".to_string());
    let mut command = if missing {
        "curl https://sh.rustup.rs -sSf | sh".to_string()
    } else {
        format!("rustup self update && rustup update {channel}")
    };
    if let Some(components) = &policy.components
        && !components.is_empty()
    {
        command.push_str(" && rustup component add ");
        command.push_str(&components.join(" "));
    }

    push_step(
        steps,
        seen,
        SyncStep {
            id: "tool:rust".to_string(),
            kind: if missing {
                SyncStepKind::Install
            } else {
                SyncStepKind::Align
            },
            target: "Rust toolchain".to_string(),
            reason: install_reason("rust", missing, false),
            command: Some(command),
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        },
    );

    Ok(())
}

fn add_package_install_step(
    package: &str,
    policy: &ToolPolicy,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    if command_path(package).is_some() {
        return Ok(());
    }

    let manager = policy.manager.as_deref().unwrap_or_default();
    let command = match command_for_package(package, manager)? {
        ToolCommand::Shell(command) => command,
        ToolCommand::Manual(instruction) => instruction,
    };

    push_step(
        steps,
        seen,
        SyncStep {
            id: format!("package:{manager}:{package}"),
            kind: SyncStepKind::Install,
            target: package.to_string(),
            reason: format!("install CLI package managed by {manager}"),
            command: Some(command),
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        },
    );

    Ok(())
}

fn execute_command_step(step: &SyncStep) -> Result<SyncStepExecution> {
    let Some(command) = &step.command else {
        bail!("install step `{}` has no command", step.id);
    };
    let output = run_shell_command(command)?;
    let detail = if !output.stdout.is_empty() {
        summarize_output("command completed", &output.stdout)
    } else if !output.stderr.is_empty() {
        summarize_output("command completed", &output.stderr)
    } else {
        "command completed".to_string()
    };

    Ok(SyncStepExecution {
        id: step.id.clone(),
        kind: step.kind,
        target: step.target.clone(),
        status: SyncStepExecutionStatus::Applied,
        detail,
        command: step.command.clone(),
        file: step.file.clone(),
        manual: step.manual,
    })
}

fn detect_install_status(plan: &InstallPlan) -> InstallStatus {
    if is_known_tool(&plan.tool) {
        let config = config_for_install_status(plan);
        let report = build_doctor_report(&config);
        if let Some(status) = report
            .tools
            .into_iter()
            .find(|tool| tool.name == plan.check_command)
        {
            return InstallStatus::KnownTool(status);
        }
    }

    match command_path(&plan.check_command) {
        Some(path) => InstallStatus::CommandVisible {
            command: plan.check_command.clone(),
            path,
        },
        None => InstallStatus::CommandNotVisible {
            command: plan.check_command.clone(),
        },
    }
}

fn config_for_install_status(plan: &InstallPlan) -> DevkitConfig {
    let mut tools = BTreeMap::new();
    tools.insert(plan.check_command.clone(), plan.effective_policy.clone());
    DevkitConfig {
        tools: Some(tools),
        ..DevkitConfig::default()
    }
}

fn policy_for_tool(tool: &str, config: &DevkitConfig) -> ToolPolicy {
    config
        .tools
        .as_ref()
        .and_then(|tools| tools.get(tool))
        .or_else(|| {
            if tool == "rust" {
                config
                    .tools
                    .as_ref()
                    .and_then(|tools| tools.get("rustup"))
                    .or_else(|| config.tools.as_ref().and_then(|tools| tools.get("rustc")))
                    .or_else(|| config.tools.as_ref().and_then(|tools| tools.get("cargo")))
            } else {
                None
            }
        })
        .map(copy_policy)
        .unwrap_or_else(|| default_policy_for(tool))
}

fn package_policy_for_tool(tool: &str, config: &DevkitConfig) -> ToolPolicy {
    if let Some(policy) = config.tools.as_ref().and_then(|tools| tools.get(tool)) {
        return copy_policy(policy);
    }

    if let Some(policy) = config.tools.as_ref().and_then(|tools| tools.get("cli"))
        && policy
            .packages
            .as_ref()
            .is_some_and(|packages| packages.iter().any(|package| package == tool))
    {
        let manager = policy.manager.clone().unwrap_or_else(|| "brew".to_string());
        return ToolPolicy {
            manager: Some(manager.clone()),
            source: policy.source.clone().or(Some(manager)),
            ..ToolPolicy::default()
        };
    }

    if config
        .homebrew
        .as_ref()
        .and_then(|homebrew| homebrew.packages.as_ref())
        .is_some_and(|packages| packages.iter().any(|package| package == tool))
    {
        return ToolPolicy {
            manager: Some("brew".to_string()),
            source: Some("brew".to_string()),
            ..ToolPolicy::default()
        };
    }

    if config
        .npm
        .as_ref()
        .and_then(|npm| npm.global_packages.as_ref())
        .is_some_and(|packages| packages.iter().any(|package| package == tool))
    {
        return ToolPolicy {
            manager: Some("npm".to_string()),
            source: Some("npm".to_string()),
            ..ToolPolicy::default()
        };
    }

    ToolPolicy::default()
}

fn default_policy_for(tool: &str) -> ToolPolicy {
    match tool {
        "fnm" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "node" => ToolPolicy {
            version: Some("stable".to_string()),
            manager: Some("fnm".to_string()),
            ..ToolPolicy::default()
        },
        "npm" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("npm".to_string()),
            ..ToolPolicy::default()
        },
        "pnpm" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("corepack".to_string()),
            ..ToolPolicy::default()
        },
        "yarn" => ToolPolicy {
            version: Some("stable".to_string()),
            manager: Some("corepack".to_string()),
            ..ToolPolicy::default()
        },
        "bun" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "deno" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "wrangler" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("npm".to_string()),
            ..ToolPolicy::default()
        },
        "uv" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("standalone".to_string()),
            ..ToolPolicy::default()
        },
        "python" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "poetry" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "ruby" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        "rust" | "rustup" | "rustc" | "cargo" => ToolPolicy {
            version: Some("stable".to_string()),
            manager: Some("rustup".to_string()),
            channel: Some("stable".to_string()),
            ..ToolPolicy::default()
        },
        "go" => ToolPolicy {
            version: Some("stable".to_string()),
            manager: Some("brew".to_string()),
            source: Some("brew".to_string()),
            ..ToolPolicy::default()
        },
        _ => ToolPolicy::default(),
    }
}

fn copy_policy(policy: &ToolPolicy) -> ToolPolicy {
    ToolPolicy {
        version: policy.version.clone(),
        manager: policy.manager.clone(),
        source: policy.source.clone(),
        channel: policy.channel.clone(),
        components: policy.components.clone(),
        package_managers: policy.package_managers.clone(),
        packages: policy.packages.clone(),
        install_dir: policy.install_dir.clone(),
    }
}

fn tool_needs_install(report: &DoctorReport, name: &str, policy: &ToolPolicy) -> bool {
    let Some(tool) = tool_status(report, name) else {
        return true;
    };
    if !matches!(tool.status, Status::Ok) {
        return true;
    }
    if let (Some(current), Some(required)) = (&tool.current, &policy.version)
        && !version_satisfies(current, required)
    {
        return true;
    }
    if let Some(manager) = policy.manager.as_deref()
        && !manager_matches_tool_path(name, manager, tool)
    {
        return true;
    }

    false
}

fn tool_is_missing(report: &DoctorReport, name: &str) -> bool {
    tool_status(report, name).is_none_or(|tool| matches!(tool.status, Status::Missing))
}

fn tool_status<'a>(report: &'a DoctorReport, name: &str) -> Option<&'a ToolStatus> {
    report.tools.iter().find(|tool| tool.name == name)
}

fn manager_matches_tool_path(name: &str, manager: &str, tool: &ToolStatus) -> bool {
    let Some(path) = &tool.path else {
        return false;
    };
    let path = path.to_string_lossy();
    match (name, manager) {
        (_, "brew" | "homebrew") => path.contains("/opt/homebrew/") || path.contains("/usr/local/"),
        ("node", "fnm") => path.contains("fnm"),
        ("python", "uv") => false,
        _ => true,
    }
}

fn install_reason(tool: &str, missing: bool, manager_mismatch: bool) -> String {
    if missing {
        format!("install {tool} because it is not visible in PATH")
    } else if manager_mismatch {
        format!("install {tool} with the configured manager")
    } else {
        format!("align {tool} with the configured install policy")
    }
}

fn target_name(tool: &str) -> String {
    match tool {
        "brew" => "Homebrew".to_string(),
        "rust" => "Rust toolchain".to_string(),
        _ => tool.to_string(),
    }
}

fn check_command_for(tool: &str) -> &str {
    match tool {
        "rust" | "rustup" | "cargo" => "rustc",
        _ => tool,
    }
}

fn canonical_known_tool(tool: &str) -> Option<String> {
    match tool {
        "rust" | "rustup" | "rustc" | "cargo" => Some("rust".to_string()),
        _ if is_known_tool(tool) => Some(tool.to_string()),
        _ => None,
    }
}

fn is_known_tool(tool: &str) -> bool {
    KNOWN_TOOLS.contains(&tool)
}

fn manager_is(policy: &ToolPolicy, expected: &str) -> bool {
    let manager = policy.manager.as_deref().unwrap_or_default();
    match expected {
        "brew" => matches!(manager, "" | "brew" | "homebrew"),
        "fnm" => matches!(manager, "" | "fnm"),
        "npm" => matches!(manager, "" | "npm"),
        _ => manager == expected,
    }
}

fn normalize_request(tool: &str) -> String {
    tool.trim().to_ascii_lowercase()
}

fn normalize_manager(manager: &str) -> String {
    match manager.trim().to_ascii_lowercase().as_str() {
        "homebrew" => "brew".to_string(),
        other => other.to_string(),
    }
}

fn push_step(steps: &mut Vec<SyncStep>, seen: &mut BTreeSet<String>, step: SyncStep) {
    if seen.insert(step.id.clone()) {
        steps.push(step);
    }
}

fn summarize_output(prefix: &str, output: &str) -> String {
    let first_line = output.lines().find(|line| !line.trim().is_empty());
    match first_line {
        Some(line) => format!("{prefix}: {}", line.trim()),
        None => prefix.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{
        config::{DevkitConfig, HomebrewConfig, NpmConfig, ToolPolicy},
        doctor::{DoctorReport, Status, ToolStatus},
    };

    use super::build_install_plan;

    #[test]
    fn builds_direct_bun_install_with_brew_prerequisite() {
        let report = DoctorReport {
            tools: vec![missing_tool("brew"), missing_tool("bun")],
            issues: vec![],
        };
        let plan =
            build_install_plan(true, "bun", None, None, &DevkitConfig::default(), &report).unwrap();

        let commands = plan
            .steps
            .iter()
            .filter_map(|step| step.command.as_deref())
            .collect::<Vec<_>>();

        assert_eq!(plan.tool, "bun");
        assert!(
            commands
                .iter()
                .any(|command| command.contains("Homebrew/install"))
        );
        assert!(commands.contains(&"brew install oven-sh/bun/bun"));
    }

    #[test]
    fn installed_tool_becomes_info_step() {
        let report = DoctorReport {
            tools: vec![installed_tool("brew"), installed_tool("bun")],
            issues: vec![],
        };
        let plan =
            build_install_plan(true, "bun", None, None, &DevkitConfig::default(), &report).unwrap();

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].id, "info:bun:installed");
    }

    #[test]
    fn rejects_unknown_tool_without_manager() {
        let report = DoctorReport {
            tools: vec![],
            issues: vec![],
        };
        let error = build_install_plan(true, "gh", None, None, &DevkitConfig::default(), &report)
            .unwrap_err();

        assert!(error.to_string().contains("unsupported tool `gh`"));
    }

    #[test]
    fn supports_explicit_brew_package_install() {
        let report = DoctorReport {
            tools: vec![installed_tool("brew")],
            issues: vec![],
        };
        let plan = build_install_plan(
            true,
            "devkit-test-cli-package",
            Some("brew"),
            None,
            &DevkitConfig::default(),
            &report,
        )
        .unwrap();

        assert_eq!(
            plan.steps[0].command.as_deref(),
            Some("brew install devkit-test-cli-package")
        );
    }

    #[test]
    fn infers_cli_package_manager_from_tools_cli_config() {
        let report = DoctorReport {
            tools: vec![installed_tool("brew")],
            issues: vec![],
        };
        let config = DevkitConfig {
            tools: Some(
                [(
                    "cli".to_string(),
                    ToolPolicy {
                        manager: Some("brew".to_string()),
                        packages: Some(vec!["devkit-configured-cli-package".to_string()]),
                        ..ToolPolicy::default()
                    },
                )]
                .into_iter()
                .collect(),
            ),
            ..DevkitConfig::default()
        };
        let plan = build_install_plan(
            true,
            "devkit-configured-cli-package",
            None,
            None,
            &config,
            &report,
        )
        .unwrap();

        assert_eq!(
            plan.steps[0].command.as_deref(),
            Some("brew install devkit-configured-cli-package")
        );
        assert_eq!(plan.effective_policy.manager.as_deref(), Some("brew"));
    }

    #[test]
    fn infers_package_manager_from_homebrew_and_npm_config() {
        let report = DoctorReport {
            tools: vec![installed_tool("brew"), installed_tool("npm")],
            issues: vec![],
        };
        let config = DevkitConfig {
            homebrew: Some(HomebrewConfig {
                mirror: None,
                packages: Some(vec!["devkit-homebrew-package".to_string()]),
            }),
            npm: Some(NpmConfig {
                global_packages: Some(vec!["devkit-npm-package".to_string()]),
            }),
            ..DevkitConfig::default()
        };
        let brew_plan = build_install_plan(
            true,
            "devkit-homebrew-package",
            None,
            None,
            &config,
            &report,
        )
        .unwrap();
        let npm_plan =
            build_install_plan(true, "devkit-npm-package", None, None, &config, &report).unwrap();

        assert_eq!(
            brew_plan.steps[0].command.as_deref(),
            Some("brew install devkit-homebrew-package")
        );
        assert_eq!(
            npm_plan.steps[0].command.as_deref(),
            Some("npm install -g devkit-npm-package@latest")
        );
    }

    #[test]
    fn supports_deno_brew_install() {
        let report = DoctorReport {
            tools: vec![installed_tool("brew"), missing_tool("deno")],
            issues: vec![],
        };
        let plan = build_install_plan(true, "deno", None, None, &DevkitConfig::default(), &report)
            .unwrap();

        assert_eq!(plan.steps[0].command.as_deref(), Some("brew install deno"));
    }

    #[test]
    fn supports_yarn_corepack_install() {
        let report = DoctorReport {
            tools: vec![installed_tool("node"), missing_tool("yarn")],
            issues: vec![],
        };
        let plan = build_install_plan(true, "yarn", None, None, &DevkitConfig::default(), &report)
            .unwrap();

        assert_eq!(
            plan.steps[0].command.as_deref(),
            Some("corepack enable && corepack prepare yarn@stable --activate")
        );
    }

    #[test]
    fn supports_python_uv_install() {
        let report = DoctorReport {
            tools: vec![installed_tool("uv"), missing_tool("python")],
            issues: vec![],
        };
        let plan = build_install_plan(
            true,
            "python",
            Some("uv"),
            Some("3.13"),
            &DevkitConfig::default(),
            &report,
        )
        .unwrap();

        assert_eq!(
            plan.steps[0].command.as_deref(),
            Some("uv python install 3.13")
        );
    }

    #[test]
    fn direct_official_go_install_is_not_automated() {
        let report = DoctorReport {
            tools: vec![missing_tool("go")],
            issues: vec![],
        };
        let config = DevkitConfig {
            tools: Some(
                [(
                    "go".to_string(),
                    ToolPolicy {
                        version: Some("1.26.2".to_string()),
                        manager: Some("official".to_string()),
                        source: Some("official".to_string()),
                        install_dir: Some("~/.local/opt/go/current".to_string()),
                        ..ToolPolicy::default()
                    },
                )]
                .into_iter()
                .collect(),
            ),
            ..DevkitConfig::default()
        };
        let error = build_install_plan(true, "go", None, None, &config, &report).unwrap_err();

        assert!(error.to_string().contains("does not automate"));
        assert!(error.to_string().contains("install Go 1.26.2"));
    }

    #[test]
    fn config_policy_can_select_node_version() {
        let report = DoctorReport {
            tools: vec![
                installed_tool("brew"),
                installed_tool("fnm"),
                missing_tool("node"),
            ],
            issues: vec![],
        };
        let config = DevkitConfig {
            tools: Some(
                [(
                    "node".to_string(),
                    ToolPolicy {
                        version: Some("24.x".to_string()),
                        manager: Some("fnm".to_string()),
                        ..ToolPolicy::default()
                    },
                )]
                .into_iter()
                .collect(),
            ),
            ..DevkitConfig::default()
        };
        let plan = build_install_plan(true, "node", None, None, &config, &report).unwrap();

        assert_eq!(
            plan.steps[0].command.as_deref(),
            Some("fnm install 24 && fnm default 24")
        );
    }

    fn missing_tool(name: &str) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            command: name.to_string(),
            path: None,
            current: None,
            required: None,
            manager: None,
            status: Status::Missing,
            note: Some("command not found in PATH".to_string()),
        }
    }

    fn installed_tool(name: &str) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            command: name.to_string(),
            path: Some(Path::new("/opt/homebrew/bin").join(name)),
            current: Some("1.0.0".to_string()),
            required: None,
            manager: None,
            status: Status::Ok,
            note: None,
        }
    }
}
