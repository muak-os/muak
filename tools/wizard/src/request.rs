//! User request for image resolution, building, and installation.

use core::fmt;

use koci::arch::Arch;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;

/// Unified request used by resolve, build, and install paths.
///
/// When `arch` is `None` the host architecture is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Version to resolve/build/install.
    pub version: String,
    /// Target deployment platform.
    pub platform: Platform,
    /// Target CPU architecture (`None` → host arch).
    pub arch: Option<Arch>,
    /// Artifact types to build (ignored by resolve and install).
    pub artifacts: Vec<Artifact>,
}

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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
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
