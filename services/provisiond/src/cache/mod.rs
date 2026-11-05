//! BuildKit-compatible garbage collection for the koci pull cache.

mod prune;

use std::path::Path;

/// Base directory for the koci pull cache on the EPHEMERAL partition.
pub(crate) const DIR: &str = "/run/ephemeral/koci";

/// Mount point of the EPHEMERAL partition.
pub(crate) const EPHEMERAL_MOUNT: &str = "/run/ephemeral";

const BTRFS_MAGIC: i64 = 0x9123_683E;

/// Configures the koci pull cache under the EPHEMERAL partition.
pub(crate) fn init() {
    wizard::config::init_cache(DIR.into());
}

/// Whether the filesystem hosting the koci pull cache supports it.
pub(crate) fn available() -> bool {
    let root = wizard::cache::root();

    rustix::fs::statfs(host(root.as_deref())).is_ok_and(|stat| stat.f_type == BTRFS_MAGIC)
}

/// Enforces the BuildKit-compatible cache eviction policy.
pub(crate) fn enforce() {
    let Some(root) = wizard::cache::root() else {
        return;
    };
    let Ok(stat) = rustix::fs::statfs(root.as_path()) else {
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

    prune::enforce(&root, total, free);
}

fn host(root: Option<&Path>) -> &Path {
    root.unwrap_or(Path::new(EPHEMERAL_MOUNT))
}

fn block_size(stat: &rustix::fs::StatFs) -> u64 {
    let size = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };

    u64::try_from(size).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{EPHEMERAL_MOUNT, host};

    #[test]
    fn host_falls_back_to_the_ephemeral_mount() {
        // ARRANGE
        let root = Path::new("/run/ephemeral/koci");

        // ACT
        let resolved = host(Some(root));
        let fallback = host(None);

        // ASSERT
        assert_eq!(resolved, root, "a resolved root is used as is");
        assert_eq!(
            fallback,
            Path::new(EPHEMERAL_MOUNT),
            "an unset root falls back to the EPHEMERAL mount"
        );
    }
}
