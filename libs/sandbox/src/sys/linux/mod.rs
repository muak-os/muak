//! Linux syscall backend.

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
