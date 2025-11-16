pub const LAN_BRIDGE_NAME: &str = "br0";
pub const RESOLV_CONF_PATH: &str = "/run/resolv.conf";

pub const BRIDGE_CREATE_RETRIES: u8 = 30;
pub const BRIDGE_CREATE_RETRY_DELAY_MS: u64 = 100;
pub const INTERFACE_ENSLAVE_RETRIES: u8 = 5;
pub const INTERFACE_ENSLAVE_RETRY_DELAY_MS: u64 = 100;
