use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    package_manager::detected_cli_manager,
    platform::{OperatingSystem, current_platform_tag},
};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct DevkitConfig {
    pub policy: Option<PolicyConfig>,
    pub tools: Option<BTreeMap<String, ToolPolicy>>,
    pub homebrew: Option<HomebrewConfig>,
    pub npm: Option<NpmConfig>,
    pub platform: Option<BTreeMap<String, PlatformOverride>>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PlatformOverride {
    pub policy: Option<PolicyConfig>,
    pub tools: Option<BTreeMap<String, ToolPolicy>>,
    pub homebrew: Option<HomebrewConfig>,
    pub npm: Option<NpmConfig>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PolicyConfig {
    pub channel: Option<String>,
    pub auto_fix: Option<bool>,
    pub mirror: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ToolPolicy {
    pub version: Option<String>,
    pub manager: Option<String>,
    pub source: Option<String>,
    pub channel: Option<String>,
    pub components: Option<Vec<String>>,
    pub package_managers: Option<Vec<String>>,
    pub packages: Option<Vec<String>>,
    pub install_dir: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct HomebrewConfig {
    pub mirror: Option<String>,
    pub packages: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct NpmConfig {
    pub global_packages: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ConfigValidationReport {
    pub ok: bool,
    pub items: Vec<ConfigValidationItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigValidationItem {
    pub level: ConfigValidationLevel,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigValidationLevel {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigExplainReport {
    pub platform: String,
    pub applied_overrides: Vec<String>,
    pub entries: Vec<ConfigExplainEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigExplainEntry {
    pub path: String,
    pub source: String,
    pub value: String,
}

impl DevkitConfig {
    pub fn read(path: &Path) -> Result<Self> {
        Self::read_raw(path).map(Self::effective_for_current_platform)
    }

    pub fn read_raw(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("failed to parse config {}", path.display()))
    }

    pub fn effective_for_current_platform(self) -> Self {
        self.effective_for(OperatingSystem::current(), &current_platform_tag())
    }

    pub fn effective_for(mut self, os: OperatingSystem, platform_tag: &str) -> Self {
        let overrides = self.platform.take();
        let Some(overrides) = overrides else {
            return self;
        };

        if let Some(override_config) = overrides.get(os.label()) {
            self.merge_platform_override(override_config);
        }
        if let Some(override_config) = overrides.get(platform_tag) {
            self.merge_platform_override(override_config);
        }

        self
    }

    fn merge_platform_override(&mut self, override_config: &PlatformOverride) {
        merge_option(&mut self.policy, &override_config.policy, merge_policy);
        merge_map(&mut self.tools, &override_config.tools, merge_tool_policy);
        merge_option(
            &mut self.homebrew,
            &override_config.homebrew,
            merge_homebrew,
        );
        merge_option(&mut self.npm, &override_config.npm, merge_npm);
    }

    pub fn validate_for_current_platform(&self) -> ConfigValidationReport {
        let mut report = ConfigValidationReport {
            ok: true,
            items: Vec::new(),
        };

        validate_platform_override_keys(self, &mut report);
        validate_config_section("", self, OperatingSystem::current(), false, &mut report);

        if let Some(overrides) = &self.platform {
            for (platform_key, override_config) in overrides {
                let Some(platform) = override_platform(platform_key) else {
                    continue;
                };
                validate_override_section(platform_key, override_config, platform, &mut report);
            }
        }

        let effective = self.clone().effective_for_current_platform();
        validate_config_section(
            "",
            &effective,
            OperatingSystem::current(),
            true,
            &mut report,
        );
        report.ok = !report.has_errors();
        report
    }

    pub fn explain_for_current_platform(&self) -> ConfigExplainReport {
        self.explain_for(OperatingSystem::current(), &current_platform_tag())
    }

    pub fn explain_for(&self, os: OperatingSystem, platform_tag: &str) -> ConfigExplainReport {
        let mut entries = BTreeMap::<String, ConfigExplainEntry>::new();
        collect_config_entries(self, "base", &mut entries);

        let mut applied_overrides = Vec::new();
        if let Some(overrides) = &self.platform {
            if let Some(override_config) = overrides.get(os.label()) {
                let source = format!("platform.{}", os.label());
                applied_overrides.push(source.clone());
                collect_override_entries(override_config, &source, &mut entries);
            }
            if let Some(override_config) = overrides.get(platform_tag) {
                let source = format!("platform.{platform_tag}");
                applied_overrides.push(source.clone());
                collect_override_entries(override_config, &source, &mut entries);
            }
        }

        ConfigExplainReport {
            platform: platform_tag.to_string(),
            applied_overrides,
            entries: entries.into_values().collect(),
        }
    }
}

impl ConfigValidationReport {
    fn add(&mut self, level: ConfigValidationLevel, path: impl Into<String>, message: String) {
        let item = ConfigValidationItem {
            level,
            path: path.into(),
            message,
        };
        if !self.items.contains(&item) {
            self.items.push(item);
        }
    }

    pub fn has_errors(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.level, ConfigValidationLevel::Error))
    }
}

fn merge_option<T: Clone>(
    base: &mut Option<T>,
    override_value: &Option<T>,
    merge: impl FnOnce(&mut T, &T),
) {
    if let Some(override_value) = override_value {
        if let Some(base) = base {
            merge(base, override_value);
        } else {
            *base = Some(override_value.clone());
        }
    }
}

fn merge_map<T: Clone>(
    base: &mut Option<BTreeMap<String, T>>,
    override_value: &Option<BTreeMap<String, T>>,
    merge: impl Fn(&mut T, &T),
) {
    let Some(override_value) = override_value else {
        return;
    };
    let base = base.get_or_insert_with(BTreeMap::new);
    for (key, override_item) in override_value {
        if let Some(base_item) = base.get_mut(key) {
            merge(base_item, override_item);
        } else {
            base.insert(key.clone(), override_item.clone());
        }
    }
}

fn merge_policy(base: &mut PolicyConfig, override_value: &PolicyConfig) {
    replace_if_some(&mut base.channel, &override_value.channel);
    replace_if_some(&mut base.auto_fix, &override_value.auto_fix);
    replace_if_some(&mut base.mirror, &override_value.mirror);
    replace_if_some(&mut base.platform, &override_value.platform);
}

fn merge_tool_policy(base: &mut ToolPolicy, override_value: &ToolPolicy) {
    replace_if_some(&mut base.version, &override_value.version);
    replace_if_some(&mut base.manager, &override_value.manager);
    replace_if_some(&mut base.source, &override_value.source);
    replace_if_some(&mut base.channel, &override_value.channel);
    replace_if_some(&mut base.components, &override_value.components);
    replace_if_some(&mut base.package_managers, &override_value.package_managers);
    replace_if_some(&mut base.packages, &override_value.packages);
    replace_if_some(&mut base.install_dir, &override_value.install_dir);
}

fn merge_homebrew(base: &mut HomebrewConfig, override_value: &HomebrewConfig) {
    replace_if_some(&mut base.mirror, &override_value.mirror);
    replace_if_some(&mut base.packages, &override_value.packages);
}

fn merge_npm(base: &mut NpmConfig, override_value: &NpmConfig) {
    replace_if_some(&mut base.global_packages, &override_value.global_packages);
}

fn replace_if_some<T: Clone>(base: &mut Option<T>, override_value: &Option<T>) {
    if let Some(override_value) = override_value {
        *base = Some(override_value.clone());
    }
}

fn validate_platform_override_keys(config: &DevkitConfig, report: &mut ConfigValidationReport) {
    let Some(overrides) = &config.platform else {
        return;
    };

    for key in overrides.keys() {
        if override_platform(key).is_none() {
            report.add(
                ConfigValidationLevel::Error,
                format!("platform.{key}"),
                format!(
                    "unknown platform override `{key}`; use macos, linux, windows, or a tag like macos-arm64"
                ),
            );
        }
    }
}

fn validate_config_section(
    prefix: &str,
    config: &DevkitConfig,
    platform: OperatingSystem,
    check_runtime: bool,
    report: &mut ConfigValidationReport,
) {
    if let Some(policy) = &config.policy {
        validate_policy(prefix, policy, Some(platform), report);
    }
    if let Some(tools) = &config.tools {
        validate_tools(prefix, tools, platform, check_runtime, report);
    }
    if let Some(homebrew) = &config.homebrew {
        validate_homebrew(prefix, homebrew, platform, report);
    }
    if let Some(npm) = &config.npm {
        validate_npm(prefix, npm, report);
    }
}

fn validate_override_section(
    platform_key: &str,
    override_config: &PlatformOverride,
    platform: OperatingSystem,
    report: &mut ConfigValidationReport,
) {
    let prefix = format!("platform.{platform_key}");
    if let Some(policy) = &override_config.policy {
        validate_policy(&prefix, policy, Some(platform), report);
    }
    if let Some(tools) = &override_config.tools {
        validate_tools(&prefix, tools, platform, false, report);
    }
    if let Some(homebrew) = &override_config.homebrew {
        validate_homebrew(&prefix, homebrew, platform, report);
    }
    if let Some(npm) = &override_config.npm {
        validate_npm(&prefix, npm, report);
    }
}

fn validate_policy(
    prefix: &str,
    policy: &PolicyConfig,
    expected_platform: Option<OperatingSystem>,
    report: &mut ConfigValidationReport,
) {
    if let Some(platform_tag) = policy.platform.as_deref() {
        let path = section_path(prefix, "policy.platform");
        match OperatingSystem::from_platform_tag(platform_tag) {
            Some(policy_platform) => {
                if let Some(expected_platform) = expected_platform
                    && policy_platform != expected_platform
                {
                    report.add(
                        ConfigValidationLevel::Error,
                        path,
                        format!(
                            "policy platform `{platform_tag}` does not match `{}` override context",
                            expected_platform.label()
                        ),
                    );
                }
            }
            None => report.add(
                ConfigValidationLevel::Error,
                path,
                format!(
                    "unknown policy platform `{platform_tag}`; use macos, linux, windows, or a tag like linux-x86_64"
                ),
            ),
        }
    }
}

fn validate_tools(
    prefix: &str,
    tools: &BTreeMap<String, ToolPolicy>,
    platform: OperatingSystem,
    check_runtime: bool,
    report: &mut ConfigValidationReport,
) {
    for (name, policy) in tools {
        let tool_path = section_path(prefix, &format!("tools.{name}"));
        if name == "cli" {
            validate_cli_policy(prefix, policy, platform, check_runtime, report);
            continue;
        }

        if !known_tool(name) {
            report.add(
                ConfigValidationLevel::Error,
                tool_path.clone(),
                format!(
                    "`{name}` is not inspected by devkit; list CLI packages under [tools.cli], [homebrew], or [npm]"
                ),
            );
            continue;
        }

        validate_optional_string(
            report,
            section_path(prefix, &format!("tools.{name}.version")),
            "version",
            policy.version.as_deref(),
        );
        validate_optional_string(
            report,
            section_path(prefix, &format!("tools.{name}.channel")),
            "channel",
            policy.channel.as_deref(),
        );

        if let Some(manager) = policy.manager.as_deref() {
            validate_tool_manager(prefix, name, "manager", manager, platform, report);
        }
        if let Some(source) = policy.source.as_deref() {
            validate_tool_manager(prefix, name, "source", source, platform, report);
        }

        if let Some(packages) = policy.packages.as_ref() {
            validate_package_list(
                report,
                section_path(prefix, &format!("tools.{name}.packages")),
                Some(packages),
            );
            validate_packages_manager(prefix, name, policy, platform, check_runtime, report);
        }

        if let Some(package_managers) = policy.package_managers.as_ref() {
            validate_package_managers(prefix, name, package_managers, report);
        }

        if policy.install_dir.is_some() && name != "go" {
            report.add(
                ConfigValidationLevel::Warning,
                section_path(prefix, &format!("tools.{name}.install_dir")),
                "`install_dir` is only used by the official Go installer".to_string(),
            );
        }
        if name == "go"
            && policy
                .source
                .as_deref()
                .or(policy.manager.as_deref())
                .is_some_and(|source| canonical_manager(source) == "official")
            && policy.install_dir.is_none()
        {
            report.add(
                ConfigValidationLevel::Warning,
                section_path(prefix, "tools.go.install_dir"),
                "official Go installs will use the default ~/.local/opt/go/current path"
                    .to_string(),
            );
        }
    }
}

fn validate_cli_policy(
    prefix: &str,
    policy: &ToolPolicy,
    platform: OperatingSystem,
    check_runtime: bool,
    report: &mut ConfigValidationReport,
) {
    if let Some(manager) = policy.manager.as_deref() {
        validate_package_manager(prefix, "tools.cli.manager", manager, report);
    }
    if let Some(source) = policy.source.as_deref() {
        validate_package_manager(prefix, "tools.cli.source", source, report);
    }

    let packages = policy.packages.as_deref().unwrap_or_default();
    validate_package_list(
        report,
        section_path(prefix, "tools.cli.packages"),
        policy.packages.as_deref(),
    );
    if packages.is_empty() {
        return;
    }

    let has_manager = policy
        .manager
        .as_deref()
        .or(policy.source.as_deref())
        .is_some_and(|manager| !manager.trim().is_empty());
    if !has_manager
        && check_runtime
        && detected_cli_manager().is_none()
        && platform.default_manager_for("cli").is_none()
    {
        report.add(
            ConfigValidationLevel::Warning,
            section_path(prefix, "tools.cli.manager"),
            "CLI packages are configured without a manager and no platform manager was detected"
                .to_string(),
        );
    }
}

fn validate_tool_manager(
    prefix: &str,
    tool: &str,
    field: &str,
    manager: &str,
    platform: OperatingSystem,
    report: &mut ConfigValidationReport,
) {
    let path = section_path(prefix, &format!("tools.{tool}.{field}"));
    validate_optional_string(report, path.clone(), field, Some(manager));
    if manager.trim().is_empty() {
        return;
    }
    if !known_manager(manager) {
        report.add(
            ConfigValidationLevel::Error,
            path,
            format!("unknown {field} `{manager}` for `{tool}`"),
        );
        return;
    }
    if !manager_supported_for_tool(tool, manager, platform) {
        report.add(
            ConfigValidationLevel::Error,
            path,
            format!(
                "unsupported {field} `{manager}` for `{tool}` on {}",
                platform.label()
            ),
        );
    }
}

fn validate_package_manager(
    prefix: &str,
    path: &str,
    manager: &str,
    report: &mut ConfigValidationReport,
) {
    let path = section_path(prefix, path);
    validate_optional_string(report, path.clone(), "manager", Some(manager));
    if manager.trim().is_empty() {
        return;
    }
    if !cli_package_manager(manager) {
        report.add(
            ConfigValidationLevel::Error,
            path,
            format!("unsupported CLI package manager `{manager}`"),
        );
    }
}

fn validate_packages_manager(
    prefix: &str,
    tool: &str,
    policy: &ToolPolicy,
    platform: OperatingSystem,
    check_runtime: bool,
    report: &mut ConfigValidationReport,
) {
    let manager = policy
        .manager
        .as_deref()
        .or(policy.source.as_deref())
        .or_else(|| platform.default_manager_for("cli"));

    match manager {
        Some(manager) if cli_package_manager(manager) => {}
        Some(manager) => report.add(
            ConfigValidationLevel::Warning,
            section_path(prefix, &format!("tools.{tool}.packages")),
            format!(
                "`packages` under `{tool}` would use `{manager}`, which is not a CLI package manager; prefer [tools.cli], [homebrew], or [npm]"
            ),
        ),
        None if check_runtime && detected_cli_manager().is_none() => report.add(
            ConfigValidationLevel::Warning,
            section_path(prefix, &format!("tools.{tool}.packages")),
            "packages are configured without a detected package manager".to_string(),
        ),
        None => {}
    }
}

fn validate_package_managers(
    prefix: &str,
    tool: &str,
    package_managers: &[String],
    report: &mut ConfigValidationReport,
) {
    let path = section_path(prefix, &format!("tools.{tool}.package_managers"));
    validate_package_list(report, path.clone(), Some(package_managers));
    if tool != "node" {
        report.add(
            ConfigValidationLevel::Warning,
            path.clone(),
            "`package_managers` is only used by the Node workflow".to_string(),
        );
    }
    for manager in package_managers {
        if !matches!(manager.as_str(), "npm" | "pnpm" | "yarn" | "bun") {
            report.add(
                ConfigValidationLevel::Error,
                path.clone(),
                format!("unsupported Node package manager `{manager}`"),
            );
        }
    }
}

fn validate_homebrew(
    prefix: &str,
    homebrew: &HomebrewConfig,
    platform: OperatingSystem,
    report: &mut ConfigValidationReport,
) {
    validate_package_list(
        report,
        section_path(prefix, "homebrew.packages"),
        homebrew.packages.as_deref(),
    );
    if platform.is_windows()
        && homebrew
            .packages
            .as_deref()
            .is_some_and(|packages| !packages.is_empty())
    {
        report.add(
            ConfigValidationLevel::Warning,
            section_path(prefix, "homebrew.packages"),
            "Homebrew packages are not applied by the Windows default workflow".to_string(),
        );
    }
}

fn validate_npm(prefix: &str, npm: &NpmConfig, report: &mut ConfigValidationReport) {
    validate_package_list(
        report,
        section_path(prefix, "npm.global_packages"),
        npm.global_packages.as_deref(),
    );
}

fn validate_optional_string(
    report: &mut ConfigValidationReport,
    path: String,
    field: &str,
    value: Option<&str>,
) {
    if value.is_some_and(|value| value.trim().is_empty()) {
        report.add(
            ConfigValidationLevel::Warning,
            path,
            format!("empty `{field}` is ignored"),
        );
    }
}

fn validate_package_list(
    report: &mut ConfigValidationReport,
    path: String,
    packages: Option<&[String]>,
) {
    let Some(packages) = packages else {
        return;
    };
    if packages.iter().any(|package| package.trim().is_empty()) {
        report.add(
            ConfigValidationLevel::Error,
            path.clone(),
            "package lists cannot contain empty entries".to_string(),
        );
    }
    if packages.is_empty() {
        report.add(
            ConfigValidationLevel::Warning,
            path,
            "empty package list has no effect".to_string(),
        );
    }
}

fn override_platform(platform_key: &str) -> Option<OperatingSystem> {
    let key = platform_key.trim().to_ascii_lowercase();
    match key.as_str() {
        "macos" | "linux" | "windows" => OperatingSystem::from_platform_tag(&key),
        _ if key.starts_with("macos-")
            || key.starts_with("linux-")
            || key.starts_with("windows-") =>
        {
            OperatingSystem::from_platform_tag(&key)
        }
        _ => None,
    }
}

fn known_tool(tool: &str) -> bool {
    matches!(
        tool,
        "brew"
            | "winget"
            | "fnm"
            | "nvm"
            | "node"
            | "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "deno"
            | "wrangler"
            | "uv"
            | "python"
            | "poetry"
            | "ruby"
            | "rust"
            | "rustup"
            | "rustc"
            | "cargo"
            | "go"
    )
}

fn known_manager(manager: &str) -> bool {
    matches!(
        canonical_manager(manager).as_str(),
        "brew"
            | "winget"
            | "standalone"
            | "fnm"
            | "nvm"
            | "npm"
            | "corepack"
            | "uv"
            | "official"
            | "pipx"
            | "manual"
            | "rustup"
            | "apt"
            | "dnf"
            | "pacman"
            | "zypper"
            | "apk"
            | "scoop"
            | "choco"
    )
}

fn cli_package_manager(manager: &str) -> bool {
    matches!(
        canonical_manager(manager).as_str(),
        "brew" | "npm" | "apt" | "dnf" | "pacman" | "zypper" | "apk" | "winget" | "scoop" | "choco"
    )
}

fn manager_supported_for_tool(tool: &str, manager: &str, platform: OperatingSystem) -> bool {
    let manager = canonical_manager(manager);
    match tool {
        "brew" => manager == "brew" && !platform.is_windows(),
        "winget" => manager == "winget",
        "fnm" => matches!(manager.as_str(), "brew" | "winget" | "standalone"),
        "nvm" => matches!(manager.as_str(), "standalone" | "nvm"),
        "node" => matches!(manager.as_str(), "fnm" | "nvm" | "brew" | "winget"),
        "npm" => manager == "npm",
        "pnpm" => manager == "corepack",
        "yarn" => matches!(manager.as_str(), "corepack" | "npm"),
        "bun" | "deno" => matches!(manager.as_str(), "brew" | "winget" | "standalone"),
        "wrangler" => manager == "npm",
        "uv" => matches!(manager.as_str(), "standalone" | "brew" | "winget"),
        "python" => matches!(manager.as_str(), "brew" | "uv" | "winget"),
        "poetry" => matches!(manager.as_str(), "brew" | "official" | "pipx"),
        "ruby" => matches!(manager.as_str(), "brew" | "manual"),
        "go" => matches!(manager.as_str(), "brew" | "winget" | "official"),
        "rust" | "rustup" | "rustc" | "cargo" => manager == "rustup",
        _ => false,
    }
}

fn canonical_manager(manager: &str) -> String {
    match manager.trim().to_ascii_lowercase().as_str() {
        "homebrew" => "brew".to_string(),
        "apt-get" => "apt".to_string(),
        "chocolatey" => "choco".to_string(),
        other => other.to_string(),
    }
}

fn section_path(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_string()
    } else {
        format!("{prefix}.{path}")
    }
}

fn collect_config_entries(
    config: &DevkitConfig,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    if let Some(policy) = &config.policy {
        collect_policy_entries("policy", policy, source, entries);
    }
    if let Some(tools) = &config.tools {
        collect_tool_map_entries("tools", tools, source, entries);
    }
    if let Some(homebrew) = &config.homebrew {
        collect_homebrew_entries("homebrew", homebrew, source, entries);
    }
    if let Some(npm) = &config.npm {
        collect_npm_entries("npm", npm, source, entries);
    }
}

fn collect_override_entries(
    override_config: &PlatformOverride,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    if let Some(policy) = &override_config.policy {
        collect_policy_entries("policy", policy, source, entries);
    }
    if let Some(tools) = &override_config.tools {
        collect_tool_map_entries("tools", tools, source, entries);
    }
    if let Some(homebrew) = &override_config.homebrew {
        collect_homebrew_entries("homebrew", homebrew, source, entries);
    }
    if let Some(npm) = &override_config.npm {
        collect_npm_entries("npm", npm, source, entries);
    }
}

fn collect_policy_entries(
    prefix: &str,
    policy: &PolicyConfig,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    insert_entry(
        entries,
        format!("{prefix}.channel"),
        source,
        &policy.channel,
    );
    insert_entry(
        entries,
        format!("{prefix}.auto_fix"),
        source,
        &policy.auto_fix,
    );
    insert_entry(entries, format!("{prefix}.mirror"), source, &policy.mirror);
    insert_entry(
        entries,
        format!("{prefix}.platform"),
        source,
        &policy.platform,
    );
}

fn collect_tool_map_entries(
    prefix: &str,
    tools: &BTreeMap<String, ToolPolicy>,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    for (tool, policy) in tools {
        let prefix = format!("{prefix}.{tool}");
        insert_entry(
            entries,
            format!("{prefix}.version"),
            source,
            &policy.version,
        );
        insert_entry(
            entries,
            format!("{prefix}.manager"),
            source,
            &policy.manager,
        );
        insert_entry(entries, format!("{prefix}.source"), source, &policy.source);
        insert_entry(
            entries,
            format!("{prefix}.channel"),
            source,
            &policy.channel,
        );
        insert_entry(
            entries,
            format!("{prefix}.components"),
            source,
            &policy.components,
        );
        insert_entry(
            entries,
            format!("{prefix}.package_managers"),
            source,
            &policy.package_managers,
        );
        insert_entry(
            entries,
            format!("{prefix}.packages"),
            source,
            &policy.packages,
        );
        insert_entry(
            entries,
            format!("{prefix}.install_dir"),
            source,
            &policy.install_dir,
        );
    }
}

fn collect_homebrew_entries(
    prefix: &str,
    homebrew: &HomebrewConfig,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    insert_entry(
        entries,
        format!("{prefix}.mirror"),
        source,
        &homebrew.mirror,
    );
    insert_entry(
        entries,
        format!("{prefix}.packages"),
        source,
        &homebrew.packages,
    );
}

fn collect_npm_entries(
    prefix: &str,
    npm: &NpmConfig,
    source: &str,
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
) {
    insert_entry(
        entries,
        format!("{prefix}.global_packages"),
        source,
        &npm.global_packages,
    );
}

fn insert_entry<T: Serialize>(
    entries: &mut BTreeMap<String, ConfigExplainEntry>,
    path: String,
    source: &str,
    value: &Option<T>,
) {
    let Some(value) = value else {
        return;
    };
    entries.insert(
        path.clone(),
        ConfigExplainEntry {
            path,
            source: source.to_string(),
            value: serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".to_string()),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{ConfigValidationLevel, DevkitConfig, PlatformOverride, PolicyConfig, ToolPolicy};
    use crate::platform::OperatingSystem;

    #[test]
    fn applies_os_and_arch_platform_overrides_in_order() {
        let config = toml::from_str::<DevkitConfig>(
            r#"
[policy]
channel = "stable"

[tools.fnm]
version = "latest"
manager = "standalone"

[platform.macos.policy]
platform = "macos"

[platform.macos.tools.fnm]
manager = "brew"

[platform.macos-arm64.tools.fnm]
version = "1.2.3"
"#,
        )
        .unwrap()
        .effective_for(OperatingSystem::Macos, "macos-arm64");
        let fnm = config.tools.as_ref().unwrap().get("fnm").unwrap();

        assert_eq!(
            config
                .policy
                .as_ref()
                .and_then(|policy| policy.platform.as_deref()),
            Some("macos")
        );
        assert_eq!(fnm.manager.as_deref(), Some("brew"));
        assert_eq!(fnm.version.as_deref(), Some("1.2.3"));
        assert!(config.platform.is_none());
    }

    #[test]
    fn platform_override_can_add_tools() {
        let config = DevkitConfig {
            platform: Some(
                [(
                    "windows".to_string(),
                    PlatformOverride {
                        policy: Some(PolicyConfig {
                            platform: Some("windows-x86_64".to_string()),
                            ..PolicyConfig::default()
                        }),
                        tools: Some(
                            [(
                                "go".to_string(),
                                ToolPolicy {
                                    manager: Some("winget".to_string()),
                                    source: Some("winget".to_string()),
                                    ..ToolPolicy::default()
                                },
                            )]
                            .into_iter()
                            .collect(),
                        ),
                        ..PlatformOverride::default()
                    },
                )]
                .into_iter()
                .collect(),
            ),
            ..DevkitConfig::default()
        }
        .effective_for(OperatingSystem::Windows, "windows-x86_64");
        let go = config.tools.as_ref().unwrap().get("go").unwrap();

        assert_eq!(go.manager.as_deref(), Some("winget"));
        assert_eq!(go.source.as_deref(), Some("winget"));
    }

    #[test]
    fn validation_reports_unknown_platform_and_unsupported_manager() {
        let config = toml::from_str::<DevkitConfig>(
            r#"
[tools.deno]
manager = "apt"

[platform.linuz.tools.go]
manager = "official"
"#,
        )
        .unwrap();
        let report = config.validate_for_current_platform();

        assert!(report.has_errors());
        assert!(report.items.iter().any(|item| {
            item.level == ConfigValidationLevel::Error && item.path == "platform.linuz"
        }));
        assert!(report.items.iter().any(|item| {
            item.level == ConfigValidationLevel::Error && item.path == "tools.deno.manager"
        }));
    }

    #[test]
    fn explain_tracks_overridden_sources() {
        let config = toml::from_str::<DevkitConfig>(
            r#"
[tools.node]
version = "stable"
manager = "fnm"

[platform.macos.tools.node]
version = "24.x"
"#,
        )
        .unwrap();
        let report = config.explain_for(OperatingSystem::Macos, "macos-arm64");

        assert_eq!(report.applied_overrides, vec!["platform.macos"]);
        assert!(report.entries.iter().any(|entry| {
            entry.path == "tools.node.manager" && entry.source == "base" && entry.value == "\"fnm\""
        }));
        assert!(report.entries.iter().any(|entry| {
            entry.path == "tools.node.version"
                && entry.source == "platform.macos"
                && entry.value == "\"24.x\""
        }));
    }

    #[test]
    fn validation_accepts_nvm_managed_node() {
        let config = toml::from_str::<DevkitConfig>(
            r#"
[tools.node]
version = "24.x"
manager = "nvm"
"#,
        )
        .unwrap();
        let report = config.validate_for_current_platform();

        assert!(!report.has_errors());
    }
}
