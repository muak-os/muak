//! Link snapshot helpers.

use crate::Link;

impl Link {
    /// Returns true when the underlying link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub const fn has_carrier(&self) -> bool {
        self.state.has_carrier()
    }

    /// Returns true when the name follows a physical Ethernet naming convention.
    ///
    /// Loopback, wireless, and unrecognised names are rejected so discovery
    /// only elects real wired uplinks.
    #[must_use = "callers must skip links that are not Ethernet"]
    pub fn is_ethernet(&self) -> bool {
        let name = self.name.as_str();
        if name == "lo" || name.starts_with("wlan") || name.starts_with("wlp") {
            return false;
        }
        name.starts_with("eth")
            || name.starts_with("enp")
            || name.starts_with("ens")
            || name.starts_with("eno")
            || name.starts_with("end")
    }
}
