//! Administrative and carrier state of a network link.

use core::fmt;

/// Administrative and carrier state of a network link.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Link is administratively up with carrier detected.
    Up,
    /// Link is administratively up but no carrier detected.
    NoCarrier,
    /// Link is administratively down.
    Down,
}

impl State {
    /// Returns true when the link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub fn has_carrier(&self) -> bool {
        *self == Self::Up
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Up => write!(f, "up"),
            Self::NoCarrier => write!(f, "no-carrier"),
            Self::Down => write!(f, "down"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::State as LinkStateKind;

    #[test]
    fn link_state_kind_has_carrier_up() {
        // ACT / ASSERT
        assert!(LinkStateKind::Up.has_carrier());
    }

    #[test]
    fn link_state_kind_has_carrier_no_carrier() {
        // ACT / ASSERT
        assert!(!LinkStateKind::NoCarrier.has_carrier());
    }

    #[test]
    fn link_state_kind_has_carrier_down() {
        // ACT / ASSERT
        assert!(!LinkStateKind::Down.has_carrier());
    }

    #[test]
    fn link_state_kind_display_up() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Up.to_string(), "up");
    }

    #[test]
    fn link_state_kind_display_no_carrier() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::NoCarrier.to_string(), "no-carrier");
    }

    #[test]
    fn link_state_kind_display_down() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Down.to_string(), "down");
    }

    #[test]
    fn link_state_kind_equality() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Up, LinkStateKind::Up);
        assert_ne!(LinkStateKind::Up, LinkStateKind::Down);
        assert_ne!(LinkStateKind::NoCarrier, LinkStateKind::Down);
    }
}
