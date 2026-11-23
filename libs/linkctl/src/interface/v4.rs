//! IPv4 address method driven by the interface controller.

use std::time::Instant;

use dhcp_client::method::Dhcp;
use dhcp_client::system::{Exchange, Kernel};
use netdev::interface::Ethernet;

use crate::{family, view};

/// IPv4 family adapter for the interface controller.
pub struct Family;

/// IPv4 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(addrconf::v4::Static),
    /// `DHCPv4` address method.
    Dhcp(Dhcp<Exchange<Kernel>>),
}

impl family::Family for Family {
    const ENABLED: bool = true;

    type Config = addrconf::v4::Config;
    type View = view::V4State;
    type Method = Method;

    async fn configure(config: addrconf::v4::Config, link: &Ethernet) -> Result<Method, String> {
        if config.dhcp {
            let exchange = Exchange::open(link.name.as_str())
                .await
                .map_err(|error| addrconf::Failure::Protocol(error.to_string()).to_string())?;
            let method = Dhcp::new(exchange, link.name.as_str(), link.mac_address);
            return Ok(Method::Dhcp(method));
        }

        Ok(Method::Static(addrconf::v4::Static::new(config.fixed())))
    }
}

impl addrconf::Method for Method {
    type View = view::V4State;

    async fn next_event(&mut self, now: Instant) -> addrconf::Event {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::next_event(method, now).await,
            Self::Dhcp(ref mut method) => addrconf::Method::next_event(method, now).await,
        }
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        match *self {
            Self::Static(ref method) => addrconf::Method::next_wake(method, now),
            Self::Dhcp(ref method) => addrconf::Method::next_wake(method, now),
        }
    }

    fn on_command(&mut self, command: addrconf::Command) {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::on_command(method, command),
            Self::Dhcp(ref mut method) => addrconf::Method::on_command(method, command),
        }
    }

    fn view(&self) -> Option<view::V4State> {
        match *self {
            Self::Static(ref method) => addrconf::Method::view(method).map(view::V4State::Static),
            Self::Dhcp(ref method) => addrconf::Method::view(method).map(view::V4State::Dhcp),
        }
    }
}
