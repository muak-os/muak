//! Request types: artifact, platform, and the concrete build request.

use core::fmt;

use koci::arch::Arch;
use serde::{Deserialize, Serialize};

/// Deployment platform; determines boot and disk behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Metal,
    Aws,
    Gcp,
}

impl Platform {
    /// Returns the lowercase path segment for this platform.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metal => "metal",
            Self::Aws => "aws",
            Self::Gcp => "gcp",
        }
    }
}

impl fmt::Display for Platform {
    /// Formats the platform as its lowercase path segment.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Concrete output artifact produced by the build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Artifact {
    Kernel,
    Initramfs,
    Cmdline,
    Uki,
    Iso,
    Raw,
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.filename())
    }
}

impl Artifact {
    /// Returns the canonical on-disk filename for this artifact.
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

/// User-facing artifact request combining identity, version, target, and desired output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub profile_id: String,
    pub version: String,
    pub artifact: Artifact,
    pub platform: Platform,
    pub arch: Arch,
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

    #[test]
    fn arch_conversions() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::Amd64.as_str(), "amd64");
        assert_eq!(Arch::Arm64.as_str(), "arm64");
        assert_eq!(format!("{}", Arch::Amd64), "amd64");
        assert_eq!(format!("{}", Arch::Arm64), "arm64");
        assert_eq!(format!("{}", Platform::Metal), "metal");
    }

    #[test]
    fn platform_display() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Platform::Metal.as_str(), "metal");
        assert_eq!(format!("{}", Platform::Aws), "aws");
        assert_eq!(format!("{}", Platform::Gcp), "gcp");
    }
}
