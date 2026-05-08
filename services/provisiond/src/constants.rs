//! Global constants for provisiond.

/// OCI architecture string for the current host.
pub fn host_oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// Base directory for secrets on the STATE partition.
pub const SECRETS_DIR: &str = "/run/state/secrets";

/// Staging directory for update operations.
pub const UPDATE_DIR: &str = "/run/state/update";

/// dm-crypt mapping name for the STATE partition.
pub const DM_STATE: &str = "muak-state";

/// dm-crypt mapping name for the DATA partition.
pub const DM_DATA: &str = "muak-data";
