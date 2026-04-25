use std::process::{Command, Stdio};

use crate::shell::shell_output;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct LatestVersion {
    pub version: Option<String>,
    pub source: &'static str,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VersionCandidates {
    pub candidates: Vec<VersionCandidate>,
    pub source: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCandidate {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
}

pub fn lookup_latest(tool: &str) -> LatestVersion {
    match tool {
        "npm" | "pnpm" | "yarn" | "wrangler" => npm_latest(tool),
        "bun" | "deno" | "fnm" | "brew" | "python" | "poetry" | "ruby" => brew_latest(tool),
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

pub fn lookup_version_candidates(tool: &str) -> VersionCandidates {
    match tool {
        "node" => node_version_candidates(),
        "go" => go_version_candidates(),
        "rust" | "rustc" | "cargo" => rust_channel_candidates(),
        _ => generic_version_candidates(tool),
    }
}

fn npm_latest(package: &str) -> LatestVersion {
    LatestVersion {
        version: shell_output(&format!("npm view {package} version")),
        source: "npm",
        note: None,
    }
}

fn node_version_candidates() -> VersionCandidates {
    let versions = command_output(
        "curl",
        &[
            "-fsSL",
            "--max-time",
            "8",
            "https://nodejs.org/dist/index.json",
        ],
    )
    .and_then(|output| parse_node_release_json(&output).ok())
    .or_else(|| command_output("fnm", &["list-remote"]).map(|output| parse_version_lines(&output)))
    .unwrap_or_default();

    let mut candidates = vec![
        candidate("Stable LTS", "stable", "devkit uses fnm lts-latest"),
        candidate("Latest release", "latest", "latest Node release"),
    ];
    extend_major_candidates(&mut candidates, "Node", &versions, 8);
    extend_exact_candidates(&mut candidates, "Node", &versions, 12);

    VersionCandidates {
        candidates: dedupe_candidates(candidates),
        source: "nodejs.org dist index".to_string(),
        note: versions.is_empty().then(|| {
            "could not fetch remote Node versions; choose latest/stable or enter a custom selector"
                .to_string()
        }),
    }
}

fn go_version_candidates() -> VersionCandidates {
    let versions = command_output(
        "curl",
        &[
            "-fsSL",
            "--max-time",
            "8",
            "https://go.dev/dl/?mode=json&include=all",
        ],
    )
    .and_then(|output| parse_go_release_json(&output).ok())
    .unwrap_or_default();

    let mut candidates = vec![candidate(
        "Latest stable",
        "latest",
        "policy selector accepted by devkit",
    )];
    extend_minor_candidates(&mut candidates, "Go", &versions, 8);
    extend_exact_candidates(&mut candidates, "Go", &versions, 12);

    VersionCandidates {
        candidates: dedupe_candidates(candidates),
        source: "go.dev download index".to_string(),
        note: versions.is_empty().then(|| {
            "could not fetch remote Go versions; choose latest or enter a custom selector"
                .to_string()
        }),
    }
}

fn rust_channel_candidates() -> VersionCandidates {
    VersionCandidates {
        candidates: vec![
            candidate("Stable channel", "stable", "recommended Rust channel"),
            candidate("Beta channel", "beta", "Rust beta channel"),
            candidate("Nightly channel", "nightly", "Rust nightly channel"),
        ],
        source: "rustup channels".to_string(),
        note: None,
    }
}

fn generic_version_candidates(tool: &str) -> VersionCandidates {
    VersionCandidates {
        candidates: vec![
            candidate("Latest", "latest", "generic policy selector"),
            candidate("Stable", "stable", "generic policy selector"),
        ],
        source: format!("{tool} defaults"),
        note: Some("no remote version-list provider registered for this tool".to_string()),
    }
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!stdout.is_empty()).then_some(stdout)
}

fn candidate(label: &str, value: &str, note: &str) -> VersionCandidate {
    VersionCandidate {
        label: label.to_string(),
        value: value.to_string(),
        note: (!note.is_empty()).then(|| note.to_string()),
    }
}

fn extend_major_candidates(
    candidates: &mut Vec<VersionCandidate>,
    tool_label: &str,
    versions: &[String],
    limit: usize,
) {
    let mut seen = Vec::<u64>::new();
    for version in versions {
        let Some((major, _, _)) = semver_tuple(version) else {
            continue;
        };
        if seen.contains(&major) {
            continue;
        }
        seen.push(major);
        candidates.push(VersionCandidate {
            label: format!("{tool_label} {major}.x"),
            value: format!("{major}.x"),
            note: Some(format!("latest matching release: {version}")),
        });
        if seen.len() >= limit {
            break;
        }
    }
}

fn extend_minor_candidates(
    candidates: &mut Vec<VersionCandidate>,
    tool_label: &str,
    versions: &[String],
    limit: usize,
) {
    let mut seen = Vec::<(u64, u64)>::new();
    for version in versions {
        let Some((major, minor, _)) = semver_tuple(version) else {
            continue;
        };
        if seen.contains(&(major, minor)) {
            continue;
        }
        seen.push((major, minor));
        candidates.push(VersionCandidate {
            label: format!("{tool_label} {major}.{minor}.x"),
            value: format!("{major}.{minor}.x"),
            note: Some(format!("latest matching release: {version}")),
        });
        if seen.len() >= limit {
            break;
        }
    }
}

fn extend_exact_candidates(
    candidates: &mut Vec<VersionCandidate>,
    tool_label: &str,
    versions: &[String],
    limit: usize,
) {
    candidates.extend(versions.iter().take(limit).map(|version| VersionCandidate {
        label: format!("{tool_label} {version}"),
        value: version.clone(),
        note: Some("exact version".to_string()),
    }));
}

fn dedupe_candidates(candidates: Vec<VersionCandidate>) -> Vec<VersionCandidate> {
    let mut values = Vec::<String>::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            if values.contains(&candidate.value) {
                false
            } else {
                values.push(candidate.value.clone());
                true
            }
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct NodeRelease {
    version: String,
}

#[derive(Debug, Deserialize)]
struct GoRelease {
    version: String,
}

fn parse_node_release_json(raw: &str) -> Result<Vec<String>, serde_json::Error> {
    let releases = serde_json::from_str::<Vec<NodeRelease>>(raw)?;
    Ok(sort_versions_desc(
        releases
            .into_iter()
            .filter_map(|release| normalize_candidate_version(&release.version))
            .collect(),
    ))
}

fn parse_go_release_json(raw: &str) -> Result<Vec<String>, serde_json::Error> {
    let releases = serde_json::from_str::<Vec<GoRelease>>(raw)?;
    Ok(sort_versions_desc(
        releases
            .into_iter()
            .filter_map(|release| normalize_candidate_version(&release.version))
            .collect(),
    ))
}

fn parse_version_lines(raw: &str) -> Vec<String> {
    sort_versions_desc(
        raw.lines()
            .filter_map(normalize_candidate_version)
            .collect(),
    )
}

fn normalize_candidate_version(raw: &str) -> Option<String> {
    let version = raw.trim().trim_start_matches('v').trim_start_matches("go");
    version
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
        .then(|| version.to_string())
}

fn sort_versions_desc(mut versions: Vec<String>) -> Vec<String> {
    versions.sort_by(|left, right| {
        semver_tuple(right)
            .cmp(&semver_tuple(left))
            .then_with(|| right.cmp(left))
    });
    versions
}

fn semver_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
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

#[cfg(test)]
mod tests {
    use super::{
        VersionCandidate, dedupe_candidates, extend_major_candidates, extend_minor_candidates,
        normalize_candidate_version, parse_go_release_json, parse_node_release_json,
        sort_versions_desc,
    };

    #[test]
    fn parses_and_sorts_node_release_json() {
        let versions = parse_node_release_json(
            r#"[{"version":"v24.2.0"},{"version":"v25.0.1"},{"version":"v24.10.0"}]"#,
        )
        .unwrap();

        assert_eq!(versions, vec!["25.0.1", "24.10.0", "24.2.0"]);
    }

    #[test]
    fn parses_go_release_json() {
        let versions =
            parse_go_release_json(r#"[{"version":"go1.26.2"},{"version":"go1.25.5"}]"#).unwrap();

        assert_eq!(versions, vec!["1.26.2", "1.25.5"]);
    }

    #[test]
    fn builds_major_and_minor_policy_selectors() {
        let versions = vec![
            "25.1.0".to_string(),
            "24.11.1".to_string(),
            "24.10.0".to_string(),
        ];
        let mut node = Vec::new();
        extend_major_candidates(&mut node, "Node", &versions, 4);
        assert_eq!(node[0].value, "25.x");
        assert_eq!(node[1].value, "24.x");

        let mut go = Vec::new();
        extend_minor_candidates(
            &mut go,
            "Go",
            &["1.26.2".to_string(), "1.26.1".to_string()],
            4,
        );
        assert_eq!(go[0].value, "1.26.x");
    }

    #[test]
    fn normalizes_version_prefixes_and_dedupes() {
        assert_eq!(
            normalize_candidate_version("v24.1.0").as_deref(),
            Some("24.1.0")
        );
        assert_eq!(
            normalize_candidate_version("go1.26.2").as_deref(),
            Some("1.26.2")
        );
        let candidates = dedupe_candidates(vec![
            VersionCandidate {
                label: "a".to_string(),
                value: "24.x".to_string(),
                note: None,
            },
            VersionCandidate {
                label: "b".to_string(),
                value: "24.x".to_string(),
                note: None,
            },
        ]);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn sorts_semver_descending() {
        assert_eq!(
            sort_versions_desc(vec![
                "24.2.0".to_string(),
                "24.11.0".to_string(),
                "25.0.0".to_string()
            ]),
            vec!["25.0.0", "24.11.0", "24.2.0"]
        );
    }
}
