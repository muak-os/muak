//! BuildKit-compatible garbage collection for the koci pull cache.

mod prune;

use std::path::Path;

/// Subtree holding in-progress ingests, which the content store collects itself.
pub(crate) use wizard::cache::INGEST_DIR;

/// Base directory for the koci pull cache on the EPHEMERAL partition.
pub(crate) const DIR: &str = "/run/ephemeral/koci";

/// Mount point of the EPHEMERAL partition.
pub(crate) const EPHEMERAL_MOUNT: &str = "/run/ephemeral";

// Btrfs `f_type` magic reported by statfs.
const BTRFS_MAGIC: i64 = 0x9123_683E;

/// Configures the koci pull cache under the EPHEMERAL partition.
pub(crate) fn init() {
    wizard::config::init_cache(DIR.into());
}

/// Whether the EPHEMERAL partition supports the koci pull cache.
pub(crate) fn available() -> bool {
    rustix::fs::statfs(EPHEMERAL_MOUNT).is_ok_and(|stat| stat.f_type == BTRFS_MAGIC)
}

/// Enforces the BuildKit-compatible cache eviction policy.
pub(crate) fn enforce() {
    let Ok(stat) = rustix::fs::statfs(EPHEMERAL_MOUNT) else {
        return;
    };
    if stat.f_type != BTRFS_MAGIC {
        return;
    }
    let block = block_size(&stat);
    let total = stat.f_blocks.saturating_mul(block);
    if total == 0 {
        return;
    }
    let free = stat.f_bavail.saturating_mul(block);

    prune::enforce(Path::new(DIR), total, free);
}

fn block_size(stat: &rustix::fs::StatFs) -> u64 {
    let size = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };

    u64::try_from(size).unwrap_or_default()
}
