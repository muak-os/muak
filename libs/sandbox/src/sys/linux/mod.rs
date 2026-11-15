//! Linux syscall backend.
//!
//! Phase 0 only carries the error conversion. The rest of the syscall surface
//! arrives in the later phases, except the availability probes detection needs.

#[cfg(feature = "probe")]
pub(crate) mod probe;

use rustix::io;

use crate::error::Errno;

impl From<io::Errno> for Errno {
    fn from(error: io::Errno) -> Self {
        Self::new(error.raw_os_error())
    }
}
