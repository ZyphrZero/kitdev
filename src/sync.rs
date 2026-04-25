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
    package_manager::detected_cli_manager,
    platform::{OperatingSystem, home_path, path_contains},
    shell::{command_path, run_shell_command},
    tool_command::{
        ToolCommand, command_for_package, command_for_tool, homebrew_install_command,
        rustup_install_command,
    },
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
    pub graph: SyncPlanGraph,
    pub steps: Vec<SyncStep>,
}

#[derive(Debug, Serialize)]
pub struct SyncPlanGraph {
    pub ready: Vec<String>,
    pub blocked: Vec<SyncBlockedStep>,
}

#[derive(Debug, Serialize)]
pub struct SyncBlockedStep {
    pub id: String,
    pub target: String,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncStep {
    pub id: String,
    pub kind: SyncStepKind,
    pub target: String,
    pub reason: String,
    pub blocked_by: Vec<String>,
    pub command: Option<String>,
    pub file: Option<PathBuf>,
    pub snippet: Option<String>,
    pub manual: bool,
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
    pub blocked_by: Vec<String>,
    pub command: Option<String>,
    pub file: Option<PathBuf>,
    pub manual: bool,
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
                blocked_by: Vec::new(),
                command: Some(homebrew_install_command()),
                file: None,
                snippet: None,
                manual: false,
                requires_sudo: false,
            },
        );
    }

    if winget_required(config) && is_tool_missing(tools.get("winget").copied()) {
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "tool:winget".to_string(),
                kind: SyncStepKind::Install,
                target: "Windows Package Manager".to_string(),
                reason: "install the package manager used by the configured toolchain".to_string(),
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

    if let Some(step) = rust_sync_step(config, &tools) {
        push_step(&mut steps, &mut seen, step);
    }

    if uses_nvm(config) && !nvm_installed() {
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "tool:nvm".to_string(),
                kind: SyncStepKind::Install,
                target: "nvm".to_string(),
                reason: "install nvm required by the configured Node workflow".to_string(),
                blocked_by: Vec::new(),
                command: sync_command_for_tool(
                    "nvm",
                    &ToolPolicy {
                        manager: Some("standalone".to_string()),
                        ..ToolPolicy::default()
                    },
                    true,
                )
                .and_then(|command| command.into_step_parts().0),
                file: None,
                snippet: None,
                manual: OperatingSystem::current().is_windows(),
                requires_sudo: false,
            },
        );
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

    if !OperatingSystem::current().is_windows()
        && let Some(mirror) = homebrew_mirror(config)
    {
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
                    blocked_by: Vec::new(),
                    command: None,
                    file: Some(profile_path),
                    snippet: homebrew_mirror_snippet(&mirror),
                    manual: false,
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
        let fnm_missing = is_tool_missing(tools.get("fnm").copied());
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "config:fnm-shell".to_string(),
                kind: SyncStepKind::Configure,
                target: format!("{shell} shell initialization"),
                reason: "initialize fnm in your shell so managed Node versions are available"
                    .to_string(),
                blocked_by: if fnm_missing {
                    vec!["tool:fnm".to_string()]
                } else {
                    Vec::new()
                },
                command: None,
                file: Some(shell_rc_path()),
                snippet: Some(fnm_shell_snippet(shell)),
                manual: false,
                requires_sudo: false,
            },
        );
    }

    if uses_nvm(config) && (tool_needs_sync(tools.get("node").copied()) || !nvm_installed()) {
        let shell = current_shell_name();
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "config:nvm-shell".to_string(),
                kind: SyncStepKind::Configure,
                target: format!("{shell} shell initialization"),
                reason: "initialize nvm in your shell so managed Node versions are available"
                    .to_string(),
                blocked_by: if nvm_installed() {
                    Vec::new()
                } else {
                    vec!["tool:nvm".to_string()]
                },
                command: None,
                file: Some(shell_rc_path()),
                snippet: Some(nvm_shell_snippet(shell)),
                manual: false,
                requires_sudo: false,
            },
        );
    }

    if let Some(install_dir) = go_install_dir(config)
        && tool_needs_sync(tools.get("go").copied())
    {
        let go_missing = is_tool_missing(tools.get("go").copied());
        push_step(
            &mut steps,
            &mut seen,
            SyncStep {
                id: "config:go-path".to_string(),
                kind: SyncStepKind::Configure,
                target: "Go PATH ordering".to_string(),
                reason: "ensure the configured Go install directory takes precedence in PATH"
                    .to_string(),
                blocked_by: if go_missing {
                    vec!["tool:go".to_string()]
                } else {
                    Vec::new()
                },
                command: None,
                file: Some(shell_rc_path()),
                snippet: Some(format!("export PATH=\"{install_dir}/bin:$PATH\"")),
                manual: false,
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
            blocked_by: Vec::new(),
            command: None,
            file: None,
            snippet: None,
            manual: false,
            requires_sudo: false,
        });
    }

    steps.push(SyncStep {
        id: "verify:doctor".to_string(),
        kind: SyncStepKind::Verify,
        target: "environment".to_string(),
        reason: "re-run doctor after applying the planned steps".to_string(),
        blocked_by: Vec::new(),
        command: Some(format!("devkit doctor --config {}", config_path.display())),
        file: None,
        snippet: None,
        manual: false,
        requires_sudo: false,
    });

    let graph = build_sync_graph(&steps);

    SyncPlan {
        dry_run,
        policy_channel,
        platform,
        auto_fix,
        graph,
        steps,
    }
}

pub fn execute_sync_plan(plan: &SyncPlan, config: &DevkitConfig) -> SyncExecution {
    execute_sync_plan_with_progress(plan, config, |_, _, _| {})
}

pub fn execute_sync_plan_with_progress<F>(
    plan: &SyncPlan,
    config: &DevkitConfig,
    mut on_step: F,
) -> SyncExecution
where
    F: FnMut(&SyncStep, usize, usize),
{
    let mut results = Vec::new();
    let mut succeeded = true;
    let mut verify_step = None;
    let total_steps = plan.steps.len();
    let mut current_step = 0;
    let ready_ids = plan
        .graph
        .ready
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    for step in &plan.steps {
        if matches!(step.kind, SyncStepKind::Verify) {
            verify_step = Some(step);
            continue;
        }
        current_step += 1;
        on_step(step, current_step, total_steps);

        if !ready_ids.contains(step.id.as_str()) {
            succeeded = false;
            results.push(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail: format!("blocked by {}", step.blocked_by.join(", ")),
                blocked_by: step.blocked_by.clone(),
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            });
            continue;
        }

        let unresolved_blockers = unresolved_blockers(step);
        if !unresolved_blockers.is_empty() {
            succeeded = false;
            results.push(SyncStepExecution {
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

        let execution = match step.kind {
            SyncStepKind::Install | SyncStepKind::Align if step.manual => Ok(SyncStepExecution {
                id: step.id.clone(),
                kind: step.kind,
                target: step.target.clone(),
                status: SyncStepExecutionStatus::Skipped,
                detail: "manual instruction not applied; review the suggested action".to_string(),
                blocked_by: step.blocked_by.clone(),
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            }),
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
                blocked_by: step.blocked_by.clone(),
                command: step.command.clone(),
                file: step.file.clone(),
                manual: step.manual,
            }),
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
                    blocked_by: step.blocked_by.clone(),
                    command: step.command.clone(),
                    file: step.file.clone(),
                    manual: step.manual,
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
        current_step += 1;
        on_step(step, current_step, total_steps);
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
        blocked_by: step.blocked_by.clone(),
        command: step.command.clone(),
        file: step.file.clone(),
        manual: step.manual,
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
            blocked_by: step.blocked_by.clone(),
            command: step.command.clone(),
            file: step.file.clone(),
            manual: step.manual,
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
            blocked_by: step.blocked_by.clone(),
            command: step.command.clone(),
            file: step.file.clone(),
            manual: step.manual,
        });
    }

    Ok(SyncStepExecution {
        id: step.id.clone(),
        kind: step.kind,
        target: step.target.clone(),
        status: SyncStepExecutionStatus::Unchanged,
        detail: step.reason.clone(),
        blocked_by: step.blocked_by.clone(),
        command: step.command.clone(),
        file: step.file.clone(),
        manual: step.manual,
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
        blocked_by: step.blocked_by.clone(),
        command: step.command.clone(),
        file: step.file.clone(),
        manual: step.manual,
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
        rustup_install_command(OperatingSystem::current())
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
        blocked_by: Vec::new(),
        command: Some(command),
        file: None,
        snippet: None,
        manual: false,
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
    let install = missing || manager_path_mismatch(tool, policy);
    let (command, manual) = sync_command_for_tool(name, policy, install)
        .map(ToolCommand::into_step_parts)
        .unwrap_or((None, false));
    let blocked_by = tool_dependency_blockers(name, policy, tools);
    let manual = manual || manager_prerequisite_missing(name, policy, tools);

    Some(SyncStep {
        id: format!("tool:{name}"),
        kind: if install {
            SyncStepKind::Install
        } else {
            SyncStepKind::Align
        },
        target: name.to_string(),
        reason: tool_sync_reason(tool),
        blocked_by,
        command,
        file: None,
        snippet: None,
        manual,
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

    let (command, manual) =
        sync_command_for_tool(name, &ToolPolicy::default(), is_tool_missing(tool))
            .map(ToolCommand::into_step_parts)
            .unwrap_or((None, false));
    let blocked_by = package_manager_dependency_blockers(name, tools);

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
        blocked_by,
        command,
        file: None,
        snippet: None,
        manual,
        requires_sudo: false,
    })
}

fn package_step(package: &str, manager: Option<&str>) -> Option<SyncStep> {
    if command_path(package).is_some() {
        return None;
    }

    let manager = manager
        .filter(|manager| !manager.is_empty())
        .map(ToString::to_string)
        .or_else(|| detected_cli_manager().map(ToString::to_string))
        .or_else(|| {
            OperatingSystem::current()
                .default_manager_for("cli")
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "manual".to_string());
    let (command, manual) = if manager == "manual" {
        (
            Some(format!(
                "choose a platform package manager for CLI package `{package}`"
            )),
            true,
        )
    } else {
        command_for_package(package, &manager)
            .map(ToolCommand::into_step_parts)
            .unwrap_or_else(|error| (Some(error.to_string()), true))
    };
    let manual = manual || (manager == "winget" && command_path("winget").is_none());
    let blocked_by = package_dependency_blockers(&manager);

    Some(SyncStep {
        id: format!("package:{manager}:{package}"),
        kind: SyncStepKind::Install,
        target: package.to_string(),
        reason: format!("install CLI package managed by {manager}"),
        blocked_by,
        command,
        file: None,
        snippet: None,
        manual,
        requires_sudo: false,
    })
}

fn issue_step(issue: &Issue) -> Option<SyncStep> {
    match issue.kind {
        IssueKind::MissingTool => None,
        IssueKind::PlatformMismatch => Some(SyncStep {
            id: "issue:platform-mismatch".to_string(),
            kind: SyncStepKind::Configure,
            target: "policy platform".to_string(),
            reason: issue.message.clone(),
            blocked_by: Vec::new(),
            command: issue.fix.clone(),
            file: None,
            snippet: None,
            manual: true,
            requires_sudo: false,
        }),
        IssueKind::PathConflict => Some(SyncStep {
            id: format!("issue:path-conflict:{}", issue_id_suffix(issue)),
            kind: SyncStepKind::Configure,
            target: issue
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "PATH ordering".to_string()),
            reason: issue.message.clone(),
            blocked_by: Vec::new(),
            command: issue.fix.clone(),
            file: None,
            snippet: None,
            manual: false,
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
            blocked_by: Vec::new(),
            command: issue.fix.clone(),
            file: issue.path.clone(),
            snippet: None,
            manual: false,
            requires_sudo: issue.fix.as_ref().is_some_and(|fix| fix.contains("sudo ")),
        }),
    }
}

fn sync_command_for_tool(name: &str, policy: &ToolPolicy, missing: bool) -> Option<ToolCommand> {
    match command_for_tool(name, policy, missing) {
        Ok(command) => Some(command),
        Err(error) => Some(ToolCommand::Manual(error.to_string())),
    }
}

fn tool_sync_reason(tool: &ToolStatus) -> String {
    match tool.status {
        Status::Missing => "tool is required by policy but not installed".to_string(),
        Status::Mismatch
            if tool.note.as_deref() == Some("installed path does not match configured manager") =>
        {
            "installed tool does not match the configured manager".to_string()
        }
        Status::Mismatch => "installed tool does not match the configured policy".to_string(),
        Status::Unknown => "tool is present but its version could not be detected".to_string(),
        Status::Ok => "tool already matches the configured policy".to_string(),
    }
}

fn manager_path_mismatch(tool: &ToolStatus, policy: &ToolPolicy) -> bool {
    let Some(manager) = policy.manager.as_deref() else {
        return false;
    };
    let Some(path) = &tool.path else {
        return false;
    };
    match manager {
        "brew" | "homebrew" => !OperatingSystem::current().homebrew_path_matches(path),
        "fnm" => !path_contains(path, &["fnm"]),
        "nvm" => !path_contains(path, &["/.nvm/", "\\nvm\\", "/nvm/"]),
        _ => false,
    }
}

fn manager_prerequisite_missing(
    tool: &str,
    policy: &ToolPolicy,
    tools: &BTreeMap<&str, &ToolStatus>,
) -> bool {
    effective_manager_for(tool, policy).is_some_and(|manager| manager == "winget")
        && is_tool_missing(tools.get("winget").copied())
}

fn tool_dependency_blockers(
    tool: &str,
    policy: &ToolPolicy,
    tools: &BTreeMap<&str, &ToolStatus>,
) -> Vec<String> {
    let mut blockers = Vec::new();
    let Some(manager) = effective_manager_for(tool, policy) else {
        return blockers;
    };

    match manager {
        "brew" | "homebrew" => push_tool_blocker_if_missing(&mut blockers, "brew", tools),
        "winget" => push_tool_blocker_if_missing(&mut blockers, "winget", tools),
        "fnm" if tool == "node" => push_tool_blocker_if_missing(&mut blockers, "fnm", tools),
        "nvm" if tool == "node" && !nvm_installed() => push_blocker(&mut blockers, "tool:nvm"),
        "npm" if tool != "npm" => push_tool_blocker_if_missing(&mut blockers, "npm", tools),
        "corepack" => push_tool_blocker_if_missing(&mut blockers, "node", tools),
        "uv" if tool == "python" => push_tool_blocker_if_missing(&mut blockers, "uv", tools),
        "official" if tool == "poetry" && python_missing(tools) => {
            push_blocker(&mut blockers, "tool:python");
        }
        "rustup" if matches!(tool, "rustc" | "cargo") => {
            push_tool_blocker_if_missing(&mut blockers, "rustup", tools)
        }
        _ => {}
    }

    blockers
}

fn package_manager_dependency_blockers(
    package_manager: &str,
    tools: &BTreeMap<&str, &ToolStatus>,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if matches!(package_manager, "npm" | "pnpm" | "yarn") {
        push_tool_blocker_if_missing(&mut blockers, "node", tools);
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

fn push_tool_blocker_if_missing(
    blockers: &mut Vec<String>,
    tool: &str,
    tools: &BTreeMap<&str, &ToolStatus>,
) {
    if dependency_missing(tool, tools) {
        push_blocker(blockers, &format!("tool:{tool}"));
    }
}

fn dependency_missing(tool: &str, tools: &BTreeMap<&str, &ToolStatus>) -> bool {
    tools
        .get(tool)
        .is_none_or(|tool| matches!(tool.status, Status::Missing))
}

fn python_missing(tools: &BTreeMap<&str, &ToolStatus>) -> bool {
    dependency_missing("python", tools)
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

fn brew_required(config: &DevkitConfig) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools.values().any(|policy| {
            matches!(
                policy.manager.as_deref().or(policy.source.as_deref()),
                Some("brew") | Some("homebrew")
            )
        })
    }) || config
        .homebrew
        .as_ref()
        .and_then(|homebrew| homebrew.packages.as_ref())
        .is_some_and(|packages| !packages.is_empty())
}

fn winget_required(config: &DevkitConfig) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools.values().any(|policy| {
            matches!(
                policy.manager.as_deref().or(policy.source.as_deref()),
                Some("winget")
            )
        })
    })
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

fn uses_nvm(config: &DevkitConfig) -> bool {
    config.tools.as_ref().is_some_and(|tools| {
        tools.contains_key("nvm")
            || tools
                .get("node")
                .and_then(|policy| policy.manager.as_deref())
                .is_some_and(|manager| manager == "nvm")
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

fn current_shell_name() -> &'static str {
    OperatingSystem::current().fnm_shell_name()
}

fn fnm_shell_snippet(shell: &str) -> String {
    match shell {
        "powershell" => {
            "fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression".to_string()
        }
        "fish" => "fnm env --use-on-cd --shell fish | source".to_string(),
        _ => format!("eval \"$(fnm env --use-on-cd --shell {shell})\""),
    }
}

fn nvm_shell_snippet(shell: &str) -> String {
    match shell {
        "powershell" => {
            "# nvm-windows does not need shell initialization; restart PowerShell after installation"
                .to_string()
        }
        "fish" => {
            "set -gx NVM_DIR \"$HOME/.nvm\"\n[ -s \"$NVM_DIR/nvm.sh\" ]; and bass source \"$NVM_DIR/nvm.sh\"".to_string()
        }
        _ => {
            "export NVM_DIR=\"$HOME/.nvm\"\n[ -s \"$NVM_DIR/nvm.sh\" ] && \\. \"$NVM_DIR/nvm.sh\"\n[ -s \"$NVM_DIR/bash_completion\" ] && \\. \"$NVM_DIR/bash_completion\"".to_string()
        }
    }
}

fn shell_rc_path() -> PathBuf {
    OperatingSystem::current().shell_rc_path()
}

fn shell_profile_path() -> PathBuf {
    OperatingSystem::current().shell_profile_path()
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

fn nvm_installed() -> bool {
    if command_path("nvm").is_some() {
        return true;
    }

    let nvm_dir = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path(".nvm"));
    nvm_dir.join("nvm.sh").is_file()
}

fn push_step(steps: &mut Vec<SyncStep>, seen: &mut BTreeSet<String>, step: SyncStep) {
    if seen.insert(step.id.clone()) {
        steps.push(step);
    }
}

fn build_sync_graph(steps: &[SyncStep]) -> SyncPlanGraph {
    let mut ready = Vec::new();
    let mut blocked = Vec::new();

    for step in steps {
        if matches!(step.kind, SyncStepKind::Verify) {
            continue;
        }

        if step.blocked_by.is_empty() {
            ready.push(step.id.clone());
        } else {
            blocked.push(SyncBlockedStep {
                id: step.id.clone(),
                target: step.target.clone(),
                blocked_by: step.blocked_by.clone(),
            });
        }
    }

    SyncPlanGraph { ready, blocked }
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
            ..DevkitConfig::default()
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
        assert!(
            plan.steps
                .iter()
                .find(|step| step.id == "tool:pnpm")
                .is_some_and(|step| step.blocked_by == vec!["tool:node"])
        );
        assert_eq!(plan.graph.ready, vec!["tool:brew", "tool:uv"]);
        assert!(
            plan.graph
                .blocked
                .iter()
                .any(|step| { step.id == "tool:fnm" && step.blocked_by == vec!["tool:brew"] })
        );
        assert!(
            plan.graph
                .blocked
                .iter()
                .any(|step| { step.id == "tool:node" && step.blocked_by == vec!["tool:fnm"] })
        );
        assert!(
            plan.graph.blocked.iter().any(|step| {
                step.id == "config:fnm-shell" && step.blocked_by == vec!["tool:fnm"]
            })
        );
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
            ..DevkitConfig::default()
        };
        let report = DoctorReport {
            tools: vec![
                ToolStatus {
                    name: "rustup".to_string(),
                    command: "rustup".to_string(),
                    path: Some(PathBuf::from("/tmp/rustup")),
                    path_candidates: vec![PathBuf::from("/tmp/rustup")],
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
                    path_candidates: vec![PathBuf::from("/tmp/rustc")],
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
                    path_candidates: vec![PathBuf::from("/tmp/cargo")],
                    current: Some("1.94.0".to_string()),
                    required: Some("stable".to_string()),
                    manager: Some("rustup".to_string()),
                    status: Status::Mismatch,
                    note: None,
                },
            ],
            issues: vec![Issue {
                kind: crate::doctor::IssueKind::PathConflict,
                severity: crate::doctor::IssueSeverity::Warning,
                path: None,
                message: "ignored".to_string(),
                evidence: Vec::new(),
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
    fn official_go_sync_step_is_manual_instruction() {
        let config = DevkitConfig {
            policy: None,
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
            homebrew: None,
            npm: None,
            ..DevkitConfig::default()
        };
        let report = DoctorReport {
            tools: vec![missing_tool("go")],
            issues: vec![],
        };

        let plan = build_sync_plan(true, Path::new("devkit.toml"), &config, &report);
        let go_step = plan.steps.iter().find(|step| step.id == "tool:go").unwrap();

        assert!(matches!(go_step.kind, SyncStepKind::Install));
        assert!(go_step.manual);
        assert_eq!(
            go_step.command.as_deref(),
            Some("install Go 1.26.2 from https://go.dev/dl/ into ~/.local/opt/go/current")
        );
    }

    #[test]
    fn winget_policy_adds_windows_package_manager_prerequisite() {
        let config = DevkitConfig {
            policy: None,
            tools: Some(
                [(
                    "deno".to_string(),
                    ToolPolicy {
                        version: Some("latest".to_string()),
                        manager: Some("winget".to_string()),
                        ..ToolPolicy::default()
                    },
                )]
                .into_iter()
                .collect(),
            ),
            homebrew: None,
            npm: None,
            ..DevkitConfig::default()
        };
        let report = DoctorReport {
            tools: vec![missing_tool("winget"), missing_tool("deno")],
            issues: vec![],
        };

        let plan = build_sync_plan(true, Path::new("devkit.toml"), &config, &report);

        assert!(plan.steps.iter().any(|step| {
            step.id == "tool:winget"
                && step.manual
                && step
                    .command
                    .as_deref()
                    .is_some_and(|command| command.contains("getwinget"))
        }));
        assert!(plan.steps.iter().any(|step| {
            step.id == "tool:deno"
                && step.manual
                && step.blocked_by == vec!["tool:winget"]
                && step
                    .command
                    .as_deref()
                    .is_some_and(|command| command.contains("winget install"))
        }));
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
            path_candidates: Vec::new(),
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
