//! IPv4 and IPv6 default route management via rtnetlink.

use std::net::{Ipv4Addr, Ipv6Addr};

use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use thiserror::Error;
use tokio_stream::StreamExt;

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

/// Finds the current IPv4 default gateway, if one exists.
pub async fn find_default_gateway(handle: &Handle) -> Result<Option<Ipv4Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await.map_err(Error::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (has_nonzero_dest, gateway) = extract_ipv4_gateway(&route.attributes);

        if is_default
            && !has_nonzero_dest
            && let Some(gw) = gateway
        {
            return Ok(Some(gw));
        }
    }

    Ok(None)
}

/// Adds an IPv4 default route via the given gateway.
pub async fn add_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
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

/// Ensures the IPv4 default route points to the given gateway.
pub async fn ensure_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    if let Some(existing_gw) = find_default_gateway(handle).await?
        && existing_gw == gateway
    {
        return Ok(());
    }

    add_default_route(handle, gateway).await
}

/// Finds the current IPv6 default gateway, if one exists.
pub async fn find_default_gateway_v6(handle: &Handle) -> Result<Option<Ipv6Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await.map_err(Error::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (has_nonzero_dest, gateway) = extract_ipv6_gateway(&route.attributes);

        if is_default
            && !has_nonzero_dest
            && let Some(gw) = gateway
        {
            return Ok(Some(gw));
        }
    }

    Ok(None)
}

/// Adds an IPv6 default route via the given gateway.
pub async fn add_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
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

/// Ensures the IPv6 default route points to the given gateway.
pub async fn ensure_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    if let Some(existing_gw) = find_default_gateway_v6(handle).await?
        && existing_gw == gateway
    {
        return Ok(());
    }

    add_default_route_v6(handle, gateway).await
}

/// Removes the IPv6 default route via the given gateway (no-op if absent).
pub async fn remove_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
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
