//! FIFO journal of observations waiting to be rendered.

use alloc::collections::VecDeque;

use crate::{Event, Level, Observation};

/// FIFO journal of observations waiting to be rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journal {
    entries: VecDeque<Observation>,
}

impl Journal {
    /// Creates an empty journal.
    #[must_use = "the journal owns the observations"]
    pub const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Appends one observation.
    pub fn record(&mut self, level: Level, event: Event) {
        self.entries.push_back(Observation { level, event });
    }

    /// Iterates over the observations in recording order and removes them.
    #[must_use = "unconsumed observations accumulate in the journal"]
    pub fn drain(&mut self) -> impl Iterator<Item = Observation> + '_ {
        self.entries.drain(..)
    }

    /// Returns true when no observation is recorded.
    #[must_use = "callers need to know whether there is anything to render"]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of recorded observations.
    #[must_use = "callers need the backlog size"]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::Journal;
    use crate::{EffectKind, Event, Level, SupervisorState};

    fn name(value: &str) -> netmodel::Name {
        netmodel::Name::new(value).expect("valid name")
    }

    #[test]
    fn new_journal_is_empty() {
        // ARRANGE
        let journal = Journal::new();

        // ACT
        let empty = journal.is_empty();
        let len = journal.len();

        // ASSERT
        assert!(empty);
        assert_eq!(len, 0);
    }

    #[test]
    fn record_keeps_order_and_levels() {
        // ARRANGE
        let mut journal = Journal::new();
        let state = Event::StateChanged {
            from: SupervisorState::Uninitialized,
            to: SupervisorState::Initializing,
        };
        let effect = Event::EffectStarted {
            kind: EffectKind::Discover,
            target: None,
        };

        // ACT
        journal.record(Level::Info, state.clone());
        journal.record(Level::Debug, effect.clone());
        let recorded = journal.drain().collect::<Vec<_>>();

        // ASSERT
        assert_eq!(journal.len(), 0);
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].level, Level::Info);
        assert_eq!(recorded[0].event, state);
        assert_eq!(recorded[1].level, Level::Debug);
        assert_eq!(recorded[1].event, effect);
    }

    #[test]
    fn drain_empties_the_journal() {
        // ARRANGE
        let mut journal = Journal::new();
        journal.record(Level::Info, Event::Detached { port: name("eth0") });

        // ACT
        let first = journal.drain().count();
        let second = journal.drain().count();

        // ASSERT
        assert_eq!(first, 1);
        assert_eq!(second, 0);
        assert!(journal.is_empty());
    }
}
