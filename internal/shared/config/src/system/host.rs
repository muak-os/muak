use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostConfig {
    pub name: String,
    pub image: String,
    pub extensions: Vec<String>,
    pub secureboot: bool,
    pub port: u16,
    pub ntp: String,
}
