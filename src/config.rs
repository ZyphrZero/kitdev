use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct DevkitConfig {
    pub policy: Option<PolicyConfig>,
    pub tools: Option<BTreeMap<String, ToolPolicy>>,
    pub homebrew: Option<HomebrewConfig>,
    pub npm: Option<NpmConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PolicyConfig {
    pub channel: Option<String>,
    pub auto_fix: Option<bool>,
    pub mirror: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
pub struct HomebrewConfig {
    pub mirror: Option<String>,
    pub packages: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NpmConfig {
    pub global_packages: Option<Vec<String>>,
}

impl DevkitConfig {
    pub fn read(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("failed to parse config {}", path.display()))
    }
}
