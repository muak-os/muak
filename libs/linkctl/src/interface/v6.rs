//! IPv6 address method driven by the interface controller.

use std::time::Instant;

use netdev::interface::Ethernet;

use crate::icmpv6::Socket;
use crate::{family, view};

/// IPv6 family adapter for the interface controller.
pub struct Family;

/// IPv6 address method the controller drives.
pub enum Method {
    /// Static address method.
    Static(addrconf::v6::Static),
    /// Stateless address autoconfiguration.
    Slaac(slaac::method::Slaac<Socket>),
}

impl family::Family for Family {
    const ENABLED: bool = true;

    type Config = addrconf::v6::Config;
    type View = view::V6State;
    type Method = Method;

    fn configure(
        config: addrconf::v6::Config,
        link: &Ethernet,
    ) -> impl Future<Output = Result<Method, String>> + Send {
        core::future::ready(Method::from_config(config, link).map_err(|error| error.to_string()))
    }
}

impl addrconf::Method for Method {
    type View = view::V6State;

    async fn next_event(&mut self, now: Instant) -> addrconf::Event {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::next_event(method, now).await,
            Self::Slaac(ref mut method) => addrconf::Method::next_event(method, now).await,
        }
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        match *self {
            Self::Static(ref method) => addrconf::Method::next_wake(method, now),
            Self::Slaac(ref method) => addrconf::Method::next_wake(method, now),
        }
    }

    fn on_command(&mut self, command: addrconf::Command) {
        match *self {
            Self::Static(ref mut method) => addrconf::Method::on_command(method, command),
            Self::Slaac(ref mut method) => addrconf::Method::on_command(method, command),
        }
    }

    fn view(&self) -> Option<view::V6State> {
        match *self {
            Self::Static(ref method) => addrconf::Method::view(method).map(view::V6State::Static),
            Self::Slaac(ref method) => addrconf::Method::view(method).map(view::V6State::Slaac),
        }
    }
}

impl Method {
    fn from_config(config: addrconf::v6::Config, link: &Ethernet) -> Result<Self, std::io::Error> {
        if !config.addresses.is_empty() {
            return Ok(Self::Static(addrconf::v6::Static::new(config.fixed())));
        }
        if config.autoconf {
            let socket = Socket::open(link.name.as_str())?;
            let interface = link.name.as_str().to_owned();
            return Ok(Self::Slaac(slaac::method::Slaac::new(
                config,
                interface,
                link.mac_address,
                socket,
            )));
        }

        Ok(Self::Static(addrconf::v6::Static::new(config.fixed())))
    }
}
