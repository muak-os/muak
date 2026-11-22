//! IPv4 and IPv6 address management.

pub(crate) mod handle;

use core::future::Future;
use core::net::{Ipv4Addr, Ipv6Addr};

use thiserror::Error;

use crate::Rtnl;

/// Address management failure conditions.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to add IPv4 address.
    #[error("failed to add IPv4 address: {0}")]
    AddIpv4(#[source] rtnetlink::Error),
    /// Failed to remove IPv4 address.
    #[error("failed to remove IPv4 address: {0}")]
    RemoveIpv4(#[source] rtnetlink::Error),
    /// Failed to add IPv6 address.
    #[error("failed to add IPv6 address: {0}")]
    AddIpv6(#[source] rtnetlink::Error),
    /// Failed to remove IPv6 address.
    #[error("failed to remove IPv6 address: {0}")]
    RemoveIpv6(#[source] rtnetlink::Error),
    /// Failed to enumerate addresses.
    #[error("failed to enumerate addresses: {0}")]
    List(#[source] rtnetlink::Error),
}

/// Address management result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Trait covering all address-layer netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Adds or confirms an IPv4 address on an interface.
    fn add_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Adds or confirms an IPv4 address on an interface.
    fn ensure_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Adds or confirms an IPv6 address on an interface.
    fn ensure_ipv6(
        &self,
        index: u32,
        ip: Ipv6Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Returns the first IPv4 address on an interface, if any.
    fn find_ipv4(&self, index: u32) -> impl Future<Output = Result<Option<(Ipv4Addr, u8)>>> + Send;

    /// Returns whether an interface has at least one IPv4 address.
    fn has_ipv4(&self, index: u32) -> impl Future<Output = Result<bool>> + Send;

    /// Removes an IPv4 address from an interface.
    fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> impl Future<Output = Result<()>> + Send;

    /// Removes an IPv6 address from an interface.
    fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> impl Future<Output = Result<()>> + Send;
}

impl Ops for Rtnl {
    async fn ensure_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
        handle::ensure_ipv4(&self.handle, index, ip, prefix).await
    }

    async fn find_ipv4(&self, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
        handle::find_ipv4(&self.handle, index).await
    }

    async fn has_ipv4(&self, index: u32) -> Result<bool> {
        handle::has_ipv4(&self.handle, index).await
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
        handle::add_ipv4(&self.handle, index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> Result<()> {
        handle::remove_ipv4(&self.handle, index, ip).await
    }

    async fn ensure_ipv6(&self, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
        handle::ensure_ipv6(&self.handle, index, ip, prefix).await
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> Result<()> {
        handle::remove_ipv6(&self.handle, index, ip).await
    }
}
