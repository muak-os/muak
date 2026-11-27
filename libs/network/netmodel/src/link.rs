//! Link snapshot helpers.

use crate::{Link, LinkState};

impl Link {
    /// Returns the combined administrative and carrier state.
    #[must_use = "every readiness decision is derived from the link state"]
    pub const fn state(&self) -> LinkState {
        LinkState::from_parts(self.admin, self.carrier)
    }

    /// Returns true when the link is administratively up.
    #[must_use = "callers must skip administratively disabled links"]
    pub const fn is_admin_up(&self) -> bool {
        self.admin.is_up()
    }

    /// Returns true when the underlying link has an active carrier signal.
    #[must_use = "callers must skip links without a carrier"]
    pub const fn has_carrier(&self) -> bool {
        self.state().has_carrier()
    }

    /// Returns true when the name follows a physical Ethernet naming convention. Loopback, wireless, and unrecognised
    /// names are rejected.
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
