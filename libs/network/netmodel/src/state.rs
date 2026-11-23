//! Link carrier state helpers.

use core::fmt;

use crate::LinkState;

impl LinkState {
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
            Self::Down => f.write_str("down"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::LinkState;

    #[test]
    fn has_carrier_only_for_up() {
        // ACT / ASSERT
        assert!(LinkState::Up.has_carrier());
        assert!(!LinkState::NoCarrier.has_carrier());
        assert!(!LinkState::Down.has_carrier());
    }

    #[test]
    fn display_matches_state() {
        // ACT / ASSERT
        assert_eq!(LinkState::Up.to_string(), "up");
        assert_eq!(LinkState::NoCarrier.to_string(), "no-carrier");
        assert_eq!(LinkState::Down.to_string(), "down");
    }

    #[test]
    fn variants_are_distinct() {
        // ACT / ASSERT
        assert_eq!(LinkState::Up, LinkState::Up);
        assert_ne!(LinkState::Up, LinkState::Down);
        assert_ne!(LinkState::NoCarrier, LinkState::Down);
    }
}
