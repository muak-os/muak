//! Commands accepted by a per-interface actor.

use core::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;

use anyhow::Result;
use config::system::network;
use tokio::sync::oneshot;

use crate::interface::snapshot::Snapshot;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ApplyMode {
    Provision,
    Reconcile,
}

#[derive(Debug)]
pub enum Command {
    ConfigureDhcp {
        mode: ApplyMode,
    },
    ConfigureStaticIpv4 {
        mode: ApplyMode,
        index: u32,
        addresses: Vec<network::Cidr4>,
        gateway: Option<Ipv4Addr>,
    },
    #[cfg(feature = "ipv6")]
    ConfigureStaticIpv6 {
        mode: ApplyMode,
        index: u32,
        addresses: Vec<network::Cidr6>,
        gateway: Option<Ipv6Addr>,
    },
    ConfigureBridge {
        bridge_name: String,
        stp: bool,
        reply: oneshot::Sender<Result<Snapshot>>,
    },
    #[cfg(feature = "ipv6")]
    ConfigureSlaac {
        mode: ApplyMode,
    },
    LinkUp,
    LinkDown,
    Shutdown,
}
