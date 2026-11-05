//! Error types and shared result alias.

use thiserror::Error;

/// Errors returned by partdev operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum PartdevError {
    /// Wraps underlying I/O failures.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps partition table encoding and decoding failures.
    #[error("partition table error: {0}")]
    Table(#[from] parttable::error::ParttableError),

    /// Wraps block device inspection failures.
    #[error("block device error: {0}")]
    Device(#[from] blockdev::error::BlockdevError),

    /// Reports a BLKPG ioctl failure for one partition.
    #[error("BLKPG {operation} of partition {number} failed: {errno}")]
    Blkpg {
        /// Operation that failed, such as `add` or `delete`.
        operation: &'static str,
        /// Partition number passed to the ioctl.
        number: u32,
        /// Errno reported by the kernel.
        errno: rustix::io::Errno,
    },

    /// Reports a device whose sector count is unknown.
    #[error("the disk sector count is unknown")]
    UnknownSectorCount,
}

/// Shared result type for partdev operations.
pub type Result<T> = core::result::Result<T, PartdevError>;
