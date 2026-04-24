use crate::shell::shell_output;

#[derive(Debug, Clone)]
pub struct LatestVersion {
    pub version: Option<String>,
    pub source: &'static str,
    pub note: Option<String>,
}

pub fn lookup_latest(tool: &str) -> LatestVersion {
    match tool {
        "npm" | "pnpm" | "wrangler" => npm_latest(tool),
        "bun" | "fnm" | "brew" => brew_latest(tool),
        "node" => node_latest(),
        "go" => go_latest(),
        "uv" => uv_latest(),
        "rustup" => rustup_latest(),
        "rustc" | "cargo" => rust_toolchain_latest(tool),
        _ => LatestVersion {
            version: None,
            source: "unsupported",
            note: Some("no latest-version provider registered".to_string()),
        },
    }
}

fn npm_latest(package: &str) -> LatestVersion {
    LatestVersion {
        version: shell_output(&format!("npm view {package} version")),
        source: "npm",
        note: None,
    }
}

fn brew_latest(formula: &str) -> LatestVersion {
    let query = format!(
        "brew info --json=v2 {formula} | ruby -rjson -e 'j=JSON.parse(STDIN.read); f=j[\"formulae\"][0]; puts f[\"versions\"][\"stable\"]'"
    );

    LatestVersion {
        version: shell_output(&query),
        source: "homebrew",
        note: None,
    }
}

fn node_latest() -> LatestVersion {
    LatestVersion {
        version: shell_output("fnm list-remote | tail -n 1 | sed 's/^v//'"),
        source: "fnm remote",
        note: Some("reports latest Node release, not necessarily LTS".to_string()),
    }
}

fn go_latest() -> LatestVersion {
    LatestVersion {
        version: shell_output(
            "curl -fsSL 'https://go.dev/VERSION?m=text' | head -n1 | sed 's/^go//'",
        ),
        source: "go.dev",
        note: None,
    }
}

fn uv_latest() -> LatestVersion {
    LatestVersion {
        version: shell_output(
            "curl -fsSL -A 'Mozilla/5.0' https://github.com/astral-sh/uv/releases/latest | ruby -e 'html=STDIN.read; m=html.match(%r{/astral-sh/uv/releases/tag/([^\"]+)}); puts((m && m[1].sub(/^v/, \"\")) || \"\")'",
        )
        .filter(|version| !version.is_empty()),
        source: "github releases",
        note: None,
    }
}

fn rustup_latest() -> LatestVersion {
    LatestVersion {
        version: shell_output("rustup check | awk '/^rustup / { print $NF }'").filter(|version| {
            version
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        }),
        source: "rustup check",
        note: Some("reports the rustup toolchain manager version".to_string()),
    }
}

fn rust_toolchain_latest(tool: &str) -> LatestVersion {
    let version = shell_output("rustup check")
        .and_then(|output| {
            output
                .lines()
                .find(|line| line.contains("stable-"))
                .map(str::to_string)
        })
        .and_then(|line| line.split(':').nth(1).map(str::trim).map(str::to_string))
        .and_then(|value| value.split_whitespace().next().map(str::to_string));

    LatestVersion {
        version,
        source: "rustup check",
        note: Some(format!("{tool} follows the active Rust stable toolchain")),
    }
}
