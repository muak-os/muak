//! Per-family method composition and construction from configuration.

use alloc::string::String;
use alloc::vec::Vec;
use std::time::{Instant, SystemTime};

use ip_method::{Input, LinkInfo, Method, Step};

use crate::{V4View, V6View};

#[cfg(feature = "ipv4")]
type DhcpMethod = dhcp::Dhcp;
#[cfg(not(feature = "ipv4"))]
type DhcpMethod = core::convert::Infallible;

#[cfg(feature = "ipv6")]
type SlaacMethod = slaac::Slaac;
#[cfg(not(feature = "ipv6"))]
type SlaacMethod = core::convert::Infallible;

/// Exactly one IPv4 provisioning method active on the interface.
pub(crate) struct V4Slot {
    static_method: Option<ip_static::Static>,
    dhcp: Option<DhcpMethod>,
}

/// Exactly one IPv6 provisioning method active on the interface.
pub(crate) struct V6Slot {
    static_method: Option<ip_static::Static>,
    slaac: Option<SlaacMethod>,
}

/// Methods a deferred reconfigure installs once the outgoing ones stop.
pub(crate) struct Replacement {
    pub(crate) v4: Option<V4Slot>,
    pub(crate) v6: Option<V6Slot>,
    pub(crate) notices: Vec<String>,
}

impl V4Slot {
    /// Builds the IPv4 method the configuration asks for and reports static overflow.
    pub(crate) fn new(link: &LinkInfo, config: netmodel::Ipv4Config) -> (Option<Self>, bool) {
        if config.addresses.is_empty() {
            return (dhcp(link, config), false);
        }

        let static_config = ip_static::Config::new(&config.addresses, config.gateway, &config.dns);
        match static_config {
            Some(config) => (Some(Self::fixed(ip_static::Static::new(config))), false),
            None => (None, true),
        }
    }

    /// Returns whether the slot owns a DHCP method.
    pub(crate) const fn is_dhcp(&self) -> bool {
        self.dhcp.is_some()
    }

    const fn fixed(method: ip_static::Static) -> Self {
        Self {
            static_method: Some(method),
            dhcp: None,
        }
    }

    #[cfg(feature = "ipv4")]
    fn dhcp_method(link: &LinkInfo, config: netmodel::Ipv4Config) -> Self {
        Self {
            static_method: None,
            dhcp: Some(dhcp::Dhcp::new(link.clone(), config)),
        }
    }
}

impl V6Slot {
    /// Builds the IPv6 method the configuration asks for and reports static overflow.
    pub(crate) fn new(link: &LinkInfo, config: netmodel::Ipv6Config) -> (Option<Self>, bool) {
        if config.addresses.is_empty() {
            return (stateless(link, config), false);
        }

        let static_config = ip_static::Config::new(&config.addresses, config.gateway, &config.dns);
        match static_config {
            Some(config) => (Some(Self::fixed(ip_static::Static::new(config))), false),
            None => (None, true),
        }
    }

    /// Returns whether the slot owns a SLAAC method.
    pub(crate) const fn is_stateless(&self) -> bool {
        self.slaac.is_some()
    }

    const fn fixed(method: ip_static::Static) -> Self {
        Self {
            static_method: Some(method),
            slaac: None,
        }
    }

    #[cfg(feature = "ipv6")]
    fn stateless_method(link: &LinkInfo, config: netmodel::Ipv6Config) -> Self {
        Self {
            static_method: None,
            slaac: Some(slaac::Slaac::new(link.clone(), config)),
        }
    }
}

impl Method for V4Slot {
    type View = V4View;

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        if let Some(method) = self.static_method.as_ref() {
            return method.next_wake(now);
        }
        #[cfg(feature = "ipv4")]
        if let Some(method) = self.dhcp.as_ref() {
            return method.next_wake(now);
        }

        None
    }

    fn step(&mut self, now: Instant, wall: SystemTime, input: Input<'_>) -> Step<'_> {
        if let Some(method) = self.static_method.as_mut() {
            return method.step(now, wall, input);
        }
        #[cfg(feature = "ipv4")]
        if let Some(method) = self.dhcp.as_mut() {
            return method.step(now, wall, input);
        }

        Step::Wait
    }

    fn view(&self) -> Option<V4View> {
        if let Some(method) = self.static_method.as_ref() {
            return method.view().map(V4View::Static);
        }
        #[cfg(feature = "ipv4")]
        if let Some(method) = self.dhcp.as_ref() {
            return method.view().map(V4View::Dhcp);
        }

        None
    }
}

impl Method for V6Slot {
    type View = V6View;

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        if let Some(method) = self.static_method.as_ref() {
            return method.next_wake(now);
        }
        #[cfg(feature = "ipv6")]
        if let Some(method) = self.slaac.as_ref() {
            return method.next_wake(now);
        }

        None
    }

    fn step(&mut self, now: Instant, wall: SystemTime, input: Input<'_>) -> Step<'_> {
        if let Some(method) = self.static_method.as_mut() {
            return method.step(now, wall, input);
        }
        #[cfg(feature = "ipv6")]
        if let Some(method) = self.slaac.as_mut() {
            return method.step(now, wall, input);
        }

        Step::Wait
    }

    fn view(&self) -> Option<V6View> {
        if let Some(method) = self.static_method.as_ref() {
            return method.view().map(V6View::Static);
        }
        #[cfg(feature = "ipv6")]
        if let Some(method) = self.slaac.as_ref() {
            return method.view().map(V6View::Slaac);
        }

        None
    }
}

#[cfg(feature = "ipv4")]
fn dhcp(link: &LinkInfo, config: netmodel::Ipv4Config) -> Option<V4Slot> {
    config.dhcp.then(|| V4Slot::dhcp_method(link, config))
}

#[cfg(not(feature = "ipv4"))]
fn dhcp(_link: &LinkInfo, _config: netmodel::Ipv4Config) -> Option<V4Slot> {
    None
}

#[cfg(feature = "ipv6")]
fn stateless(link: &LinkInfo, config: netmodel::Ipv6Config) -> Option<V6Slot> {
    config
        .autoconf
        .then(|| V6Slot::stateless_method(link, config))
}

#[cfg(not(feature = "ipv6"))]
fn stateless(_link: &LinkInfo, _config: netmodel::Ipv6Config) -> Option<V6Slot> {
    None
}
