//! IPv6 address method driven by the interface controller.

use netdev::interface::Ethernet;

use crate::{provision, view};

/// IPv6 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(provision::v6::Static),
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
        config: provision::v6::Config,
    ) -> Result<Self, std::io::Error> {
        if !config.addresses.is_empty() {
            return Ok(Self::Static(provision::v6::Static::new(config)));
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
        Ok(Self::Static(provision::v6::Static::new(config)))
    }

    /// Returns the next event produced by the method.
    pub async fn next_event(&mut self) -> provision::Event {
        match *self {
            Self::Static(ref mut method) => provision::Method::next_event(method).await,
            Self::Slaac(ref mut method) => provision::Method::next_event(method).await,
        }
    }

    /// Applies a command from the controller.
    pub fn on_command(&mut self, command: provision::Command) {
        match *self {
            Self::Static(ref mut method) => provision::Method::on_command(method, command),
            Self::Slaac(ref mut method) => provision::Method::on_command(method, command),
        }
    }

    /// Returns the method's current view, or `None` when it has nothing to report.
    #[must_use = "callers publish the view in the interface snapshot"]
    pub fn view(&self) -> Option<view::V6State> {
        match *self {
            Self::Static(ref method) => provision::Method::view(method),
            Self::Slaac(ref method) => provision::Method::view(method),
        }
    }
}
