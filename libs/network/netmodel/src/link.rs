//! Link snapshot helpers.

use crate::Link;

impl Link {
    /// Returns true when the underlying link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub const fn has_carrier(&self) -> bool {
        self.state.has_carrier()
    }
}
