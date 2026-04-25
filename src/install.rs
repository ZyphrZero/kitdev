use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::{
    config::{DevkitConfig, ToolPolicy},
    doctor::{DoctorReport, Status, ToolStatus, build_doctor_report, version_satisfies},
    package_manager::detected_cli_manager,
    platform::{OperatingSystem, home_path, path_contains},
    shell::{command_path, run_shell_command},
    sync::{SyncStep, SyncStepExecution, SyncStepExecutionStatus, SyncStepKind},
    tool_command::{
        ToolCommand, command_for_package, command_for_tool, homebrew_install_command,
        rustup_install_command,
    },
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
    "brew", "winget", "fnm", "nvm", "node", "npm", "pnpm", "yarn", "bun", "deno", "wrangler", "uv",
    "python", "poetry", "ruby", "rust", "rustup", "rustc", "cargo", "go",
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
            "unsupported tool `{}`; supported tools: {}; for CLI packages use --manager brew, npm, apt, dnf, pacman, zypper, apk, winget, scoop, choco, or list it under [tools.cli], [homebrew], or [npm]",
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
                blocked_by: Vec::new(),
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
        let unresolved_blockers = unresolved_blockers(step);
        if !unresolved_blockers.is_empty() {
            succeeded = false;
            steps.push(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail: format!("blocked by {}", unresolved_blockers.join(", ")),
                blocked_by: unresolved_blockers,
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            });
            continue;
        }

        let result = match step.kind {
            SyncStepKind::Install | SyncStepKind::Align if step.manual => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail: "manual instruction not applied by install".to_string(),
                blocked_by: step.blocked_by.clone(),
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
                blocked_by: step.blocked_by.clone(),
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
                    blocked_by: step.blocked_by.clone(),
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
                    blocked_by: step.blocked_by.clone(),
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
        "fnm" | "bun" => add_platform_manager_prerequisite(tool, policy, report, steps, seen),
        "nvm" => add_nvm_prerequisite(steps, seen),
        "node" => {
            if manager_is_for("node", policy, "fnm") {
                add_prerequisite_steps("fnm", &default_policy_for("fnm"), report, steps, seen)?;
                add_install_step("fnm", &default_policy_for("fnm"), report, steps, seen)?;
            } else if manager_is_for("node", policy, "nvm") {
                add_nvm_prerequisite(steps, seen);
            } else {
                add_platform_manager_prerequisite("node", policy, report, steps, seen);
            }
        }
        "npm" | "pnpm" | "yarn" | "wrangler" if command_path("node").is_none() => {
            add_prerequisite_steps("node", &default_policy_for("node"), report, steps, seen)?;
            add_install_step("node", &default_policy_for("node"), report, steps, seen)?;
        }
        "deno" | "uv" | "python" | "poetry" | "ruby" => {
            add_platform_manager_prerequisite(tool, policy, report, steps, seen);
            if tool == "poetry"
                && manager_is_for(tool, policy, "official")
                && command_path("python3").is_none()
                && command_path("python").is_none()
                && command_path("py").is_none()
            {
                add_prerequisite_steps(
                    "python",
                    &default_policy_for("python"),
                    report,
                    steps,
                    seen,
                )?;
                add_install_step("python", &default_policy_for("python"), report, steps, seen)?;
            } else if tool == "python"
                && manager_is_for(tool, policy, "uv")
                && command_path("uv").is_none()
            {
                add_install_step("uv", &default_policy_for("uv"), report, steps, seen)?;
            }
        }
        "go" => {
            if source_is_for("go", policy, "brew") {
                add_brew_prerequisite(report, steps, seen);
            } else if source_is_for("go", policy, "winget") {
                add_winget_prerequisite(report, steps, seen);
            }
        }
        _ if !is_known_tool(tool) => match policy.manager.as_deref() {
            Some("brew" | "homebrew") => add_brew_prerequisite(report, steps, seen),
            Some("winget") => add_winget_prerequisite(report, steps, seen),
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

fn add_platform_manager_prerequisite(
    tool: &str,
    policy: &ToolPolicy,
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) {
    if manager_is_for(tool, policy, "brew") {
        add_brew_prerequisite(report, steps, seen);
    } else if manager_is_for(tool, policy, "winget") {
        add_winget_prerequisite(report, steps, seen);
    }
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
            blocked_by: Vec::new(),
            command: Some(homebrew_install_command()),
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        },
    );
}

fn add_winget_prerequisite(
    report: &DoctorReport,
    steps: &mut Vec<SyncStep>,
    seen: &mut BTreeSet<String>,
) {
    if !tool_is_missing(report, "winget") {
        return;
    }

    push_step(
        steps,
        seen,
        SyncStep {
            id: "tool:winget".to_string(),
            kind: SyncStepKind::Install,
            target: "Windows Package Manager".to_string(),
            reason: "install the package manager required for this tool".to_string(),
            blocked_by: Vec::new(),
            command: Some(
                "install App Installer from Microsoft Store or https://aka.ms/getwinget"
                    .to_string(),
            ),
            file: None,
            snippet: None,
            manual: true,
            requires_sudo: false,
        },
    );
}

fn add_nvm_prerequisite(steps: &mut Vec<SyncStep>, seen: &mut BTreeSet<String>) {
    if nvm_installed() {
        return;
    }

    let policy = ToolPolicy {
        manager: Some("standalone".to_string()),
        ..ToolPolicy::default()
    };
    let (command, manual) = command_for_tool("nvm", &policy, true)
        .map(ToolCommand::into_step_parts)
        .unwrap_or_else(|error| (Some(error.to_string()), true));

    push_step(
        steps,
        seen,
        SyncStep {
            id: "tool:nvm".to_string(),
            kind: SyncStepKind::Install,
            target: "nvm".to_string(),
            reason: "install nvm required by the configured Node workflow".to_string(),
            blocked_by: Vec::new(),
            command,
            file: None,
            snippet: None,
            manual,
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
        return add_package_install_step(tool, policy, report, steps, seen);
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
    let manual = manager_prerequisite_missing(tool, policy, report);
    let blocked_by = tool_dependency_blockers(tool, policy, report);
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
            blocked_by,
            command: Some(command),
            file: None,
            snippet: None,
            manual,
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
        rustup_install_command(OperatingSystem::current())
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
            blocked_by: Vec::new(),
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
    report: &DoctorReport,
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
    let manual = manager == "winget" && tool_is_missing(report, "winget");
    let blocked_by = package_dependency_blockers(manager);

    push_step(
        steps,
        seen,
        SyncStep {
            id: format!("package:{manager}:{package}"),
            kind: SyncStepKind::Install,
            target: package.to_string(),
            reason: format!("install CLI package managed by {manager}"),
            blocked_by,
            command: Some(command),
            file: None,
            snippet: None,
            manual,
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
        blocked_by: step.blocked_by.clone(),
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
        let manager = policy
            .manager
            .clone()
            .or_else(|| detected_cli_manager().map(ToString::to_string))
            .or_else(|| {
                OperatingSystem::current()
                    .default_manager_for("cli")
                    .map(ToString::to_string)
            });
        return ToolPolicy {
            manager: manager.clone(),
            source: policy.source.clone().or(manager),
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
    let platform = OperatingSystem::current();
    match tool {
        "fnm" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: platform.default_manager_for("fnm").map(ToString::to_string),
            ..ToolPolicy::default()
        },
        "nvm" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: Some("standalone".to_string()),
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
            manager: platform.default_manager_for("bun").map(ToString::to_string),
            ..ToolPolicy::default()
        },
        "deno" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: platform
                .default_manager_for("deno")
                .map(ToString::to_string),
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
            manager: platform
                .default_manager_for("python")
                .map(ToString::to_string),
            ..ToolPolicy::default()
        },
        "poetry" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: platform
                .default_manager_for("poetry")
                .map(ToString::to_string),
            ..ToolPolicy::default()
        },
        "ruby" => ToolPolicy {
            version: Some("latest".to_string()),
            manager: platform
                .default_manager_for("ruby")
                .map(ToString::to_string),
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
            manager: platform.default_manager_for("go").map(ToString::to_string),
            source: Some(platform.default_go_source().to_string()),
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
    match (name, manager) {
        (_, "brew" | "homebrew") => OperatingSystem::current().homebrew_path_matches(path),
        ("node", "fnm") => path_contains(path, &["fnm"]),
        ("node", "nvm") => path_contains(path, &["/.nvm/", "\\nvm\\", "/nvm/"]),
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
        "winget" => "Windows Package Manager".to_string(),
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

fn manager_is_for(tool: &str, policy: &ToolPolicy, expected: &str) -> bool {
    let manager = effective_manager_for(tool, policy).unwrap_or_default();
    match expected {
        "brew" => matches!(manager, "brew" | "homebrew"),
        _ => manager == expected,
    }
}

fn source_is_for(tool: &str, policy: &ToolPolicy, expected: &str) -> bool {
    let source = policy
        .source
        .as_deref()
        .filter(|source| !source.is_empty())
        .or_else(|| effective_manager_for(tool, policy))
        .unwrap_or_default();
    match expected {
        "brew" => matches!(source, "brew" | "homebrew"),
        _ => source == expected,
    }
}

fn manager_prerequisite_missing(tool: &str, policy: &ToolPolicy, report: &DoctorReport) -> bool {
    manager_is_for(tool, policy, "winget") && tool_is_missing(report, "winget")
}

fn tool_dependency_blockers(tool: &str, policy: &ToolPolicy, report: &DoctorReport) -> Vec<String> {
    let mut blockers = Vec::new();
    let Some(manager) = effective_manager_for(tool, policy) else {
        return blockers;
    };

    match manager {
        "brew" | "homebrew" => push_tool_blocker_if_missing(&mut blockers, "brew", report),
        "winget" => push_tool_blocker_if_missing(&mut blockers, "winget", report),
        "fnm" if tool == "node" => push_tool_blocker_if_missing(&mut blockers, "fnm", report),
        "nvm" if tool == "node" && !nvm_installed() => push_blocker(&mut blockers, "tool:nvm"),
        "npm" if tool != "npm" => push_tool_blocker_if_missing(&mut blockers, "npm", report),
        "corepack" => push_tool_blocker_if_missing(&mut blockers, "node", report),
        "uv" if tool == "python" => push_tool_blocker_if_missing(&mut blockers, "uv", report),
        "official" if tool == "poetry" => {
            push_tool_blocker_if_missing(&mut blockers, "python", report)
        }
        "rustup" if matches!(tool, "rustc" | "cargo") => {
            push_tool_blocker_if_missing(&mut blockers, "rustup", report)
        }
        _ => {}
    }

    blockers
}

fn package_dependency_blockers(manager: &str) -> Vec<String> {
    let mut blockers = Vec::new();
    match canonical_manager(manager).as_str() {
        "brew" if command_path("brew").is_none() => push_blocker(&mut blockers, "tool:brew"),
        "npm" if command_path("npm").is_none() => push_blocker(&mut blockers, "tool:npm"),
        "winget" if command_path("winget").is_none() => push_blocker(&mut blockers, "tool:winget"),
        "apt" if command_path("apt-get").is_none() => push_blocker(&mut blockers, "manager:apt"),
        "dnf" if command_path("dnf").is_none() => push_blocker(&mut blockers, "manager:dnf"),
        "pacman" if command_path("pacman").is_none() => {
            push_blocker(&mut blockers, "manager:pacman")
        }
        "zypper" if command_path("zypper").is_none() => {
            push_blocker(&mut blockers, "manager:zypper")
        }
        "apk" if command_path("apk").is_none() => push_blocker(&mut blockers, "manager:apk"),
        "scoop" if command_path("scoop").is_none() => push_blocker(&mut blockers, "manager:scoop"),
        "choco" if command_path("choco").is_none() => push_blocker(&mut blockers, "manager:choco"),
        _ => {}
    }
    blockers
}

fn push_tool_blocker_if_missing(blockers: &mut Vec<String>, tool: &str, report: &DoctorReport) {
    if tool_is_missing(report, tool) {
        push_blocker(blockers, &format!("tool:{tool}"));
    }
}

fn push_blocker(blockers: &mut Vec<String>, blocker: &str) {
    if !blockers.iter().any(|existing| existing == blocker) {
        blockers.push(blocker.to_string());
    }
}

fn unresolved_blockers(step: &SyncStep) -> Vec<String> {
    step.blocked_by
        .iter()
        .filter(|blocker| !blocker_resolved(blocker))
        .cloned()
        .collect()
}

fn blocker_resolved(blocker: &str) -> bool {
    if let Some(tool) = blocker.strip_prefix("tool:") {
        return tool_command_visible(tool);
    }
    if let Some(manager) = blocker.strip_prefix("manager:") {
        return manager_command(manager).is_some_and(|command| command_path(command).is_some());
    }
    true
}

fn tool_command_visible(tool: &str) -> bool {
    match tool {
        "python" => {
            command_path("python3").is_some()
                || command_path("python").is_some()
                || command_path("py").is_some()
        }
        "nvm" => nvm_installed(),
        "rust" => command_path("rustup").is_some(),
        "rustc" | "cargo" => command_path(tool).is_some() || command_path("rustup").is_some(),
        other => command_path(other).is_some(),
    }
}

fn manager_command(manager: &str) -> Option<&'static str> {
    match canonical_manager(manager).as_str() {
        "brew" => Some("brew"),
        "nvm" => Some("nvm"),
        "npm" => Some("npm"),
        "winget" => Some("winget"),
        "apt" => Some("apt-get"),
        "dnf" => Some("dnf"),
        "pacman" => Some("pacman"),
        "zypper" => Some("zypper"),
        "apk" => Some("apk"),
        "scoop" => Some("scoop"),
        "choco" => Some("choco"),
        _ => None,
    }
}

fn nvm_installed() -> bool {
    if command_path("nvm").is_some() {
        return true;
    }

    let nvm_dir = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path(".nvm"));
    nvm_dir.join("nvm.sh").is_file()
}

fn canonical_manager(manager: &str) -> String {
    match manager {
        "homebrew" => "brew".to_string(),
        "apt-get" => "apt".to_string(),
        "chocolatey" => "choco".to_string(),
        other => other.to_string(),
    }
}

fn effective_manager_for<'a>(tool: &str, policy: &'a ToolPolicy) -> Option<&'a str> {
    policy
        .manager
        .as_deref()
        .filter(|manager| !manager.is_empty())
        .or_else(|| policy.source.as_deref().filter(|source| !source.is_empty()))
        .or_else(|| OperatingSystem::current().default_manager_for(tool))
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
    fn marks_winget_install_step_manual_when_winget_is_missing() {
        let report = DoctorReport {
            tools: vec![missing_tool("winget"), missing_tool("deno")],
            issues: vec![],
        };
        let plan = build_install_plan(
            true,
            "deno",
            Some("winget"),
            None,
            &DevkitConfig::default(),
            &report,
        )
        .unwrap();
        let deno_step = plan
            .steps
            .iter()
            .find(|step| step.id == "tool:deno")
            .unwrap();

        assert!(deno_step.manual);
        assert_eq!(deno_step.blocked_by, vec!["tool:winget"]);
        assert_eq!(
            deno_step.command.as_deref(),
            Some("winget install --exact --id DenoLand.Deno")
        );
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

    #[test]
    fn supports_nvm_managed_node_install_plan() {
        let report = DoctorReport {
            tools: vec![missing_tool("node")],
            issues: vec![],
        };
        let plan = build_install_plan(
            true,
            "node",
            Some("nvm"),
            Some("24.x"),
            &DevkitConfig::default(),
            &report,
        )
        .unwrap();
        let node_step = plan
            .steps
            .iter()
            .find(|step| step.id == "tool:node")
            .unwrap();

        assert!(
            node_step
                .command
                .as_deref()
                .is_some_and(|command| command.contains("nvm install 24"))
        );
    }

    fn missing_tool(name: &str) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            command: name.to_string(),
            path: None,
            path_candidates: Vec::new(),
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
            path_candidates: vec![Path::new("/opt/homebrew/bin").join(name)],
            current: Some("1.0.0".to_string()),
            required: None,
            manager: None,
            status: Status::Ok,
            note: None,
        }
    }
}
