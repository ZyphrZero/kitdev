use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    config::DevkitConfig,
    shell::{command_path, command_version},
};

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub name: String,
    pub command: String,
    pub path: Option<PathBuf>,
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
    pub path: Option<PathBuf>,
    pub message: String,
    pub fix: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueKind {
    PathConflict,
    LegacyInstall,
    MissingTool,
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
        name: "bun",
        command: "bun",
        version_args: &["--version"],
        version_prefix: None,
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
];

pub fn build_doctor_report(config: &DevkitConfig) -> DoctorReport {
    let tools = TOOL_SPECS
        .iter()
        .map(|spec| inspect_tool(spec, config))
        .collect::<Vec<_>>();
    let issues = detect_issues(&tools);

    DoctorReport { tools, issues }
}

fn inspect_tool(spec: &ToolSpec, config: &DevkitConfig) -> ToolStatus {
    let policy = config.tools.as_ref().and_then(|tools| tools.get(spec.name));
    let required = policy.and_then(|policy| policy.version.clone());
    let manager = policy.and_then(|policy| policy.manager.clone());

    let path = command_path(spec.command);
    let current = path
        .as_ref()
        .and_then(|_| command_version(spec.command, spec.version_args));
    let normalized = current
        .as_deref()
        .map(|version| normalize_version(version, spec.version_prefix));

    let status = match (&path, &normalized, &required) {
        (None, _, _) => Status::Missing,
        (Some(_), Some(current), Some(required)) if version_satisfies(current, required) => {
            Status::Ok
        }
        (Some(_), Some(_), Some(_)) => Status::Mismatch,
        (Some(_), Some(_), None) => Status::Ok,
        (Some(_), None, _) => Status::Unknown,
    };

    let note = match status {
        Status::Missing => Some("command not found in PATH".to_string()),
        Status::Mismatch => Some("installed version does not match configured policy".to_string()),
        Status::Unknown => Some("installed, but version could not be detected".to_string()),
        Status::Ok => None,
    };

    ToolStatus {
        name: spec.name.to_string(),
        command: spec.command.to_string(),
        path,
        current: normalized,
        required,
        manager,
        status,
        note,
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

fn detect_issues(tools: &[ToolStatus]) -> Vec<Issue> {
    let mut issues = Vec::new();

    for tool in tools {
        if matches!(tool.status, Status::Missing) {
            issues.push(Issue {
                kind: IssueKind::MissingTool,
                path: None,
                message: format!("{} is not available in PATH", tool.name),
                fix: suggested_install(&tool.name),
            });
        }
    }

    if path_exists("/usr/local/go") {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            path: Some(PathBuf::from("/usr/local/go")),
            message: "legacy official Go install may shadow managed Go versions".to_string(),
            fix: Some("sudo rm -rf /usr/local/go".to_string()),
        });
    }

    if path_exists("/usr/local/lib/node_modules") {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            path: Some(PathBuf::from("/usr/local/lib/node_modules")),
            message: "legacy global Node modules remain under /usr/local".to_string(),
            fix: Some("sudo rm -rf /usr/local/lib/node_modules".to_string()),
        });
    }

    if path_exists(home_path(".nvm")) {
        issues.push(Issue {
            kind: IssueKind::LegacyInstall,
            path: Some(home_path(".nvm")),
            message: "nvm directory exists; verify it is not initialized by your shell".to_string(),
            fix: Some("remove ~/.nvm after confirming it is unused".to_string()),
        });
    }

    detect_path_conflict(
        &mut issues,
        tools,
        "go",
        &["/usr/local/go/bin", "/opt/homebrew/bin"],
    );
    detect_path_conflict(
        &mut issues,
        tools,
        "node",
        &["/usr/local/bin", "/Applications/Codex.app"],
    );

    issues
}

fn suggested_install(name: &str) -> Option<String> {
    match name {
        "fnm" => Some("brew install fnm".to_string()),
        "node" => Some("fnm install --lts && fnm default lts-latest".to_string()),
        "pnpm" => Some("corepack enable && corepack prepare pnpm@latest --activate".to_string()),
        "bun" => Some("brew install oven-sh/bun/bun".to_string()),
        "uv" => Some("curl -LsSf https://astral.sh/uv/install.sh | sh".to_string()),
        "rustup" | "rustc" | "cargo" => Some("curl https://sh.rustup.rs -sSf | sh".to_string()),
        "go" => Some("install from https://go.dev/dl/ or brew install go".to_string()),
        "brew" => Some("install Homebrew from https://brew.sh".to_string()),
        _ => None,
    }
}

fn detect_path_conflict(
    issues: &mut Vec<Issue>,
    tools: &[ToolStatus],
    tool_name: &str,
    suspicious: &[&str],
) {
    let Some(tool) = tools.iter().find(|tool| tool.name == tool_name) else {
        return;
    };
    let Some(path) = &tool.path else {
        return;
    };
    let path_text = path.to_string_lossy();

    if suspicious.iter().any(|segment| path_text.contains(segment)) {
        issues.push(Issue {
            kind: IssueKind::PathConflict,
            path: Some(path.clone()),
            message: format!(
                "{} resolves to a path that may conflict with managed toolchains",
                tool_name
            ),
            fix: Some("inspect shell PATH ordering and prefer your configured manager".to_string()),
        });
    }
}

fn path_exists(path: impl AsRef<Path>) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn home_path(path: impl AsRef<Path>) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(path)
}

#[cfg(test)]
mod tests {
    use super::{normalize_version, version_satisfies};

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
}
