use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use crate::platform::OperatingSystem;

#[derive(Debug)]
pub struct ShellCommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub fn command_path(command: &str) -> Option<PathBuf> {
    command_paths(command).into_iter().next()
}

pub fn command_paths(command: &str) -> Vec<PathBuf> {
    let mut paths = which::which_all(command)
        .map(|paths| paths.collect::<Vec<_>>())
        .unwrap_or_default();
    paths.extend(
        command_lookup_outputs(command)
            .into_iter()
            .map(PathBuf::from),
    );
    dedupe_paths(paths)
}

pub fn command_version(command: &str, args: &[&str]) -> Option<String> {
    let script = std::iter::once(command)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");

    direct_output(command, args).or_else(|| {
        shell_output(&script)
            .and_then(|output| output.lines().next().map(|line| line.trim().to_string()))
    })
}

pub fn shell_output(script: &str) -> Option<String> {
    let shell = command_shell();
    let args = shell_args(&shell, script);
    let output = Command::new(shell)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

fn direct_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };

    text.lines().next().map(|line| line.trim().to_string())
}

pub fn run_shell_command(script: &str) -> Result<ShellCommandOutput> {
    let shell = command_shell();
    let args = shell_args(&shell, script);
    let output = Command::new(&shell)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to start shell command `{script}`"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let summary = if stderr.is_empty() { &stdout } else { &stderr };
        bail!(
            "command `{script}` failed with status {}{}{}",
            output.status,
            if summary.is_empty() { "" } else { ": " },
            summary
        );
    }

    Ok(ShellCommandOutput { stdout, stderr })
}

fn command_lookup_outputs(command: &str) -> Vec<String> {
    let script = if OperatingSystem::current().is_windows() {
        format!("where {command}")
    } else {
        format!("which -a {command}")
    };

    shell_output(&script)
        .map(|paths| {
            paths
                .lines()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn command_shell() -> String {
    if OperatingSystem::current().is_windows() {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn shell_args<'a>(shell: &str, script: &'a str) -> Vec<&'a str> {
    let shell_name = shell.rsplit('/').next().unwrap_or(shell);
    let shell_name = shell_name.rsplit('\\').next().unwrap_or(shell_name);
    if shell_name.eq_ignore_ascii_case("cmd.exe") || shell_name.eq_ignore_ascii_case("cmd") {
        vec!["/C", script]
    } else if shell_name.eq_ignore_ascii_case("powershell.exe")
        || shell_name.eq_ignore_ascii_case("powershell")
        || shell_name.eq_ignore_ascii_case("pwsh.exe")
        || shell_name.eq_ignore_ascii_case("pwsh")
    {
        vec!["-NoProfile", "-Command", script]
    } else if matches!(shell_name, "bash" | "zsh" | "fish") {
        vec!["-lc", script]
    } else {
        vec!["-c", script]
    }
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::<PathBuf>::new();
    for path in paths {
        if !seen.contains(&path) {
            seen.push(path);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{dedupe_paths, shell_args};

    #[test]
    fn builds_windows_shell_args() {
        assert_eq!(
            shell_args("cmd.exe", "where node"),
            vec!["/C", "where node"]
        );
        assert_eq!(
            shell_args(
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
                "Get-Command node"
            ),
            vec!["-NoProfile", "-Command", "Get-Command node"]
        );
    }

    #[test]
    fn builds_unix_shell_args() {
        assert_eq!(
            shell_args("/bin/zsh", "command -v node"),
            vec!["-lc", "command -v node"]
        );
        assert_eq!(
            shell_args("/bin/sh", "command -v node"),
            vec!["-c", "command -v node"]
        );
    }

    #[test]
    fn deduplicates_path_candidates_without_reordering() {
        let paths = dedupe_paths(vec![
            PathBuf::from("/a/node"),
            PathBuf::from("/b/node"),
            PathBuf::from("/a/node"),
        ]);

        assert_eq!(
            paths,
            vec![PathBuf::from("/a/node"), PathBuf::from("/b/node")]
        );
    }
}
