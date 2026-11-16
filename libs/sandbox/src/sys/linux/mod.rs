//! Linux syscall backend.
//!
//! Phase 0 only carried the error conversion. Phase 2 adds the credential,
//! limit and spawn calls the child path needs, each gated by the feature that
//! can reach it: the identity and no-new-privs calls ride on `spawn`, the
//! capability calls on `caps`, and the limit calls on `limits` and `spawn`.
//! Phase 4 adds the namespace, mount and root filesystem switch calls, which
//! only the child path reaches, so they ride on `spawn` as well.

#[cfg(feature = "spawn")]
pub(crate) mod credentials;
#[cfg(all(feature = "limits", feature = "spawn"))]
pub(crate) mod limits;
#[cfg(all(feature = "mounts", feature = "spawn"))]
pub(crate) mod mount;
#[cfg(all(feature = "namespaces", feature = "spawn"))]
pub(crate) mod namespace;
#[cfg(feature = "probe")]
pub(crate) mod probe;
#[cfg(all(feature = "rootfs", feature = "spawn"))]
pub(crate) mod rootfs;
#[cfg(feature = "spawn")]
pub(crate) mod spawn;

use rustix::io;

use crate::error::Errno;

impl From<io::Errno> for Errno {
    fn from(error: io::Errno) -> Self {
        Self::new(error.raw_os_error())
    }
}
