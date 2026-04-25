use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::platform::{OperatingSystem, home_path};

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
    let candidates = cleanup_candidates(OperatingSystem::current());

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

fn cleanup_candidates(platform: OperatingSystem) -> Vec<(PathBuf, &'static str, String, bool)> {
    match platform {
        OperatingSystem::Windows => vec![
            (
                windows_appdata_path("nvm"),
                "legacy nvm directory",
                "Remove-Item -Recurse -Force $env:APPDATA\\nvm".to_string(),
                false,
            ),
            (
                home_path("AppData/Roaming/npm"),
                "legacy global npm bin directory",
                "Remove-Item -Recurse -Force $env:APPDATA\\npm".to_string(),
                false,
            ),
        ],
        OperatingSystem::Macos | OperatingSystem::Linux | OperatingSystem::Other => vec![
            (
                PathBuf::from("/usr/local/go"),
                "legacy official Go install",
                "sudo rm -rf /usr/local/go".to_string(),
                true,
            ),
            (
                PathBuf::from("/usr/local/lib/node_modules"),
                "legacy global Node modules",
                "sudo rm -rf /usr/local/lib/node_modules".to_string(),
                true,
            ),
            (
                home_path(".nvm"),
                "legacy nvm directory",
                "rm -rf ~/.nvm".to_string(),
                false,
            ),
        ],
    }
}

fn path_exists(path: impl AsRef<Path>) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn windows_appdata_path(path: &str) -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path("AppData/Roaming"))
        .join(path)
}

#[cfg(test)]
mod tests {
    use crate::platform::OperatingSystem;

    use super::cleanup_candidates;

    #[test]
    fn builds_windows_cleanup_commands() {
        let candidates = cleanup_candidates(OperatingSystem::Windows);

        assert!(
            candidates
                .iter()
                .any(|(_, _, command, _)| command.contains("Remove-Item"))
        );
    }

    #[test]
    fn builds_unix_cleanup_commands() {
        let candidates = cleanup_candidates(OperatingSystem::Linux);

        assert!(
            candidates
                .iter()
                .any(|(path, _, command, requires_sudo)| path.ends_with("go")
                    && command == "sudo rm -rf /usr/local/go"
                    && *requires_sudo)
        );
    }
}
