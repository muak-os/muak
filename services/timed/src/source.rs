//! Time source selection.

use core::fmt::{self, Display};
use core::time::Duration;

use anyhow::{Result, bail};

use crate::hypervisor;
use crate::ntp;

/// Selected time source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Hypervisor clock when available, NTP otherwise.
    Auto,
    /// Hypervisor clock only (`/dev/ptp0`); fail if absent.
    Hypervisor,
    /// NTP only.
    Ntp,
}

impl Source {
    /// Parses the `host.clock` config value (empty means [`Source::Auto`]).
    pub fn from_config(value: &str) -> Result<Self> {
        match value {
            "" | "auto" => Ok(Self::Auto),
            "hypervisor" => Ok(Self::Hypervisor),
            "ntp" => Ok(Self::Ntp),
            other => bail!(
                "unknown host.clock value {other:?} (expected \"auto\", \"hypervisor\" or \"ntp\")"
            ),
        }
    }

    /// Returns whether this source requires a configured NTP server.
    pub fn needs_ntp(self) -> bool {
        !matches!(self, Self::Hypervisor)
    }
}

impl Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Hypervisor => f.write_str("hypervisor clock"),
            Self::Auto => f.write_str("auto"),
            Self::Ntp => f.write_str("NTP"),
        }
    }
}

/// Performs a single synchronization attempt against the selected source.
pub async fn sync(source: Source, server: &str) -> Result<(Source, Duration)> {
    match source {
        Source::Hypervisor => hypervisor::sync().map(|offset| (Source::Hypervisor, offset)),
        Source::Ntp => ntp::sync(server).await.map(|offset| (Source::Ntp, offset)),
        Source::Auto => match hypervisor::sync() {
            Ok(offset) => Ok((Source::Hypervisor, offset)),
            Err(ptp_error) => ntp::sync(server)
                .await
                .map(|offset| (Source::Ntp, offset))
                .map_err(|ntp_error| {
                    anyhow::anyhow!(
                        "hypervisor clock unavailable: {ptp_error:#}; NTP sync failed: {ntp_error:#}"
                    )
                }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_parses_known_values() {
        // ARRANGE
        let values = [
            ("auto", Source::Auto),
            ("hypervisor", Source::Hypervisor),
            ("ntp", Source::Ntp),
        ];

        for (value, expected) in values {
            // ACT
            let parsed = Source::from_config(value);

            // ASSERT
            assert_eq!(parsed.ok(), Some(expected), "value {value:?}");
        }
    }

    #[test]
    fn from_config_treats_empty_as_auto() {
        // ARRANGE
        let value = "";

        // ACT
        let parsed = Source::from_config(value);

        // ASSERT
        assert_eq!(parsed.ok(), Some(Source::Auto));
    }

    #[test]
    fn from_config_rejects_unknown_values() {
        // ARRANGE
        let value = "ptp";

        // ACT
        let parsed = Source::from_config(value);

        // ASSERT
        assert!(parsed.is_err(), "value {value:?} should be rejected");
    }

    #[test]
    fn needs_ntp_only_for_hypervisor_independent_sources() {
        // ARRANGE
        let sources = [
            (Source::Hypervisor, false),
            (Source::Auto, true),
            (Source::Ntp, true),
        ];

        for (source, expected) in sources {
            // ACT
            let needs = source.needs_ntp();

            // ASSERT
            assert_eq!(needs, expected, "source {source}");
        }
    }
}
