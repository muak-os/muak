//! Concrete output artifact produced by the build pipeline.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Concrete output artifact produced by the build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Artifact {
    /// Kernel image.
    Kernel,
    /// Initial RAM filesystem image.
    Initramfs,
    /// Kernel command-line file.
    Cmdline,
    /// Unified kernel image (UKI) EFI binary.
    Uki,
    /// ISO 9660 bootable image.
    Iso,
    /// Raw disk image.
    Raw,
    /// Board-specific overlay boot assets as a tar archive.
    Overlays,
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.filename())
    }
}

impl Artifact {
    /// Returns the canonical on-disk filename or directory for this artifact.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Initramfs => "initramfs.img",
            Self::Cmdline => "cmdline",
            Self::Uki => "uki.efi",
            Self::Iso => "muak.iso",
            Self::Raw => "muak.raw",
            Self::Overlays => "overlays.tar",
        }
    }

    /// Returns the MIME media type for this artifact.
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Cmdline => "text/plain; charset=utf-8",
            Self::Iso => "application/x-iso9660-image",
            Self::Kernel | Self::Initramfs | Self::Uki | Self::Raw => "application/octet-stream",
            Self::Overlays => "application/x-tar",
        }
    }

    /// Number of artifact variants. Update when variants are added.
    pub(crate) const COUNT: usize = 7;

    /// Returns true when this artifact carries a transport codec.
    #[must_use]
    pub const fn supports_codec(self) -> bool {
        matches!(self, Self::Raw)
    }

    /// Returns the on-disk output name for this artifact under `codec`.
    #[must_use]
    pub fn output_name(self, codec: crate::codec::Codec) -> String {
        if !self.supports_codec() {
            return self.filename().to_owned();
        }

        match codec.extension() {
            "" => self.filename().to_owned(),
            extension => format!("{}.{}", self.filename(), extension),
        }
    }

    /// Returns a zero-based index for use as an array index.
    #[must_use]
    pub(crate) fn to_index(self) -> usize {
        match self {
            Self::Kernel => 0,
            Self::Initramfs => 1,
            Self::Cmdline => 2,
            Self::Uki => 3,
            Self::Iso => 4,
            Self::Raw => 5,
            Self::Overlays => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;

    #[test]
    fn artifact_filename_and_media_type() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Artifact::Iso.filename(), "muak.iso");
        assert_eq!(Artifact::Cmdline.media_type(), "text/plain; charset=utf-8");
        assert_eq!(Artifact::Kernel.filename(), "kernel");
    }

    #[test]
    fn all_artifact_filenames() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Artifact::Kernel.filename(), "kernel");
        assert_eq!(Artifact::Initramfs.filename(), "initramfs.img");
        assert_eq!(Artifact::Cmdline.filename(), "cmdline");
        assert_eq!(Artifact::Uki.filename(), "uki.efi");
        assert_eq!(Artifact::Iso.filename(), "muak.iso");
        assert_eq!(Artifact::Raw.filename(), "muak.raw");
        assert_eq!(Artifact::Overlays.filename(), "overlays.tar");
    }

    #[test]
    fn only_raw_supports_a_codec() {
        // ARRANGE
        let artifacts = [
            Artifact::Kernel,
            Artifact::Initramfs,
            Artifact::Cmdline,
            Artifact::Uki,
            Artifact::Iso,
            Artifact::Raw,
            Artifact::Overlays,
        ];

        // ACT & ASSERT
        for artifact in artifacts {
            assert_eq!(
                artifact.supports_codec(),
                artifact == Artifact::Raw,
                "{artifact} codec support mismatch"
            );
        }
    }

    #[test]
    fn output_name_applies_codec_to_raw_only() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(Artifact::Raw.output_name(Codec::Zstd), "muak.raw.zst");
        assert_eq!(Artifact::Raw.output_name(Codec::Gzip), "muak.raw.gz");
        assert_eq!(Artifact::Raw.output_name(Codec::None), "muak.raw");
        assert_eq!(Artifact::Iso.output_name(Codec::Gzip), "muak.iso");
        assert_eq!(Artifact::Kernel.output_name(Codec::Zstd), "kernel");
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
        assert_eq!(Artifact::Overlays.media_type(), "application/x-tar");
    }
}
