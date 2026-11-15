//! Linux syscall backend.
//!
//! Phase 0 only carries the error conversion. The rest of the syscall surface
//! arrives in Phases 3 and 5.

use rustix::io;

use crate::error::Errno;

impl From<io::Errno> for Errno {
    fn from(error: io::Errno) -> Self {
        Self::new(error.raw_os_error())
    }
}
