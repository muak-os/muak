//! OCI platform architecture identifiers.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Target CPU architecture for an OCI image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// 64-bit x86 architecture.
    Amd64,
    /// 64-bit ARM architecture.
    Arm64,
    /// 64-bit RISC-V architecture.
    Riscv64,
}

impl Arch {
    /// All supported architectures.
    const ALL: [Arch; 3] = [Self::Amd64, Self::Arm64, Self::Riscv64];

    /// Returns the OCI architecture identifier for this target.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Riscv64 => "riscv64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::str::FromStr for Arch {
    type Err = String;

    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|arch| arch.as_str() == s)
            .ok_or_else(|| {
                format!(
                    "unknown architecture '{s}' (expected {})",
                    Self::ALL
                        .iter()
                        .map(|arch| arch.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

/// Returns the OCI architecture for the current host.
#[must_use]
pub fn host() -> Arch {
    match std::env::consts::ARCH {
        "aarch64" => Arch::Arm64,
        "riscv64" => Arch::Riscv64,
        _ => Arch::Amd64,
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
        assert_eq!(Arch::Riscv64.as_str(), "riscv64");
    }

    #[test]
    fn display_formats_lowercase() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(format!("{}", Arch::Amd64), "amd64");
        assert_eq!(format!("{}", Arch::Arm64), "arm64");
        assert_eq!(format!("{}", Arch::Riscv64), "riscv64");
    }

    #[test]
    fn from_str_parses_known_architectures() {
        // ARRANGE / ACT / ASSERT
        assert!(matches!("amd64".parse(), Ok(Arch::Amd64)));
        assert!(matches!("arm64".parse(), Ok(Arch::Arm64)));
        assert!(matches!("riscv64".parse(), Ok(Arch::Riscv64)));
    }

    #[test]
    fn from_str_reports_unknown_architectures() {
        // ARRANGE / ACT
        let error = "mips".parse::<Arch>().expect_err("parse should fail");

        // ASSERT
        assert!(error.contains("unknown architecture 'mips'"));
    }
}
