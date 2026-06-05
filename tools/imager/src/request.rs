//! Request types: architecture, platform, and focused request structs.

use core::fmt;

use koci::arch::Arch;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;

/// Deployment platform; determines boot and disk behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Bare-metal installation.
    Metal,
    /// Amazon Web Services.
    Aws,
    /// Google Cloud Platform.
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

/// Request to resolve a profile into versioned OCI references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolve {
    /// Version to resolve.
    pub version: String,
    /// Target deployment platform.
    pub platform: Platform,
    /// Target CPU architecture.
    pub arch: Arch,
}

/// Request to build a specific artifact from a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// Version to build.
    pub version: String,
    /// Target deployment platform.
    pub platform: Platform,
    /// Target CPU architecture.
    pub arch: Arch,
    /// Artifact type to build.
    pub artifact: Artifact,
}

/// Request to prepare install assets from a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// Version to install.
    pub version: String,
    /// Target deployment platform.
    pub platform: Platform,
    /// Target CPU architecture.
    pub arch: Arch,
}

#[cfg(test)]
mod tests {
    use super::*;

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
