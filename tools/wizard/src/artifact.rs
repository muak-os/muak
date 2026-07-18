//! Concrete output artifact produced by the build pipeline.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Concrete output artifact produced by the build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Artifact {
    /// Linux kernel image.
    Kernel,
    /// Initial RAM filesystem image.
    Initramfs,
    /// Kernel command-line file.
    Cmdline,
    /// Unified kernel image (UKI) EFI binary.
    Uki,
    /// ISO 9660 bootable image.
    Iso,
    /// Raw disk image (compressed via zstd).
    Raw,
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.filename())
    }
}

impl Artifact {
    /// Number of artifact variants. Update when variants are added.
    pub(crate) const COUNT: usize = 6;

    /// Returns the zero-based discriminant for use as an array index.
    pub(crate) fn discriminant(self) -> usize {
        match self {
            Self::Kernel => 0,
            Self::Initramfs => 1,
            Self::Cmdline => 2,
            Self::Uki => 3,
            Self::Iso => 4,
            Self::Raw => 5,
        }
    }

    /// Returns the canonical on-disk filename or directory for this artifact.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Initramfs => "initramfs.img",
            Self::Cmdline => "cmdline",
            Self::Uki => "uki.efi",
            Self::Iso => "muak.iso",
            Self::Raw => "muak.raw.zst",
        }
    }

    /// Returns the MIME media type for this artifact.
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Cmdline => "text/plain; charset=utf-8",
            Self::Iso => "application/x-iso9660-image",
            Self::Kernel | Self::Initramfs | Self::Uki | Self::Raw => "application/octet-stream",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_filename_and_media_type() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Artifact::Iso.filename(), "muak.iso");
        assert_eq!(Artifact::Cmdline.media_type(), "text/plain; charset=utf-8");
        assert_eq!(Artifact::Kernel.filename(), "kernel");
    }

    #[test]
    fn all_artifact_filenames() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Artifact::Kernel.filename(), "kernel");
        assert_eq!(Artifact::Initramfs.filename(), "initramfs.img");
        assert_eq!(Artifact::Cmdline.filename(), "cmdline");
        assert_eq!(Artifact::Uki.filename(), "uki.efi");
        assert_eq!(Artifact::Iso.filename(), "muak.iso");
        assert_eq!(Artifact::Raw.filename(), "muak.raw.zst");
    }

    #[test]
    fn all_artifact_media_types() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Artifact::Cmdline.media_type(), "text/plain; charset=utf-8");
        assert_eq!(Artifact::Iso.media_type(), "application/x-iso9660-image");
        assert_eq!(Artifact::Kernel.media_type(), "application/octet-stream");
        assert_eq!(Artifact::Initramfs.media_type(), "application/octet-stream");
        assert_eq!(Artifact::Uki.media_type(), "application/octet-stream");
        assert_eq!(Artifact::Raw.media_type(), "application/octet-stream");
    }
}
