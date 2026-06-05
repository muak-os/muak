use serde::{Deserialize, Serialize};

/// Virtual machine configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    /// Whether to automatically restart VMs on failure.
    pub auto_restart: bool,
}
