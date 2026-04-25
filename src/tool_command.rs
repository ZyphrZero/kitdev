use anyhow::{Result, bail};

use crate::config::ToolPolicy;

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
    let manager = policy.manager.as_deref().unwrap_or_default();
    let command = match tool {
        "brew" => {
            if install {
                ToolCommand::Shell(homebrew_install_command())
            } else {
                ToolCommand::Shell("brew update && brew upgrade".to_string())
            }
        }
        "fnm" => match manager {
            "" | "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install fnm".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade fnm".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `fnm`"),
        },
        "node" => match manager {
            "" | "fnm" => {
                let version = node_selector(policy.version.as_deref());
                ToolCommand::Shell(format!("fnm install {version} && fnm default {version}"))
            }
            "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install node".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade node".to_string())
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
            "" | "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install oven-sh/bun/bun".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade oven-sh/bun/bun".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `bun`"),
        },
        "deno" => match manager {
            "" | "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install deno".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade deno".to_string())
                }
            }
            "standalone" => {
                if install {
                    ToolCommand::Shell("curl -fsSL https://deno.land/install.sh | sh".to_string())
                } else {
                    ToolCommand::Shell("deno upgrade".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `deno`"),
        },
        "wrangler" => ToolCommand::Shell("npm install -g wrangler@latest".to_string()),
        "uv" => match manager {
            "" | "standalone" => {
                if install {
                    ToolCommand::Shell(
                        "curl -LsSf https://astral.sh/uv/install.sh | sh".to_string(),
                    )
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
            _ => bail!("unsupported manager `{manager}` for `uv`"),
        },
        "python" => match manager {
            "" | "brew" | "homebrew" => {
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
            _ => bail!("unsupported manager `{manager}` for `python`"),
        },
        "poetry" => match manager {
            "" | "brew" | "homebrew" => {
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
                    ToolCommand::Shell(format!(
                        "curl -sSL https://install.python-poetry.org | python3 - --version {version}"
                    ))
                } else {
                    ToolCommand::Shell(
                        "curl -sSL https://install.python-poetry.org | python3 -".to_string(),
                    )
                }
            }
            _ => bail!("unsupported manager `{manager}` for `poetry`"),
        },
        "ruby" => match manager {
            "" | "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install ruby".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade ruby".to_string())
                }
            }
            _ => bail!("unsupported manager `{manager}` for `ruby`"),
        },
        "go" => match policy.source.as_deref().unwrap_or(manager) {
            "" | "brew" | "homebrew" => {
                if install {
                    ToolCommand::Shell("brew install go".to_string())
                } else {
                    ToolCommand::Shell("brew upgrade go".to_string())
                }
            }
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
        _ => bail!("unsupported tool `{tool}`"),
    };

    Ok(command)
}

pub fn command_for_package(package: &str, manager: &str) -> Result<ToolCommand> {
    let command = match manager {
        "brew" | "homebrew" => format!("brew install {package}"),
        "npm" => format!("npm install -g {package}@latest"),
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
