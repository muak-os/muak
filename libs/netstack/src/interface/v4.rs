//! IPv4 address method driven by the interface controller.

use netdev::interface::Ethernet;

use crate::transport::dhcp::System;
use crate::{provision, view};

/// IPv4 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(provision::v4::Static),
    /// `DHCPv4` address method.
    Dhcp(provision::v4::Dhcp<System>),
}

impl Method {
    /// Creates the method matching the given configuration and link.
    ///
    /// # Errors
    ///
    /// Returns an error when the `DHCPv4` raw socket cannot be opened on `link`.
    pub async fn from_config(
        config: provision::v4::Config,
        link: &Ethernet,
    ) -> Result<Self, provision::Failure> {
        if config.dhcp {
            let exchange = System::open(link.name.as_str())
                .await
                .map_err(|error| provision::Failure::Protocol(error.to_string()))?;
            let method = provision::v4::Dhcp::new(exchange, link.name.as_str(), link.mac_address);
            return Ok(Self::Dhcp(method));
        }

        Ok(Self::Static(provision::v4::Static::new(config.fixed())))
    }

    /// Returns the next event produced by the method.
    pub async fn next_event(&mut self) -> provision::Event {
        match *self {
            Self::Static(ref mut method) => provision::Method::next_event(method).await,
            Self::Dhcp(ref mut method) => provision::Method::next_event(method).await,
        }
    }

    /// Applies a command from the controller.
    pub fn on_command(&mut self, command: provision::Command) {
        match *self {
            Self::Static(ref mut method) => provision::Method::on_command(method, command),
            Self::Dhcp(ref mut method) => provision::Method::on_command(method, command),
        }
    }

    /// Returns the method's current view, or `None` when it has nothing to report.
    #[must_use = "callers publish the view in the interface snapshot"]
    pub fn view(&self) -> Option<view::V4State> {
        match *self {
            Self::Static(ref method) => provision::Method::view(method),
            Self::Dhcp(ref method) => provision::Method::view(method),
        }
    }
}
