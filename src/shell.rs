use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};

#[derive(Debug)]
pub struct ShellCommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub fn command_path(command: &str) -> Option<PathBuf> {
    shell_output(&format!("command -v {command}"))
        .and_then(|path| path.lines().next().map(str::trim).map(PathBuf::from))
        .or_else(|| which::which(command).ok())
}

pub fn command_version(command: &str, args: &[&str]) -> Option<String> {
    let script = std::iter::once(command)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");

    shell_output(&script)
        .and_then(|output| output.lines().next().map(|line| line.trim().to_string()))
        .or_else(|| direct_output(command, args))
}

pub fn shell_output(script: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = Command::new(shell).args(["-ic", script]).output().ok()?;
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
    let output = Command::new(command).args(args).output().ok()?;
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
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = Command::new(&shell)
        .args(["-ic", script])
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
