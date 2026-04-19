//! Commands accepted by a per-interface actor.

use std::net::Ipv4Addr;

use anyhow::Result;
use tokio::sync::oneshot;

use crate::interface::snapshot::InterfaceSnapshot;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ApplyMode {
    Provision,
    Reconcile,
}

#[derive(Debug)]
pub enum InterfaceCommand {
    ConfigureDhcp {
        mode: ApplyMode,
    },
    ConfigureStaticIpv4 {
        mode: ApplyMode,
        index: u32,
        addresses: Vec<config::Cidr4>,
        gateway: Option<Ipv4Addr>,
    },
    ConfigureStaticIpv6 {
        mode: ApplyMode,
        index: u32,
        addresses: Vec<config::Cidr6>,
        gateway: Option<std::net::Ipv6Addr>,
    },
    ConfigureBridge {
        bridge_name: String,
        stp: bool,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    ConfigureSlaac {
        mode: ApplyMode,
    },
    LinkUp,
    LinkDown,
    Shutdown,
}
