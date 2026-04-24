use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct DevkitConfig {
    pub tools: Option<BTreeMap<String, ToolPolicy>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ToolPolicy {
    pub version: Option<String>,
    pub manager: Option<String>,
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
