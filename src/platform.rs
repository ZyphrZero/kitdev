use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Macos,
    Linux,
    Windows,
    Other,
}

impl OperatingSystem {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Self::Macos,
            "linux" => Self::Linux,
            "windows" => Self::Windows,
            _ => Self::Other,
        }
    }

    pub fn from_platform_tag(platform: &str) -> Option<Self> {
        let platform = platform.trim().to_ascii_lowercase();
        if platform.starts_with("macos") || platform.starts_with("darwin") {
            Some(Self::Macos)
        } else if platform.starts_with("linux") {
            Some(Self::Linux)
        } else if platform.starts_with("windows") || platform.starts_with("win32") {
            Some(Self::Windows)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Other => "unknown",
        }
    }

    pub fn is_windows(self) -> bool {
        matches!(self, Self::Windows)
    }

    pub fn is_macos(self) -> bool {
        matches!(self, Self::Macos)
    }

    pub fn default_manager_for(self, tool: &str) -> Option<&'static str> {
        match tool {
            "fnm" | "bun" | "deno" => match self {
                Self::Macos => Some("brew"),
                Self::Linux => Some("standalone"),
                Self::Windows => Some("winget"),
                Self::Other => Some("standalone"),
            },
            "node" => Some("fnm"),
            "npm" => Some("npm"),
            "pnpm" | "yarn" => Some("corepack"),
            "wrangler" => Some("npm"),
            "uv" => Some("standalone"),
            "python" => match self {
                Self::Macos => Some("brew"),
                Self::Linux | Self::Windows | Self::Other => Some("uv"),
            },
            "poetry" => match self {
                Self::Macos => Some("brew"),
                Self::Linux | Self::Windows | Self::Other => Some("official"),
            },
            "ruby" => match self {
                Self::Macos => Some("brew"),
                Self::Linux | Self::Windows | Self::Other => Some("manual"),
            },
            "rust" | "rustup" | "rustc" | "cargo" => Some("rustup"),
            "go" => match self {
                Self::Macos => Some("brew"),
                Self::Linux | Self::Windows | Self::Other => Some("official"),
            },
            "cli" => match self {
                Self::Macos => Some("brew"),
                Self::Windows => Some("winget"),
                Self::Linux | Self::Other => None,
            },
            _ => None,
        }
    }

    pub fn default_go_source(self) -> &'static str {
        self.default_manager_for("go").unwrap_or("official")
    }

    pub fn homebrew_path_matches(self, path: &Path) -> bool {
        let path = normalized_path(path);
        match self {
            Self::Macos => {
                path.contains("/opt/homebrew/")
                    || path.contains("/usr/local/")
                    || path.contains("/homebrew/")
            }
            Self::Linux => path.contains("/home/linuxbrew/.linuxbrew/"),
            Self::Windows | Self::Other => false,
        }
    }

    pub fn fnm_shell_name(self) -> &'static str {
        match self {
            Self::Windows => "powershell",
            Self::Macos | Self::Linux | Self::Other => unix_shell_name(),
        }
    }

    pub fn shell_rc_path(self) -> PathBuf {
        match self {
            Self::Windows => home_path("Documents")
                .join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
            Self::Macos | Self::Linux | Self::Other => home_path(match unix_shell_name() {
                "bash" => ".bashrc",
                "fish" => ".config/fish/config.fish",
                _ => ".zshrc",
            }),
        }
    }

    pub fn shell_profile_path(self) -> PathBuf {
        match self {
            Self::Windows => self.shell_rc_path(),
            Self::Macos | Self::Linux | Self::Other => home_path(match unix_shell_name() {
                "bash" => ".bash_profile",
                "fish" => ".config/fish/config.fish",
                _ => ".zprofile",
            }),
        }
    }
}

pub fn current_platform_tag() -> String {
    format!(
        "{}-{}",
        OperatingSystem::current().label(),
        normalized_arch(std::env::consts::ARCH)
    )
}

pub fn home_path(path: impl AsRef<Path>) -> PathBuf {
    home_dir().join(path)
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        })
        .unwrap_or_else(|| PathBuf::from("~"))
}

pub fn path_contains(path: &Path, patterns: &[&str]) -> bool {
    let path = normalized_path(path);
    patterns
        .iter()
        .any(|pattern| path.contains(&pattern.replace('\\', "/").to_ascii_lowercase()))
}

fn unix_shell_name() -> &'static str {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("/bash") {
        "bash"
    } else if shell.ends_with("/fish") {
        "fish"
    } else {
        "zsh"
    }
}

fn normalized_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        other => other,
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::OperatingSystem;

    #[test]
    fn parses_platform_tags() {
        assert_eq!(
            OperatingSystem::from_platform_tag("macos-arm64"),
            Some(OperatingSystem::Macos)
        );
        assert_eq!(
            OperatingSystem::from_platform_tag("linux-x86_64"),
            Some(OperatingSystem::Linux)
        );
        assert_eq!(
            OperatingSystem::from_platform_tag("windows-x86_64"),
            Some(OperatingSystem::Windows)
        );
    }

    #[test]
    fn default_managers_are_platform_aware() {
        assert_eq!(
            OperatingSystem::Macos.default_manager_for("bun"),
            Some("brew")
        );
        assert_eq!(
            OperatingSystem::Linux.default_manager_for("bun"),
            Some("standalone")
        );
        assert_eq!(
            OperatingSystem::Windows.default_manager_for("fnm"),
            Some("winget")
        );
        assert_eq!(
            OperatingSystem::Linux.default_manager_for("python"),
            Some("uv")
        );
    }

    #[test]
    fn homebrew_paths_are_platform_specific() {
        assert!(OperatingSystem::Macos.homebrew_path_matches(Path::new("/opt/homebrew/bin/bun")));
        assert!(
            OperatingSystem::Linux
                .homebrew_path_matches(Path::new("/home/linuxbrew/.linuxbrew/bin/bun"))
        );
        assert!(
            !OperatingSystem::Windows
                .homebrew_path_matches(Path::new("C:\\Homebrew\\bin\\bun.exe"))
        );
    }
}
