//! Commands accepted by a per-interface actor.

use std::net::Ipv4Addr;

use anyhow::Result;
use tokio::sync::oneshot;

use crate::interface::snapshot::InterfaceSnapshot;
use crate::slaac::SlaacEvent;

/// Distinguishes which DHCP timer phase triggered a lease action.
#[derive(Debug)]
pub enum LeaseAction {
    Renew,
    Rebind,
    Expired,
}

#[derive(Debug)]
pub enum InterfaceCommand {
    ConfigureDhcp {
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    ConfigureStaticIpv4 {
        index: u32,
        addresses: Vec<config::Cidr4>,
        gateway: Option<Ipv4Addr>,
    },
    ConfigureStaticIpv6 {
        index: u32,
        addresses: Vec<config::Cidr6>,
        gateway: Option<std::net::Ipv6Addr>,
    },
    ConfigureBridge {
        bridge_name: String,
        stp: bool,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    LeaseAction(LeaseAction),
    StartSlaac,
    Slaac(SlaacEvent),
    LinkUp,
    LinkDown,
    Shutdown,
}
