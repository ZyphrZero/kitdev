use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CleanupPlan {
    pub dry_run: bool,
    pub items: Vec<CleanupItem>,
}

#[derive(Debug, Serialize)]
pub struct CleanupItem {
    pub path: PathBuf,
    pub reason: String,
    pub command: String,
    pub requires_sudo: bool,
}

pub fn build_cleanup_plan(dry_run: bool) -> CleanupPlan {
    let candidates = [
        (
            PathBuf::from("/usr/local/go"),
            "legacy official Go install",
            "sudo rm -rf /usr/local/go",
            true,
        ),
        (
            PathBuf::from("/usr/local/lib/node_modules"),
            "legacy global Node modules",
            "sudo rm -rf /usr/local/lib/node_modules",
            true,
        ),
        (
            home_path(".nvm"),
            "legacy nvm directory",
            "rm -rf ~/.nvm",
            false,
        ),
    ];

    let items = candidates
        .into_iter()
        .filter(|(path, _, _, _)| path_exists(path))
        .map(|(path, reason, command, requires_sudo)| CleanupItem {
            path,
            reason: reason.to_string(),
            command: command.to_string(),
            requires_sudo,
        })
        .collect();

    CleanupPlan { dry_run, items }
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
