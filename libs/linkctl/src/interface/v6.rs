//! IPv6 address method driven by the interface controller.

use std::time::Instant;

use netdev::interface::Ethernet;

use crate::{provision, view};

/// IPv6 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(addrconf::v6::Static),
    /// Stateless address autoconfiguration.
    Slaac(provision::v6::slaac::Slaac<packet::icmpv6::Socket>),
}

impl Method {
    /// Creates the method matching the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the `ICMPv6` socket needed by SLAAC cannot be opened.
    pub fn from_config(
        link: &Ethernet,
        config: addrconf::v6::Config,
    ) -> Result<Self, std::io::Error> {
        if !config.addresses.is_empty() {
            return Ok(Self::Static(addrconf::v6::Static::new(config.fixed())));
        }
        if config.autoconf {
            let socket = packet::icmpv6::Socket::open(
                link.name.as_str(),
                &[::slaac::icmpv6::ROUTER_ADVERTISEMENT],
            )
            .map_err(std::io::Error::other)?;
            let interface = link.name.as_str().to_owned();
            return Ok(Self::Slaac(provision::v6::slaac::Slaac::new(
                config,
                interface,
                link.mac_address,
                socket,
            )));
        }
        Ok(Self::Static(addrconf::v6::Static::new(config.fixed())))
    }

    /// Performs at most one I/O step and returns the next event.
    pub async fn next_event(&mut self, now: Instant) -> addrconf::Event {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::next_event(method, now).await,
            Self::Slaac(ref mut method) => addrconf::Method::next_event(method, now).await,
        }
    }

    /// Returns the earliest instant at which the method must be polled again.
    #[must_use = "callers arm the controller wake deadline with it"]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        match *self {
            Self::Static(ref method) => addrconf::Method::next_wake(method, now),
            Self::Slaac(ref method) => addrconf::Method::next_wake(method, now),
        }
    }

    /// Applies a command from the controller.
    pub fn on_command(&mut self, command: addrconf::Command) {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::on_command(method, command),
            Self::Slaac(ref mut method) => addrconf::Method::on_command(method, command),
        }
    }

    /// Returns the method's current view, or `None` when it has nothing to report.
    #[must_use = "callers publish the view in the interface snapshot"]
    pub fn view(&self) -> Option<view::V6State> {
        match *self {
            Self::Static(ref method) => addrconf::Method::view(method).map(view::V6State::Static),
            Self::Slaac(ref method) => addrconf::Method::view(method),
        }
    }
}
