//! OCI platform architecture identifiers.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Target CPU architecture for an OCI image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    /// Returns the OCI architecture identifier for this target.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returns the OCI architecture for the current host.
#[must_use]
pub fn host() -> Arch {
    if std::env::consts::ARCH == "aarch64" {
        Arch::Arm64
    } else {
        Arch::Amd64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_identifiers() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::Amd64.as_str(), "amd64");
        assert_eq!(Arch::Arm64.as_str(), "arm64");
    }

    #[test]
    fn display_formats_lowercase() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(format!("{}", Arch::Amd64), "amd64");
        assert_eq!(format!("{}", Arch::Arm64), "arm64");
    }
}
