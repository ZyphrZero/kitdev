use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    config::{DevkitConfig, ToolPolicy},
    doctor::{DoctorReport, Issue, IssueKind, Status, ToolStatus, build_doctor_report},
    shell::{command_path, run_shell_command},
};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncStepKind {
    Install,
    Align,
    Configure,
    Cleanup,
    Verify,
    Info,
}

#[derive(Debug, Serialize)]
pub struct SyncPlan {
    pub dry_run: bool,
    pub policy_channel: Option<String>,
    pub platform: Option<String>,
    pub auto_fix: bool,
    pub steps: Vec<SyncStep>,
}

#[derive(Debug, Serialize)]
pub struct SyncStep {
    pub id: String,
    pub kind: SyncStepKind,
    pub target: String,
    pub reason: String,
    pub command: Option<String>,
    pub file: Option<PathBuf>,
    pub snippet: Option<String>,
    pub requires_sudo: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct SyncExecution {
    pub applied: bool,
    pub policy_channel: Option<String>,
    pub platform: Option<String>,
    pub auto_fix: bool,
    pub succeeded: bool,
    pub steps: Vec<SyncStepExecution>,
    pub doctor: DoctorReport,
}

#[derive(Debug, Serialize)]
pub struct SyncStepExecution {
    pub id: String,
    pub kind: SyncStepKind,
    pub target: String,
    pub status: SyncStepExecutionStatus,
    pub detail: String,
    pub command: Option<String>,
    pub file: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncStepExecutionStatus {
    Applied,
    Unchanged,
    Skipped,
    Failed,
    Verified,
}

pub fn build_sync_plan(
    dry_run: bool,
    config_path: &Path,
    config: &DevkitConfig,
    report: &DoctorReport,
) -> SyncPlan {
    let tools = report
        .tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool))
        .collect::<BTreeMap<_, _>>();
    let policy_channel = config
        .policy
        .as_ref()
        .and_then(|policy| policy.channel.clone());
    let platform = config
        .policy
        .as_ref()
        .and_then(|policy| policy.platform.clone());
    let auto_fix = config
        .policy
        .as_ref()
        .and_then(|policy| policy.auto_fix)
        .unwrap_or(false);
    let mut seen = BTreeSet::new();
    let mut steps = Vec::new();

    if brew_required(config) && is_tool_missing(tools.get("brew").copied()) {
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "tool:brew".to_string(),
                kind: SyncStepKind::Install,
                target: "Homebrew".to_string(),
                reason: "install the package manager used by the configured toolchain".to_string(),
                command: Some("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".to_string()),
                file: None,
                snippet: None,
                requires_sudo: false,
            },
        );
    }

    if let Some(step) = rust_sync_step(config, &tools) {
        push_step(&mut steps, &mut seen, step);
    }

    if let Some(tool_policies) = config.tools.as_ref() {
        for (name, policy) in tool_policies {
            if is_rust_policy(name) {
                continue;
            }

            if let Some(step) = tool_sync_step(name, policy, &tools) {
                push_step(&mut steps, &mut seen, step);
            }

            if name == "node" {
                for package_manager in policy.package_managers.as_deref().unwrap_or(&[]) {
                    if tool_policies.contains_key(package_manager) {
                        continue;
                    }

                    if let Some(step) = package_manager_step(package_manager, &tools) {
                        push_step(&mut steps, &mut seen, step);
                    }
                }
            }

            if let Some(packages) = policy.packages.as_ref() {
                for package in packages {
                    if tool_policies.contains_key(package) {
                        continue;
                    }

                    if let Some(step) = package_step(package, policy.manager.as_deref()) {
                        push_step(&mut steps, &mut seen, step);
                    }
                }
            }
        }
    }

    if let Some(packages) = config
        .homebrew
        .as_ref()
        .and_then(|homebrew| homebrew.packages.as_ref())
    {
        for package in packages {
            if config
                .tools
                .as_ref()
                .is_some_and(|tools| tools.contains_key(package))
            {
                continue;
            }

            if let Some(step) = package_step(package, Some("brew")) {
                push_step(&mut steps, &mut seen, step);
            }
        }
    }

    if let Some(packages) = config
        .npm
        .as_ref()
        .and_then(|npm| npm.global_packages.as_ref())
    {
        for package in packages {
            if config
                .tools
                .as_ref()
                .is_some_and(|tools| tools.contains_key(package))
            {
                continue;
            }

            if let Some(step) = package_step(package, Some("npm")) {
                push_step(&mut steps, &mut seen, step);
            }
        }
    }

    if let Some(mirror) = homebrew_mirror(config) {
        let brew_missing = is_tool_missing(tools.get("brew").copied());
        let has_brew_packages = config
            .homebrew
            .as_ref()
            .and_then(|homebrew| homebrew.packages.as_ref())
            .is_some_and(|packages| {
                packages
                    .iter()
                    .any(|package| command_path(package).is_none())
            });
        if brew_missing || has_brew_packages {
            let profile_path = shell_profile_path();
            push_step(
                &mut steps,
                &mut seen,
                SyncStep {
                    id: "config:homebrew-mirror".to_string(),
                    kind: SyncStepKind::Configure,
                    target: "Homebrew mirror".to_string(),
                    reason: format!("configure Homebrew mirror to {}", mirror),
                    command: None,
                    file: Some(profile_path),
                    snippet: homebrew_mirror_snippet(&mirror),
                    requires_sudo: false,
                },
            );
        }
    }

    if uses_fnm(config)
        && (tool_needs_sync(tools.get("fnm").copied())
            || tool_needs_sync(tools.get("node").copied()))
    {
        let shell = current_shell_name();
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "config:fnm-shell".to_string(),
                kind: SyncStepKind::Configure,
                target: format!("{shell} shell initialization"),
                reason: "initialize fnm in your shell so managed Node versions are available"
                    .to_string(),
                command: None,
                file: Some(shell_rc_path()),
                snippet: Some(format!("eval \"$(fnm env --use-on-cd --shell {shell})\"")),
                requires_sudo: false,
            },
        );
    }

    if let Some(install_dir) = go_install_dir(config)
        && tool_needs_sync(tools.get("go").copied())
    {
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "config:go-path".to_string(),
                kind: SyncStepKind::Configure,
                target: "Go PATH ordering".to_string(),
                reason: "ensure the configured Go install directory takes precedence in PATH"
                    .to_string(),
                command: None,
                file: Some(shell_rc_path()),
                snippet: Some(format!("export PATH=\"{install_dir}/bin:$PATH\"")),
                requires_sudo: false,
            },
        );
    }

    for issue in &report.issues {
        if let Some(step) = issue_step(issue) {
            push_step(&mut steps, &mut seen, step);
        }
    }

    if steps.is_empty() {
        steps.push(SyncStep {
            id: "info:healthy".to_string(),
            kind: SyncStepKind::Info,
            target: "environment".to_string(),
            reason: "environment already matches the configured bootstrap policy".to_string(),
            command: None,
            file: None,
            snippet: None,
            requires_sudo: false,
        });
    }

    steps.push(SyncStep {
        id: "verify:doctor".to_string(),
        kind: SyncStepKind::Verify,
        target: "environment".to_string(),
        reason: "re-run doctor after applying the planned steps".to_string(),
        command: Some(format!("devkit doctor --config {}", config_path.display())),
        file: None,
        snippet: None,
        requires_sudo: false,
    });

    SyncPlan {
        dry_run,
        policy_channel,
        platform,
        auto_fix,
        steps,
    }
}

pub fn execute_sync_plan(plan: &SyncPlan, config: &DevkitConfig) -> SyncExecution {
    let mut results = Vec::new();
    let mut succeeded = true;
    let mut verify_step = None;

    for step in &plan.steps {
        if matches!(step.kind, SyncStepKind::Verify) {
            verify_step = Some(step);
            continue;
        }

        let execution = match step.kind {
            SyncStepKind::Install | SyncStepKind::Align => execute_command_step(step),
            SyncStepKind::Configure => execute_configure_step(step),
            SyncStepKind::Cleanup => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail:
                    "cleanup steps remain manual; review `devkit cleanup` before deleting files"
                        .to_string(),
                command: step.command.clone(),
                file: step.file.clone(),
            }),
            SyncStepKind::Info => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Unchanged,
                detail: step.reason.clone(),
                command: step.command.clone(),
                file: step.file.clone(),
            }),
            SyncStepKind::Verify => unreachable!(),
        };

        match execution {
            Ok(result) => results.push(result),
            Err(error) => {
                succeeded = false;
                results.push(SyncStepExecution {
                    id: step.id.clone(),
                    kind: step.kind,
                    target: step.target.clone(),
                    status: SyncStepExecutionStatus::Failed,
                    detail: error.to_string(),
                    command: step.command.clone(),
                    file: step.file.clone(),
                });
                break;
            }
        }
    }

    let doctor = build_doctor_report(config);
    let policy_matches = report_matches_policy(&doctor);
    if !policy_matches {
        succeeded = false;
    }

    if let Some(step) = verify_step {
        results.push(build_verify_result(step, &doctor, policy_matches));
    }

    SyncExecution {
        applied: true,
        policy_channel: plan.policy_channel.clone(),
        platform: plan.platform.clone(),
        auto_fix: plan.auto_fix,
        succeeded,
        steps: results,
        doctor,
    }
}

fn execute_command_step(step: &SyncStep) -> Result<SyncStepExecution> {
    let command = step
        .command
        .as_deref()
        .with_context(|| format!("sync step `{}` has no command", step.id))?;
    let output = run_shell_command(command)
        .with_context(|| format!("failed while applying sync step `{}`", step.id))?;
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
    })
}

fn execute_configure_step(step: &SyncStep) -> Result<SyncStepExecution> {
    if let (Some(file), Some(snippet)) = (&step.file, &step.snippet) {
        let changed = ensure_managed_snippet(file, &step.id, snippet)
            .with_context(|| format!("failed while updating {}", file.display()))?;
        return Ok(SyncStepExecution {
            id: step.id.clone(),
            kind: step.kind,
            target: step.target.clone(),
            status: if changed {
                SyncStepExecutionStatus::Applied
            } else {
                SyncStepExecutionStatus::Unchanged
            },
            detail: if changed {
                format!("updated {}", file.display())
            } else {
                format!("{} already contains the managed snippet", file.display())
            },
            command: step.command.clone(),
            file: step.file.clone(),
        });
    }

    if step.command.is_some() {
        return Ok(SyncStepExecution {
            id: step.id.clone(),
            kind: step.kind,
            target: step.target.clone(),
            status: SyncStepExecutionStatus::Skipped,
            detail: "manual configuration step left in plan; review the suggested command"
                .to_string(),
            command: step.command.clone(),
            file: step.file.clone(),
        });
    }

    Ok(SyncStepExecution {
        id: step.id.clone(),
        kind: step.kind,
        target: step.target.clone(),
        status: SyncStepExecutionStatus::Unchanged,
        detail: step.reason.clone(),
        command: step.command.clone(),
        file: step.file.clone(),
    })
}

fn build_verify_result(
    step: &SyncStep,
    doctor: &DoctorReport,
    policy_matches: bool,
) -> SyncStepExecution {
    let blocking_tools = doctor
        .tools
        .iter()
        .filter(|tool| {
            !matches!(tool.status, Status::Ok)
                && (tool.required.is_some() || tool.manager.is_some())
        })
        .count();
    let blocking_issues = doctor
        .issues
        .iter()
        .filter(|issue| !matches!(issue.kind, IssueKind::MissingTool))
        .count();
    let detail = if policy_matches {
        "doctor confirms the environment matches the configured policy".to_string()
    } else {
        format!(
            "doctor still found {} blocking tool state(s) and {} blocking issue(s)",
            blocking_tools, blocking_issues
        )
    };

    SyncStepExecution {
        id: step.id.clone(),
        kind: step.kind,
        target: step.target.clone(),
        status: SyncStepExecutionStatus::Verified,
        detail,
        command: step.command.clone(),
        file: step.file.clone(),
    }
}

fn rust_sync_step(config: &DevkitConfig, tools: &BTreeMap<&str, &ToolStatus>) -> Option<SyncStep> {
    let tool_policies = config.tools.as_ref()?;

    if !["rust", "rustup", "rustc", "cargo"]
        .iter()
        .any(|name| tool_policies.contains_key(*name))
    {
        return None;
    }

    let rustup = tools.get("rustup").copied();
    let rustc = tools.get("rustc").copied();
    let cargo = tools.get("cargo").copied();
    let missing = is_tool_missing(rustup) || is_tool_missing(rustc) || is_tool_missing(cargo);
    let needs_sync =
        missing || tool_needs_sync(rustup) || tool_needs_sync(rustc) || tool_needs_sync(cargo);

    if !needs_sync {
        return None;
    }

    let policy = tool_policies
        .get("rust")
        .or_else(|| tool_policies.get("rustup"))
        .or_else(|| tool_policies.get("rustc"))
        .or_else(|| tool_policies.get("cargo"));
    let channel = policy
        .and_then(|policy| policy.channel.clone().or_else(|| policy.version.clone()))
        .unwrap_or_else(|| "stable".to_string());
    let components = policy
        .and_then(|policy| policy.components.as_ref())
        .cloned()
        .unwrap_or_default();
    let mut command = if is_tool_missing(rustup) {
        "curl https://sh.rustup.rs -sSf | sh".to_string()
    } else {
        format!("rustup self update && rustup update {channel}")
    };
    if !components.is_empty() {
        command.push_str(" && rustup component add ");
        command.push_str(&components.join(" "));
    }

    Some(SyncStep {
        id: "tool:rust".to_string(),
        kind: if missing {
            SyncStepKind::Install
        } else {
            SyncStepKind::Align
        },
        target: "Rust toolchain".to_string(),
        reason: if missing {
            "install the Rust toolchain managed by rustup".to_string()
        } else {
            format!("align Rust with the configured {channel} toolchain")
        },
        command: Some(command),
        file: None,
        snippet: None,
        requires_sudo: false,
    })
}

fn tool_sync_step(
    name: &str,
    policy: &ToolPolicy,
    tools: &BTreeMap<&str, &ToolStatus>,
) -> Option<SyncStep> {
    let tool = tools.get(name).copied()?;
    if matches!(tool.status, Status::Ok) {
        return None;
    }

    let missing = matches!(tool.status, Status::Missing);
    Some(SyncStep {
        id: format!("tool:{name}"),
        kind: if missing {
            SyncStepKind::Install
        } else {
            SyncStepKind::Align
        },
        target: name.to_string(),
        reason: tool_sync_reason(tool),
        command: sync_command_for_tool(name, policy, missing),
        file: None,
        snippet: None,
        requires_sudo: false,
    })
}

fn package_manager_step(name: &str, tools: &BTreeMap<&str, &ToolStatus>) -> Option<SyncStep> {
    let tool = tools.get(name).copied();
    if tool.is_some_and(|tool| matches!(tool.status, Status::Ok)) {
        return None;
    }

    if tool.is_none() && command_path(name).is_some() {
        return None;
    }

    Some(SyncStep {
        id: format!("tool:{name}"),
        kind: if is_tool_missing(tool) {
            SyncStepKind::Install
        } else {
            SyncStepKind::Align
        },
        target: name.to_string(),
        reason: if is_tool_missing(tool) {
            format!("install {name} required by the configured Node workflow")
        } else {
            format!("align {name} with the configured Node workflow")
        },
        command: sync_command_for_tool(name, &ToolPolicy::default(), is_tool_missing(tool)),
        file: None,
        snippet: None,
        requires_sudo: false,
    })
}

fn package_step(package: &str, manager: Option<&str>) -> Option<SyncStep> {
    if command_path(package).is_some() {
        return None;
    }

    let manager = manager.unwrap_or("brew");
    let command = match manager {
        "brew" | "homebrew" => Some(format!("brew install {package}")),
        "npm" => Some(format!("npm install -g {package}@latest")),
        _ => Some(format!("{manager} install {package}")),
    };

    Some(SyncStep {
        id: format!("package:{manager}:{package}"),
        kind: SyncStepKind::Install,
        target: package.to_string(),
        reason: format!("install CLI package managed by {manager}"),
        command,
        file: None,
        snippet: None,
        requires_sudo: false,
    })
}

fn issue_step(issue: &Issue) -> Option<SyncStep> {
    match issue.kind {
        IssueKind::MissingTool => None,
        IssueKind::PathConflict => Some(SyncStep {
            id: format!("issue:path-conflict:{}", issue_id_suffix(issue)),
            kind: SyncStepKind::Configure,
            target: issue
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "PATH ordering".to_string()),
            reason: issue.message.clone(),
            command: issue.fix.clone(),
            file: None,
            snippet: None,
            requires_sudo: false,
        }),
        IssueKind::LegacyInstall => Some(SyncStep {
            id: format!("issue:cleanup:{}", issue_id_suffix(issue)),
            kind: SyncStepKind::Cleanup,
            target: issue
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "legacy install".to_string()),
            reason: issue.message.clone(),
            command: issue.fix.clone(),
            file: issue.path.clone(),
            snippet: None,
            requires_sudo: issue.fix.as_ref().is_some_and(|fix| fix.contains("sudo ")),
        }),
    }
}

fn sync_command_for_tool(name: &str, policy: &ToolPolicy, missing: bool) -> Option<String> {
    let manager = policy.manager.as_deref().unwrap_or_default();
    match name {
        "brew" => {
            if missing {
                Some("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".to_string())
            } else {
                Some("brew update && brew upgrade".to_string())
            }
        }
        "fnm" => Some(if missing {
            "brew install fnm".to_string()
        } else {
            "brew upgrade fnm".to_string()
        }),
        "node" => match manager {
            "fnm" | "" => {
                let version = node_selector(policy.version.as_deref());
                Some(format!("fnm install {version} && fnm default {version}"))
            }
            "brew" | "homebrew" => Some("brew install node".to_string()),
            _ => None,
        },
        "npm" => Some("npm install -g npm@latest".to_string()),
        "pnpm" => Some("corepack enable && corepack prepare pnpm@latest --activate".to_string()),
        "bun" => Some(
            if manager == "brew" || manager == "homebrew" || manager.is_empty() {
                if missing {
                    "brew install oven-sh/bun/bun".to_string()
                } else {
                    "brew upgrade oven-sh/bun/bun".to_string()
                }
            } else {
                "bun upgrade".to_string()
            },
        ),
        "wrangler" => Some("npm install -g wrangler@latest".to_string()),
        "uv" => Some(if missing {
            "curl -LsSf https://astral.sh/uv/install.sh | sh".to_string()
        } else {
            "uv self update".to_string()
        }),
        "go" => {
            let version = policy
                .version
                .clone()
                .unwrap_or_else(|| "stable".to_string());
            let install_dir = policy
                .install_dir
                .clone()
                .unwrap_or_else(|| "~/.local/opt/go/current".to_string());
            let source = policy.source.as_deref().unwrap_or(manager);
            if matches!(source, "brew" | "homebrew") {
                Some("brew install go".to_string())
            } else {
                Some(format!(
                    "install Go {version} from https://go.dev/dl/ into {install_dir}"
                ))
            }
        }
        _ => None,
    }
}

fn tool_sync_reason(tool: &ToolStatus) -> String {
    match tool.status {
        Status::Missing => "tool is required by policy but not installed".to_string(),
        Status::Mismatch => "installed tool does not match the configured policy".to_string(),
        Status::Unknown => "tool is present but its version could not be detected".to_string(),
        Status::Ok => "tool already matches the configured policy".to_string(),
    }
}

fn brew_required(config: &DevkitConfig) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools
            .values()
            .any(|policy| matches!(policy.manager.as_deref(), Some("brew") | Some("homebrew")))
    }) || config
        .homebrew
        .as_ref()
        .and_then(|homebrew| homebrew.packages.as_ref())
        .is_some_and(|packages| !packages.is_empty())
}

fn uses_fnm(config: &DevkitConfig) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools.contains_key("fnm")
            || tools
                .get("node")
                .and_then(|policy| policy.manager.as_deref())
                .is_some_and(|manager| manager == "fnm")
    })
}

fn homebrew_mirror(config: &DevkitConfig) -> Option<String> {
    config
        .homebrew
        .as_ref()
        .and_then(|homebrew| homebrew.mirror.clone())
        .or_else(|| {
            config
                .policy
                .as_ref()
                .and_then(|policy| policy.mirror.clone())
        })
}

fn homebrew_mirror_snippet(mirror: &str) -> Option<String> {
    match mirror {
        "ustc" => Some(
            "export HOMEBREW_API_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles/api\"\nexport HOMEBREW_BOTTLE_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles\"".to_string(),
        ),
        _ => None,
    }
}

fn go_install_dir(config: &DevkitConfig) -> Option<String> {
    config
        .tools
        .as_ref()
        .and_then(|tools| tools.get("go"))
        .and_then(|policy| policy.install_dir.clone())
}

fn node_selector(version: Option<&str>) -> String {
    match version {
        Some("latest") => "latest".to_string(),
        Some("stable") => "lts-latest".to_string(),
        Some(version) => version
            .trim_start_matches('v')
            .trim_end_matches(".x")
            .to_string(),
        None => "lts-latest".to_string(),
    }
}

fn current_shell_name() -> &'static str {
    if std::env::var("SHELL")
        .unwrap_or_default()
        .ends_with("/bash")
    {
        "bash"
    } else {
        "zsh"
    }
}

fn shell_rc_path() -> PathBuf {
    home_path(if current_shell_name() == "bash" {
        ".bashrc"
    } else {
        ".zshrc"
    })
}

fn shell_profile_path() -> PathBuf {
    home_path(if current_shell_name() == "bash" {
        ".bash_profile"
    } else {
        ".zprofile"
    })
}

fn home_path(path: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(path)
}

fn is_rust_policy(name: &str) -> bool {
    matches!(name, "rust" | "rustup" | "rustc" | "cargo")
}

fn tool_needs_sync(tool: Option<&ToolStatus>) -> bool {
    tool.is_some_and(|tool| !matches!(tool.status, Status::Ok))
}

fn is_tool_missing(tool: Option<&ToolStatus>) -> bool {
    tool.is_none_or(|tool| matches!(tool.status, Status::Missing))
}

fn push_step(steps: &mut Vec<SyncStep>, seen: &mut BTreeSet<String>, step: SyncStep) {
    if seen.insert(step.id.clone()) {
        steps.push(step);
    }
}

fn issue_id_suffix(issue: &Issue) -> String {
    issue
        .path
        .as_ref()
        .map(|path| {
            path.to_string_lossy()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                .collect()
        })
        .unwrap_or_else(|| "issue".to_string())
}

fn report_matches_policy(report: &DoctorReport) -> bool {
    let tools_match = report.tools.iter().all(|tool| {
        matches!(tool.status, Status::Ok) || (tool.required.is_none() && tool.manager.is_none())
    });
    let issues_match = report
        .issues
        .iter()
        .all(|issue| matches!(issue.kind, IssueKind::MissingTool));

    tools_match && issues_match
}

fn summarize_output(prefix: &str, output: &str) -> String {
    let first_line = output.lines().find(|line| !line.trim().is_empty());
    match first_line {
        Some(line) => format!("{prefix}: {}", line.trim()),
        None => prefix.to_string(),
    }
}

fn ensure_managed_snippet(path: &Path, step_id: &str, snippet: &str) -> Result<bool> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_managed_block(&existing, step_id, snippet);

    if updated == existing {
        return Ok(false);
    }

    if !existing.is_empty() && path.exists() {
        backup_file(path, &existing)?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    fs::write(path, updated).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn upsert_managed_block(existing: &str, step_id: &str, snippet: &str) -> String {
    let block = managed_block(step_id, snippet);
    let start = managed_block_start(step_id);
    let end = managed_block_end(step_id);
    let snippet_body = snippet.trim_end();
    let normalized = strip_managed_blocks(existing, &start, &end);

    let matches = normalized
        .match_indices(snippet_body)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if let Some(first) = matches.first().copied() {
        let mut updated = String::with_capacity(normalized.len() + block.len());
        updated.push_str(&normalized[..first]);
        updated.push_str(&block);
        let mut cursor = first + snippet_body.len();
        for duplicate in matches.iter().skip(1) {
            updated.push_str(&normalized[cursor..*duplicate]);
            cursor = duplicate + snippet_body.len();
        }
        updated.push_str(&normalized[cursor..]);
        return trim_excess_blank_lines(&updated);
    }

    if normalized.is_empty() {
        return block;
    }

    let mut updated = trim_excess_blank_lines(&normalized);
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.ends_with("\n\n") {
        updated.push('\n');
    }
    updated.push_str(&block);
    updated
}

fn managed_block(step_id: &str, snippet: &str) -> String {
    format!(
        "{}\n{}\n{}\n",
        managed_block_start(step_id),
        snippet.trim_end(),
        managed_block_end(step_id)
    )
}

fn managed_block_start(step_id: &str) -> String {
    format!("# >>> devkit sync {} >>>", step_id)
}

fn managed_block_end(step_id: &str) -> String {
    format!("# <<< devkit sync {} <<<", step_id)
}

fn strip_managed_blocks(existing: &str, start: &str, end: &str) -> String {
    let mut remaining = existing;
    let mut stripped = String::new();

    loop {
        let Some(start_idx) = remaining.find(start) else {
            stripped.push_str(remaining);
            return trim_excess_blank_lines(&stripped);
        };
        stripped.push_str(&remaining[..start_idx]);
        let Some(end_relative) = remaining[start_idx..].find(end) else {
            return trim_excess_blank_lines(&stripped);
        };
        let block_end = start_idx + end_relative + end.len();
        remaining = remaining[block_end..]
            .strip_prefix('\n')
            .unwrap_or(&remaining[block_end..]);
    }
}

fn trim_excess_blank_lines(text: &str) -> String {
    let mut result = String::new();
    let mut blank_run = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            result.push('\n');
            continue;
        }

        blank_run = 0;
        result.push_str(line);
        result.push('\n');
    }

    while result.ends_with("\n\n\n") {
        result.pop();
    }

    result
}

fn backup_file(path: &Path, contents: &str) -> Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("failed to compute backup timestamp")?
        .as_secs();
    let backup_path = path.with_extension(format!("devkit.bak.{timestamp}"));
    fs::write(&backup_path, contents)
        .with_context(|| format!("failed to write backup {}", backup_path.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        config::{DevkitConfig, HomebrewConfig, NpmConfig, ToolPolicy},
        doctor::{DoctorReport, Issue, Status, ToolStatus},
    };

    use super::{
        SyncStepKind, build_sync_plan, ensure_managed_snippet, managed_block, upsert_managed_block,
    };

    #[test]
    fn builds_bootstrap_steps_for_missing_toolchain() {
        let config = DevkitConfig {
            policy: None,
            tools: Some(
                [
                    (
                        "fnm".to_string(),
                        ToolPolicy {
                            version: Some("latest".to_string()),
                            manager: Some("brew".to_string()),
                            ..ToolPolicy::default()
                        },
                    ),
                    (
                        "node".to_string(),
                        ToolPolicy {
                            version: Some("24.x".to_string()),
                            manager: Some("fnm".to_string()),
                            package_managers: Some(vec!["pnpm".to_string()]),
                            ..ToolPolicy::default()
                        },
                    ),
                    (
                        "uv".to_string(),
                        ToolPolicy {
                            version: Some("latest".to_string()),
                            manager: Some("standalone".to_string()),
                            ..ToolPolicy::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            homebrew: Some(HomebrewConfig {
                mirror: None,
                packages: None,
            }),
            npm: Some(NpmConfig {
                global_packages: None,
            }),
        };
        let report = DoctorReport {
            tools: vec![
                missing_tool("brew"),
                missing_tool("fnm"),
                missing_tool("node"),
                missing_tool("pnpm"),
                missing_tool("uv"),
            ],
            issues: vec![],
        };

        let plan = build_sync_plan(true, Path::new("examples/devkit.toml"), &config, &report);
        let ids = plan
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"tool:brew"));
        assert!(ids.contains(&"tool:fnm"));
        assert!(ids.contains(&"tool:node"));
        assert!(ids.contains(&"tool:pnpm"));
        assert!(ids.contains(&"tool:uv"));
        assert!(ids.contains(&"config:fnm-shell"));
        assert!(ids.contains(&"verify:doctor"));
    }

    #[test]
    fn collapses_rust_into_one_alignment_step() {
        let config = DevkitConfig {
            policy: None,
            tools: Some(
                [
                    (
                        "rustc".to_string(),
                        ToolPolicy {
                            version: Some("stable".to_string()),
                            manager: Some("rustup".to_string()),
                            ..ToolPolicy::default()
                        },
                    ),
                    (
                        "cargo".to_string(),
                        ToolPolicy {
                            version: Some("stable".to_string()),
                            manager: Some("rustup".to_string()),
                            ..ToolPolicy::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            homebrew: None,
            npm: None,
        };
        let report = DoctorReport {
            tools: vec![
                ToolStatus {
                    name: "rustup".to_string(),
                    command: "rustup".to_string(),
                    path: Some(PathBuf::from("/tmp/rustup")),
                    current: Some("1.29.0".to_string()),
                    required: None,
                    manager: None,
                    status: Status::Ok,
                    note: None,
                },
                ToolStatus {
                    name: "rustc".to_string(),
                    command: "rustc".to_string(),
                    path: Some(PathBuf::from("/tmp/rustc")),
                    current: Some("1.94.0".to_string()),
                    required: Some("stable".to_string()),
                    manager: Some("rustup".to_string()),
                    status: Status::Mismatch,
                    note: None,
                },
                ToolStatus {
                    name: "cargo".to_string(),
                    command: "cargo".to_string(),
                    path: Some(PathBuf::from("/tmp/cargo")),
                    current: Some("1.94.0".to_string()),
                    required: Some("stable".to_string()),
                    manager: Some("rustup".to_string()),
                    status: Status::Mismatch,
                    note: None,
                },
            ],
            issues: vec![Issue {
                kind: crate::doctor::IssueKind::PathConflict,
                path: None,
                message: "ignored".to_string(),
                fix: None,
            }],
        };

        let plan = build_sync_plan(true, Path::new("devkit.toml"), &config, &report);
        let rust_steps = plan
            .steps
            .iter()
            .filter(|step| step.id == "tool:rust")
            .collect::<Vec<_>>();

        assert_eq!(rust_steps.len(), 1);
        assert!(matches!(rust_steps[0].kind, SyncStepKind::Align));
    }

    #[test]
    fn upserts_managed_block_without_duplication() {
        let existing = "# existing\n";
        let updated = upsert_managed_block(
            existing,
            "config:fnm-shell",
            "eval \"$(fnm env --shell zsh)\"",
        );
        let rerun = upsert_managed_block(
            &updated,
            "config:fnm-shell",
            "eval \"$(fnm env --shell zsh)\"",
        );

        assert_eq!(updated, rerun);
        assert!(updated.contains(&managed_block(
            "config:fnm-shell",
            "eval \"$(fnm env --shell zsh)\""
        )));
    }

    #[test]
    fn converts_existing_plain_snippet_into_managed_block() {
        let existing = "# shell\nexport HOMEBREW_API_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles/api\"\nexport HOMEBREW_BOTTLE_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles\"\n";
        let updated = upsert_managed_block(
            existing,
            "config:homebrew-mirror",
            "export HOMEBREW_API_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles/api\"\nexport HOMEBREW_BOTTLE_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles\"",
        );

        assert!(updated.contains("# >>> devkit sync config:homebrew-mirror >>>"));
        assert_eq!(
            updated
                .matches("export HOMEBREW_API_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles/api\"")
                .count(),
            1
        );
        assert_eq!(
            updated
                .matches(
                    "export HOMEBREW_BOTTLE_DOMAIN=\"https://mirrors.ustc.edu.cn/homebrew-bottles\""
                )
                .count(),
            1
        );
    }

    #[test]
    fn collapses_duplicate_managed_blocks() {
        let duplicate = format!(
            "{}\n# comment\n{}",
            managed_block("config:test", "export TEST_ONE=1"),
            managed_block("config:test", "export TEST_ONE=1")
        );
        let updated = upsert_managed_block(&duplicate, "config:test", "export TEST_ONE=1");

        assert_eq!(
            updated.matches("# >>> devkit sync config:test >>>").count(),
            1
        );
        assert_eq!(updated.matches("export TEST_ONE=1").count(), 1);
    }

    #[test]
    fn writes_managed_snippet_idempotently() {
        let path = unique_temp_path("sync-snippet");
        let snippet = "export PATH=\"$HOME/.local/bin:$PATH\"";

        let first = ensure_managed_snippet(&path, "config:test", snippet).unwrap();
        let second = ensure_managed_snippet(&path, "config:test", snippet).unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert!(first);
        assert!(!second);
        assert!(contents.contains(snippet));

        let _ = fs::remove_file(&path);
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

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{timestamp}.txt"))
    }
}
