//! IPv4 and IPv6 default route management via rtnetlink.

use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr};

use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use thiserror::Error;
use tokio_stream::StreamExt;

use crate::ops::RtnetlinkOps;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to add default route: {0}")]
    AddDefaultRoute(#[source] rtnetlink::Error),
    #[error("failed to add IPv6 default route: {0}")]
    AddDefaultRouteV6(#[source] rtnetlink::Error),
    #[error("failed to remove IPv6 default route: {0}")]
    RemoveDefaultRouteV6(#[source] rtnetlink::Error),
    #[error("failed to enumerate routes: {0}")]
    List(#[source] rtnetlink::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Adds an IPv4 default route via the given gateway.
pub(crate) async fn add_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    handle
        .route()
        .add(
            RouteMessageBuilder::<Ipv4Addr>::new()
                .gateway(gateway)
                .build(),
        )
        .execute()
        .await
        .map_err(Error::AddDefaultRoute)
}

fn extract_ipv4_gateway(attrs: &[RouteAttribute]) -> (bool, Option<Ipv4Addr>) {
    let mut has_nonzero_dest = false;
    let mut gateway = None;
    for attr in attrs {
        if let RouteAttribute::Destination(RouteAddress::Inet(addr)) = attr
            && !addr.is_unspecified()
        {
            has_nonzero_dest = true;
        }
        if let RouteAttribute::Gateway(RouteAddress::Inet(gw)) = attr {
            gateway = Some(*gw);
        }
    }
    (has_nonzero_dest, gateway)
}

fn extract_ipv6_gateway(attrs: &[RouteAttribute]) -> (bool, Option<Ipv6Addr>) {
    let mut has_nonzero_dest = false;
    let mut gateway = None;
    for attr in attrs {
        if let RouteAttribute::Destination(RouteAddress::Inet6(addr)) = attr
            && !addr.is_unspecified()
        {
            has_nonzero_dest = true;
        }
        if let RouteAttribute::Gateway(RouteAddress::Inet6(gw)) = attr {
            gateway = Some(*gw);
        }
    }
    (has_nonzero_dest, gateway)
}

/// Trait covering all route-layer netlink operations.
pub trait RouteOps: Clone + Send + Sync + 'static {
    /// Adds or confirms the default IPv4 route via a gateway.
    fn ensure_default_route(&self, gateway: Ipv4Addr) -> impl Future<Output = Result<()>> + Send;

    /// Adds or confirms the default IPv6 route via a gateway.
    fn ensure_default_route_v6(&self, gateway: Ipv6Addr)
    -> impl Future<Output = Result<()>> + Send;

    /// Removes the default IPv6 route via a gateway.
    fn remove_default_route_v6(&self, gateway: Ipv6Addr)
    -> impl Future<Output = Result<()>> + Send;
}

impl RouteOps for RtnetlinkOps {
    async fn ensure_default_route(&self, gateway: Ipv4Addr) -> Result<()> {
        ensure_default_route(&self.handle, gateway).await
    }

    async fn ensure_default_route_v6(&self, gateway: Ipv6Addr) -> Result<()> {
        ensure_default_route_v6(&self.handle, gateway).await
    }

    async fn remove_default_route_v6(&self, gateway: Ipv6Addr) -> Result<()> {
        remove_default_route_v6(&self.handle, gateway).await
    }
}

async fn ensure_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();
    while let Some(route) = routes.try_next().await.map_err(Error::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (has_nonzero_dest, existing_gw) = extract_ipv4_gateway(&route.attributes);
        if is_default && !has_nonzero_dest && existing_gw == Some(gateway) {
            return Ok(());
        }
    }
    add_default_route(handle, gateway).await
}

async fn ensure_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();
    while let Some(route) = routes.try_next().await.map_err(Error::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (has_nonzero_dest, existing_gw) = extract_ipv6_gateway(&route.attributes);
        if is_default && !has_nonzero_dest && existing_gw == Some(gateway) {
            return Ok(());
        }
    }
    handle
        .route()
        .add(
            RouteMessageBuilder::<Ipv6Addr>::new()
                .gateway(gateway)
                .build(),
        )
        .execute()
        .await
        .map_err(Error::AddDefaultRouteV6)
}

async fn remove_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();
    while let Some(route) = routes.try_next().await.map_err(Error::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (_, route_gateway) = extract_ipv6_gateway(&route.attributes);
        if is_default && route_gateway == Some(gateway) {
            handle
                .route()
                .del(route)
                .execute()
                .await
                .map_err(Error::RemoveDefaultRouteV6)?;
            return Ok(());
        }
    }
    Ok(())
}
