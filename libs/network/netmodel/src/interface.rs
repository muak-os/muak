//! Address family and method state rendering.

use core::fmt;

use crate::{Family, MethodState};

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::V4 => f.write_str("IPv4"),
            Self::V6 => f.write_str("IPv6"),
        }
    }
}

impl fmt::Display for MethodState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Acquiring => f.write_str("acquiring"),
            Self::Bound => f.write_str("bound"),
            Self::Degraded => f.write_str("degraded"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Family, MethodState};

    #[test]
    fn family_display_names_the_protocol() {
        // ACT / ASSERT
        assert_eq!(Family::V4.to_string(), "IPv4");
        assert_eq!(Family::V6.to_string(), "IPv6");
    }

    #[test]
    fn method_state_display_is_lowercase() {
        // ACT / ASSERT
        assert_eq!(MethodState::Acquiring.to_string(), "acquiring");
        assert_eq!(MethodState::Bound.to_string(), "bound");
        assert_eq!(MethodState::Degraded.to_string(), "degraded");
        assert_eq!(MethodState::Stopped.to_string(), "stopped");
    }
}
