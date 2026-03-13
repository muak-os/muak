use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    pub auto_restart: bool,
}
