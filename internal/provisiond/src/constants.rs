//! Global constants for provisiond.

/// Base directory for secrets on the STATE partition.
pub const SECRETS_DIR: &str = "/run/state/secrets";

/// Working directory for installation operations.
pub const INSTALL_DIR: &str = "/run/install";

/// Staging directory for update operations.
pub const UPDATE_DIR: &str = "/run/state/update";

/// Default kernel command line for x86_64 architecture.
#[cfg(target_arch = "x86_64")]
pub const DEFAULT_CMDLINE: &str =
    include_str!("../../../pkgs/kernel/cmdline-amd64.txt").trim_ascii();

/// Default kernel command line for AArch64 architecture.
#[cfg(target_arch = "aarch64")]
pub const DEFAULT_CMDLINE: &str =
    include_str!("../../../pkgs/kernel/cmdline-arm64.txt").trim_ascii();

/// Size of the LUKS key in bytes.
pub const LUKS_KEY_SIZE: usize = 64;

/// dm-crypt mapping name for the STATE partition.
pub const DM_STATE: &str = "muak-state";

/// dm-crypt mapping name for the DATA partition.
pub const DM_DATA: &str = "muak-data";
