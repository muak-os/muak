//! Generic state machine trait for validated state transitions.

use std::fmt;

#[derive(Debug, Clone)]
pub struct InvalidTransition<S: fmt::Display> {
    pub from: S,
    pub to: S,
}

impl<S: fmt::Display> fmt::Display for InvalidTransition<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid state transition: {} -> {}", self.from, self.to)
    }
}

impl<S: fmt::Debug + fmt::Display> std::error::Error for InvalidTransition<S> {}

/// Enforces a finite set of valid transitions between states.
pub trait StateMachine: Sized + Clone + PartialEq + fmt::Display + 'static {
    /// Returns the set of states reachable from the current state.
    fn valid_next_states(&self) -> &'static [Self];

    /// Advances to `to` if the transition is valid, otherwise returns an error.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestState {
        A,
        B,
        C,
    }

    impl fmt::Display for TestState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::A => write!(f, "A"),
                Self::B => write!(f, "B"),
                Self::C => write!(f, "C"),
            }
        }
    }

    impl StateMachine for TestState {
        fn valid_next_states(&self) -> &'static [Self] {
            match self {
                Self::A => &[Self::B],
                Self::B => &[Self::A, Self::C],
                Self::C => &[],
            }
        }
    }

    #[test]
    fn valid_transition_advances_state() {
        // ARRANGE
        let mut state = TestState::A;

        // ACT
        let result = state.transition(TestState::B);

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(state, TestState::B);
    }

    #[test]
    fn invalid_transition_returns_error() {
        // ARRANGE
        let mut state = TestState::A;

        // ACT
        let result = state.transition(TestState::C);

        // ASSERT
        assert!(result.is_err());
        assert_eq!(state, TestState::A);
    }

    #[test]
    fn terminal_state_rejects_all_transitions() {
        // ARRANGE
        let mut state = TestState::C;

        // ACT / ASSERT
        assert!(state.transition(TestState::A).is_err());
        assert!(state.transition(TestState::B).is_err());
        assert_eq!(state, TestState::C);
    }

    #[test]
    fn error_message_includes_states() {
        // ARRANGE
        let mut state = TestState::A;

        // ACT
        let err = state.transition(TestState::C).unwrap_err();

        // ASSERT
        assert_eq!(err.to_string(), "invalid state transition: A -> C");
    }

    #[test]
    fn bidirectional_transition_works() {
        // ARRANGE
        let mut state = TestState::A;

        // ACT
        state.transition(TestState::B).unwrap();
        state.transition(TestState::A).unwrap();

        // ASSERT
        assert_eq!(state, TestState::A);
    }

    #[test]
    fn invalid_transition_implements_std_error() {
        // ARRANGE
        let mut state = TestState::A;
        let err = state.transition(TestState::C).unwrap_err();

        // ACT
        let dyn_err: &dyn std::error::Error = &err;

        // ASSERT
        assert!(dyn_err.source().is_none());
        assert_eq!(dyn_err.to_string(), "invalid state transition: A -> C");
    }
}
