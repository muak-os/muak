//! Administrative and carrier state helpers.

use core::fmt;

use crate::{AdminState, CarrierState, LinkState};

impl AdminState {
    /// Returns true when the interface is administratively enabled.
    #[must_use = "callers must distinguish disabled links"]
    pub const fn is_up(self) -> bool {
        matches!(self, Self::Up)
    }
}

impl LinkState {
    /// Combines the administrative and carrier parts into a link state.
    #[must_use = "the combined state drives every readiness decision"]
    pub const fn from_parts(admin: AdminState, carrier: CarrierState) -> Self {
        if !admin.is_up() {
            return Self::Down;
        }

        match carrier {
            CarrierState::Up | CarrierState::Unknown => Self::Up,
            CarrierState::Dormant => Self::Dormant,
            CarrierState::Down => Self::NoCarrier,
        }
    }

    /// Returns the administrative part of the link state.
    #[must_use = "callers must persist the administrative state"]
    pub const fn admin(self) -> AdminState {
        match self {
            Self::Up | Self::NoCarrier | Self::Dormant => AdminState::Up,
            Self::Down => AdminState::Down,
        }
    }

    /// Returns the carrier part of the link state.
    #[must_use = "callers must persist the carrier state"]
    pub const fn carrier(self) -> CarrierState {
        match self {
            Self::Up => CarrierState::Up,
            Self::NoCarrier => CarrierState::Down,
            Self::Dormant => CarrierState::Dormant,
            Self::Down => CarrierState::Unknown,
        }
    }

    /// Returns true when the link is administratively up.
    #[must_use = "callers must skip administratively disabled links"]
    pub const fn is_admin_up(self) -> bool {
        !matches!(self, Self::Down)
    }

    /// Returns true when the link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub const fn has_carrier(self) -> bool {
        matches!(self, Self::Up)
    }
}

impl fmt::Display for LinkState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Up => f.write_str("up"),
            Self::NoCarrier => f.write_str("no-carrier"),
            Self::Dormant => f.write_str("dormant"),
            Self::Down => f.write_str("down"),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::string::ToString as _;

    use crate::{AdminState, CarrierState, LinkState};

    #[test]
    fn from_parts_maps_every_combination() {
        // ARRANGE
        let cases = [
            (AdminState::Up, CarrierState::Up, LinkState::Up),
            (AdminState::Up, CarrierState::Unknown, LinkState::Up),
            (AdminState::Up, CarrierState::Dormant, LinkState::Dormant),
            (AdminState::Up, CarrierState::Down, LinkState::NoCarrier),
            (AdminState::Down, CarrierState::Up, LinkState::Down),
            (AdminState::Down, CarrierState::Unknown, LinkState::Down),
            (AdminState::Down, CarrierState::Dormant, LinkState::Down),
            (AdminState::Down, CarrierState::Down, LinkState::Down),
        ];

        // ACT / ASSERT
        for (admin, carrier, expected) in cases {
            assert_eq!(LinkState::from_parts(admin, carrier), expected);
        }
    }

    #[test]
    fn parts_roundtrip_through_state() {
        // ARRANGE
        let states = [
            LinkState::Up,
            LinkState::NoCarrier,
            LinkState::Dormant,
            LinkState::Down,
        ];

        // ACT / ASSERT
        for state in states {
            assert_eq!(LinkState::from_parts(state.admin(), state.carrier()), state);
        }
    }

    #[test]
    fn admin_and_carrier_are_reported_separately() {
        // ACT / ASSERT
        assert_eq!(LinkState::Up.admin(), AdminState::Up);
        assert_eq!(LinkState::Up.carrier(), CarrierState::Up);
        assert_eq!(LinkState::NoCarrier.admin(), AdminState::Up);
        assert_eq!(LinkState::NoCarrier.carrier(), CarrierState::Down);
        assert_eq!(LinkState::Dormant.admin(), AdminState::Up);
        assert_eq!(LinkState::Dormant.carrier(), CarrierState::Dormant);
        assert_eq!(LinkState::Down.admin(), AdminState::Down);
        assert_eq!(LinkState::Down.carrier(), CarrierState::Unknown);
    }

    #[test]
    fn is_admin_up_excludes_only_down() {
        // ACT / ASSERT
        assert!(LinkState::Up.is_admin_up());
        assert!(LinkState::NoCarrier.is_admin_up());
        assert!(LinkState::Dormant.is_admin_up());
        assert!(!LinkState::Down.is_admin_up());
    }

    #[test]
    fn has_carrier_only_for_up() {
        // ACT / ASSERT
        assert!(LinkState::Up.has_carrier());
        assert!(!LinkState::NoCarrier.has_carrier());
        assert!(!LinkState::Dormant.has_carrier());
        assert!(!LinkState::Down.has_carrier());
    }

    #[test]
    fn display_matches_state() {
        // ACT / ASSERT
        assert_eq!(LinkState::Up.to_string(), "up");
        assert_eq!(LinkState::NoCarrier.to_string(), "no-carrier");
        assert_eq!(LinkState::Dormant.to_string(), "dormant");
        assert_eq!(LinkState::Down.to_string(), "down");
    }

    #[test]
    fn defaults_match_the_unknown_link() {
        // ACT / ASSERT
        assert_eq!(AdminState::default(), AdminState::Down);
        assert_eq!(CarrierState::default(), CarrierState::Unknown);
    }

    #[test]
    fn variants_are_distinct() {
        // ACT / ASSERT
        assert_eq!(LinkState::Up, LinkState::Up);
        assert_ne!(LinkState::Up, LinkState::Dormant);
        assert_ne!(LinkState::Dormant, LinkState::NoCarrier);
        assert_ne!(LinkState::NoCarrier, LinkState::Down);
    }
}
