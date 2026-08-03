//! Generic state machine trait for validated state transitions.

use core::fmt;

#[derive(Debug, Clone)]
pub struct InvalidTransition<S: fmt::Display> {
    pub from: S,
    pub to: S,
}

/// Enforces a finite set of valid transitions between states.
pub trait StateMachine: Sized + Clone + PartialEq + fmt::Display + 'static {
    /// Returns the set of states reachable from the current state.
    fn valid_next_states(&self) -> &'static [Self];

    /// Advances to `to` if the transition is valid, otherwise returns an error.
    ///
    /// # Errors
    ///
    /// Returns `InvalidTransition` when moving from the current state to `to` is not allowed.
    fn transition(&mut self, to: Self) -> Result<(), InvalidTransition<Self>> {
        if self.valid_next_states().contains(&to) {
            *self = to;
            Ok(())
        } else {
            Err(InvalidTransition {
                from: self.clone(),
                to,
            })
        }
    }
}

impl<S: fmt::Display> fmt::Display for InvalidTransition<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid state transition: {} -> {}", self.from, self.to)
    }
}

impl<S: fmt::Debug + fmt::Display> core::error::Error for InvalidTransition<S> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestState {
        Alpha,
        Beta,
        Gamma,
    }

    impl fmt::Display for TestState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match *self {
                Self::Alpha => write!(f, "Alpha"),
                Self::Beta => write!(f, "Beta"),
                Self::Gamma => write!(f, "Gamma"),
            }
        }
    }

    impl StateMachine for TestState {
        fn valid_next_states(&self) -> &'static [Self] {
            match *self {
                Self::Alpha => &[Self::Beta],
                Self::Beta => &[Self::Alpha, Self::Gamma],
                Self::Gamma => &[],
            }
        }
    }

    #[test]
    fn valid_transition_advances_state() {
        // ARRANGE
        let mut state = TestState::Alpha;

        // ACT
        let result = state.transition(TestState::Beta);

        // ASSERT
        result.unwrap();
        assert_eq!(state, TestState::Beta);
    }

    #[test]
    fn invalid_transition_returns_error() {
        // ARRANGE
        let mut state = TestState::Alpha;

        // ACT
        let result = state.transition(TestState::Gamma);

        // ASSERT
        assert!(result.is_err());
        assert_eq!(state, TestState::Alpha);
    }

    #[test]
    fn terminal_state_rejects_all_transitions() {
        // ARRANGE
        let mut state = TestState::Gamma;

        // ACT / ASSERT
        assert!(state.transition(TestState::Alpha).is_err());
        assert!(state.transition(TestState::Beta).is_err());
        assert_eq!(state, TestState::Gamma);
    }

    #[test]
    fn error_message_includes_states() {
        // ARRANGE
        let mut state = TestState::Alpha;

        // ACT
        let err = state.transition(TestState::Gamma).unwrap_err();

        // ASSERT
        assert_eq!(err.to_string(), "invalid state transition: Alpha -> Gamma");
    }

    #[test]
    fn bidirectional_transition_works() {
        // ARRANGE
        let mut state = TestState::Alpha;

        // ACT
        state.transition(TestState::Beta).unwrap();
        state.transition(TestState::Alpha).unwrap();

        // ASSERT
        assert_eq!(state, TestState::Alpha);
    }

    #[test]
    fn invalid_transition_implements_std_error() {
        // ARRANGE
        let mut state = TestState::Alpha;
        let err = state.transition(TestState::Gamma).unwrap_err();

        // ACT
        let dyn_err: &dyn core::error::Error = &err;

        // ASSERT
        assert!(dyn_err.source().is_none());
        assert_eq!(
            dyn_err.to_string(),
            "invalid state transition: Alpha -> Gamma"
        );
    }
}
