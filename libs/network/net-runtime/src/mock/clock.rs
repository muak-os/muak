//! Clock and entropy port implementations for the deterministic mock runtime.

use super::{Mock, State};

impl crate::Clock for Mock {
    fn now(&self) -> std::time::Instant {
        self.read(|state| state.now)
    }

    fn wall_now(&self) -> std::time::SystemTime {
        self.read(|state| state.wall)
    }
}

impl crate::Entropy for Mock {
    fn fill(&self, bytes: &mut [u8]) {
        self.write(|state| fill(state, bytes));
    }
}

fn fill(state: &mut State, bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        *byte = state.entropy.to_le_bytes()[0];
        state.entropy = state.entropy.wrapping_add(1);
    }
}
