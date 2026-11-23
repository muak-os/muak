//! Link snapshot integration tests.

pub mod common;

#[cfg(test)]
mod tests {
    use netmodel::LinkState;

    use crate::common;

    #[test]
    fn has_carrier_follows_state() {
        // ARRANGE
        let up = common::link("eth0", LinkState::Up);
        let down = common::link("eth0", LinkState::Down);

        // ACT / ASSERT
        assert!(up.has_carrier());
        assert!(!down.has_carrier());
    }
}
