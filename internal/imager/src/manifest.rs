use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
}

impl ExtensionManifest {
    pub fn from_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;
        let manifest: ExtensionManifest = serde_yaml::from_str(&content)?;
        Ok(manifest)
    }
}
