//! Mount operations for early boot.

mod layers;
mod luks;
mod partition;
mod persistent;
mod pseudo;
mod rootfs;

/// Image format file extension.
pub(crate) const IMAGE_EXTENSION: &str = "erofs";

/// Kernel filesystem type used to mount image files.
pub(crate) const IMAGE_FSTYPE: &str = IMAGE_EXTENSION;

pub(crate) use persistent::persistent;
pub(crate) use pseudo::pseudo;
pub(crate) use rootfs::rootfs;
