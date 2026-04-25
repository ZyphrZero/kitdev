use anyhow::{Result, bail};

use crate::config::ToolPolicy;
use crate::platform::OperatingSystem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCommand {
    Shell(String),
    Manual(String),
}

impl ToolCommand {
    pub fn into_step_parts(self) -> (Option<String>, bool) {
        match self {
            Self::Shell(command) => (Some(command), false),
            Self::Manual(instruction) => (Some(instruction), true),
        }
    }
}

pub fn command_for_tool(tool: &str, policy: &ToolPolicy, install: bool) -> Result<ToolCommand> {
    command_for_tool_on(tool, policy, install, OperatingSystem::current())
}

pub fn command_for_tool_on(
    tool: &str,
    policy: &ToolPolicy,
    install: bool,
    platform: OperatingSystem,
) -> Result<ToolCommand> {
    let manager = effective_manager(tool, policy, platform);
    let command = match tool {
        "brew" => {
            if platform.is_windows() {
                ToolCommand::Manual("install Homebrew on a Unix-like host from https://brew.sh".to_string())
            } else if install {
                ToolCommand::Shell(homebrew_install_command())
            } else {
                ToolCommand::Shell("brew update && brew upgrade".to_string())
            }
        }
        "winget" => ToolCommand::Manual(
            "install App Installer from Microsoft Store or https://aka.ms/getwinget".to_string(),
        ),
        "fnm" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install fnm".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade fnm".to_string())
                }
            }
            "winget" => winget_tool_command("Schniz.fnm", install),
            "standalone" => match platform {
                OperatingSystem::Windows => ToolCommand::Manual(
                    "install fnm with winget (`winget install --exact --id Schniz.fnm`) or another Windows package manager"
                        .to_string(),
                ),
                _ => ToolCommand::Shell(fnm_install_command(platform)),
            },
            _ => bail!("unsupported manager `{manager}` for `fnm`"),
        },
        "nvm" => match manager {
            "standalone" | "nvm" => match platform {
                OperatingSystem::Windows => ToolCommand::Manual(
                    "install nvm-windows from https://github.com/coreybutler/nvm-windows/releases"
                        .to_string(),
                ),
                _ => ToolCommand::Shell(nvm_install_command()),
            },
            _ => bail!("unsupported manager `{manager}` for `nvm`"),
        },
        "node" => match manager {
            "fnm" => {
                let version = node_selector(policy.version.as_deref());
                ToolCommand::Shell(format!("fnm install {version} && fnm default {version}"))
            }
            "nvm" => {
                let version = nvm_node_selector(policy.version.as_deref());
                ToolCommand::Shell(nvm_node_command(&version))
            }
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install node".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade node".to_string())
                }
            }
            "winget" => {
                if install {
                    ToolCommand::Shell("winget install --exact --id OpenJS.NodeJS.LTS".to_string())
                } else {
                    ToolCommand::Shell("winget upgrade --exact --id OpenJS.NodeJS.LTS".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `node`"),
        },
        "npm" => ToolCommand::Shell("npm install -g npm@latest".to_string()),
        "pnpm" => ToolCommand::Shell(
            "corepack enable && corepack prepare pnpm@latest --activate".to_string(),
        ),
        "yarn" => match manager {
            "" | "corepack" => {
                let version = policy
                    .version
                    .as_deref()
                    .filter(|version| *version != "latest")
                    .unwrap_or("stable");
                ToolCommand::Shell(format!(
                    "corepack enable && corepack prepare yarn@{version} --activate"
                ))
            }
            "npm" => {
                let version = npm_package_selector(policy.version.as_deref());
                ToolCommand::Shell(format!("npm install -g yarn@{version}"))
            }
            _ => bail!("unsupported manager `{manager}` for `yarn`"),
        },
        "bun" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install oven-sh/bun/bun".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade oven-sh/bun/bun".to_string())
                }
            }
            "winget" => winget_tool_command("Oven-sh.Bun", install),
            "standalone" => {
                if install {
                    ToolCommand::Shell(bun_install_command(platform))
                } else {
                    ToolCommand::Shell("bun upgrade".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `bun`"),
        },
        "deno" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install deno".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade deno".to_string())
                }
            }
            "winget" => winget_tool_command("DenoLand.Deno", install),
            "standalone" => {
                if install {
                    ToolCommand::Shell(deno_install_command(platform))
                } else {
                    ToolCommand::Shell("deno upgrade".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `deno`"),
        },
        "wrangler" => ToolCommand::Shell("npm install -g wrangler@latest".to_string()),
        "uv" => match manager {
            "standalone" => {
                if install {
                    ToolCommand::Shell(uv_install_command(platform))
                } else {
                    ToolCommand::Shell("uv self update".to_string())
                }
            }
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install uv".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade uv".to_string())
                }
            }
            "winget" => winget_tool_command("astral-sh.uv", install),
            _ => bail!("unsupported manager `{manager}` for `uv`"),
        },
        "python" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install python".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade python".to_string())
                }
            }
            "uv" => {
                let version = policy.version.as_deref().unwrap_or_default();
                if version.is_empty() || version == "latest" || version == "stable" {
                    ToolCommand::Shell("uv python install".to_string())
                } else {
                    ToolCommand::Shell(format!("uv python install {version}"))
                }
            }
            "winget" => {
                let version = policy
                    .version
                    .as_deref()
                    .filter(|version| *version != "latest" && *version != "stable")
                    .unwrap_or("3");
                let package = format!("Python.Python.{version}");
                winget_tool_command(&package, install)
            }
            _ => bail!("unsupported manager `{manager}` for `python`"),
        },
        "poetry" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install poetry".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade poetry".to_string())
                }
            }
            "official" => {
                let version = policy
                    .version
                    .as_deref()
                    .filter(|version| *version != "latest" && *version != "stable");
                if let Some(version) = version {
                    ToolCommand::Shell(poetry_install_command(platform, Some(version)))
                } else {
                    ToolCommand::Shell(poetry_install_command(platform, None))
                }
            }
            "pipx" => {
                if install {
                    ToolCommand::Shell("pipx install poetry".to_string())
                } else {
                    ToolCommand::Shell("pipx upgrade poetry".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `poetry`"),
        },
        "ruby" => match manager {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install ruby".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade ruby".to_string())
                }
            }
            "manual" => ToolCommand::Manual(
                "install Ruby with the platform package manager or a Ruby version manager"
                    .to_string(),
            ),
            _ => bail!("unsupported manager `{manager}` for `ruby`"),
        },
        "go" => match effective_source(policy, manager, platform) {
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install go".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade go".to_string())
                }
            }
            "winget" => winget_tool_command("GoLang.Go", install),
            "official" => {
                let version = policy
                    .version
                    .clone()
                    .unwrap_or_else(|| "stable".to_string());
                let install_dir = policy
                    .install_dir
                    .clone()
                    .unwrap_or_else(|| "~/.local/opt/go/current".to_string());
                ToolCommand::Manual(format!(
                    "install Go {version} from https://go.dev/dl/ into {install_dir}"
                ))
            }
            source => bail!("unsupported source `{source}` for `go`"),
        },
        "rust" | "rustup" | "rustc" | "cargo" => match manager {
            "rustup" => {
                if install {
                    ToolCommand::Shell(rustup_install_command(platform))
                } else {
                    let channel = policy
                        .channel
                        .as_deref()
                        .or(policy.version.as_deref())
                        .unwrap_or("stable");
                    ToolCommand::Shell(format!("rustup self update && rustup update {channel}"))
                }
            }
            _ => bail!("unsupported manager `{manager}` for `{tool}`"),
        },
        _ => bail!("unsupported tool `{tool}`"),
    };

    Ok(command)
}

pub fn command_for_package(package: &str, manager: &str) -> Result<ToolCommand> {
    let command = match manager {
        "brew" | "homebrew" => format!("brew install {package}"),
        "npm" => format!("npm install -g {package}@latest"),
        "apt" | "apt-get" => format!("sudo apt-get update && sudo apt-get install -y {package}"),
        "dnf" => format!("sudo dnf install -y {package}"),
        "pacman" => format!("sudo pacman -S --needed {package}"),
        "zypper" => format!("sudo zypper install -y {package}"),
        "apk" => format!("sudo apk add {package}"),
        "winget" => format!("winget install --exact --id {package}"),
        "scoop" => format!("scoop install {package}"),
        "choco" | "chocolatey" => format!("choco install {package} -y"),
        _ => bail!("unsupported package manager `{manager}` for `{package}`"),
    };

    Ok(ToolCommand::Shell(command))
}

pub fn homebrew_install_command() -> String {
    "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
        .to_string()
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

fn npm_package_selector(version: Option<&str>) -> String {
    match version {
        Some("stable") | Some("latest") | None => "latest".to_string(),
        Some(version) => version.to_string(),
    }
}

fn nvm_node_selector(version: Option<&str>) -> String {
    match version {
        Some("latest") => "node".to_string(),
        Some("stable") | None => "--lts".to_string(),
        Some(version) => version
            .trim_start_matches('v')
            .trim_end_matches(".x")
            .to_string(),
    }
}

fn effective_manager<'a>(tool: &str, policy: &'a ToolPolicy, platform: OperatingSystem) -> &'a str {
    policy
        .manager
        .as_deref()
        .filter(|manager| !manager.is_empty())
        .or_else(|| policy.source.as_deref().filter(|source| !source.is_empty()))
        .or_else(|| platform.default_manager_for(tool))
        .unwrap_or_default()
}

fn effective_source<'a>(
    policy: &'a ToolPolicy,
    manager: &'a str,
    platform: OperatingSystem,
) -> &'a str {
    policy
        .source
        .as_deref()
        .filter(|source| !source.is_empty())
        .unwrap_or_else(|| {
            if manager.is_empty() {
                platform.default_go_source()
            } else {
                manager
            }
        })
}

fn winget_tool_command(package_id: &str, install: bool) -> ToolCommand {
    let action = if install { "install" } else { "upgrade" };
    ToolCommand::Shell(format!("winget {action} --exact --id {package_id}"))
}

fn fnm_install_command(platform: OperatingSystem) -> String {
    match platform {
        OperatingSystem::Windows => "winget install --exact --id Schniz.fnm".to_string(),
        _ => "curl -fsSL https://fnm.vercel.app/install | bash".to_string(),
    }
}

fn nvm_install_command() -> String {
    "curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash".to_string()
}

fn nvm_node_command(version: &str) -> String {
    format!(
        "export NVM_DIR=\"$HOME/.nvm\" && [ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" && nvm install {version} && nvm alias default {version} && nvm use default"
    )
}

fn bun_install_command(platform: OperatingSystem) -> String {
    match platform {
        OperatingSystem::Windows => powershell_command("irm https://bun.sh/install.ps1 | iex"),
        _ => "curl -fsSL https://bun.sh/install | bash".to_string(),
    }
}

fn deno_install_command(platform: OperatingSystem) -> String {
    match platform {
        OperatingSystem::Windows => powershell_command("irm https://deno.land/install.ps1 | iex"),
        _ => "curl -fsSL https://deno.land/install.sh | sh".to_string(),
    }
}

pub fn uv_install_command(platform: OperatingSystem) -> String {
    match platform {
        OperatingSystem::Windows => {
            powershell_command("irm https://astral.sh/uv/install.ps1 | iex")
        }
        _ => "curl -LsSf https://astral.sh/uv/install.sh | sh".to_string(),
    }
}

pub fn rustup_install_command(platform: OperatingSystem) -> String {
    match platform {
        OperatingSystem::Windows => "winget install --exact --id Rustlang.Rustup".to_string(),
        _ => "curl https://sh.rustup.rs -sSf | sh".to_string(),
    }
}

fn poetry_install_command(platform: OperatingSystem, version: Option<&str>) -> String {
    let version_arg = version
        .map(|version| format!(" --version {version}"))
        .unwrap_or_default();
    match platform {
        OperatingSystem::Windows => powershell_command(&format!(
            "(Invoke-WebRequest -Uri https://install.python-poetry.org -UseBasicParsing).Content | py -{version_arg}"
        )),
        _ => format!("curl -sSL https://install.python-poetry.org | python3 -{version_arg}"),
    }
}

fn powershell_command(script: &str) -> String {
    format!("powershell -NoProfile -ExecutionPolicy Bypass -Command \"{script}\"")
}

#[cfg(test)]
mod tests {
    use crate::{config::ToolPolicy, platform::OperatingSystem};

    use super::{ToolCommand, command_for_package, command_for_tool_on};

    #[test]
    fn uses_homebrew_defaults_on_macos() {
        assert_eq!(
            command_for_tool_on("bun", &ToolPolicy::default(), true, OperatingSystem::Macos)
                .unwrap(),
            ToolCommand::Shell("brew install oven-sh/bun/bun".to_string())
        );
    }

    #[test]
    fn uses_standalone_defaults_on_linux() {
        assert_eq!(
            command_for_tool_on("bun", &ToolPolicy::default(), true, OperatingSystem::Linux)
                .unwrap(),
            ToolCommand::Shell("curl -fsSL https://bun.sh/install | bash".to_string())
        );
        assert_eq!(
            command_for_tool_on(
                "python",
                &ToolPolicy::default(),
                true,
                OperatingSystem::Linux
            )
            .unwrap(),
            ToolCommand::Shell("uv python install".to_string())
        );
    }

    #[test]
    fn uses_windows_package_manager_defaults() {
        assert_eq!(
            command_for_tool_on(
                "fnm",
                &ToolPolicy::default(),
                true,
                OperatingSystem::Windows
            )
            .unwrap(),
            ToolCommand::Shell("winget install --exact --id Schniz.fnm".to_string())
        );
    }

    #[test]
    fn supports_nvm_managed_node() {
        let policy = ToolPolicy {
            version: Some("24.x".to_string()),
            manager: Some("nvm".to_string()),
            ..ToolPolicy::default()
        };
        let command = command_for_tool_on("node", &policy, true, OperatingSystem::Macos).unwrap();

        match command {
            ToolCommand::Shell(command) => {
                assert!(command.contains("nvm install 24"));
                assert!(command.contains("nvm alias default 24"));
            }
            command => panic!("unexpected command: {command:?}"),
        }
    }

    #[test]
    fn supports_platform_package_managers_for_cli_packages() {
        assert_eq!(
            command_for_package("GitHub.cli", "winget").unwrap(),
            ToolCommand::Shell("winget install --exact --id GitHub.cli".to_string())
        );
        assert_eq!(
            command_for_package("gh", "apt").unwrap(),
            ToolCommand::Shell("sudo apt-get update && sudo apt-get install -y gh".to_string())
        );
        assert_eq!(
            command_for_package("gh", "apk").unwrap(),
            ToolCommand::Shell("sudo apk add gh".to_string())
        );
    }
}
