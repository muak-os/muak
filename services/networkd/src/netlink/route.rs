use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use tokio_stream::StreamExt;

pub async fn find_default_gateway(handle: &Handle) -> Result<Option<Ipv4Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await? {
        let mut is_default = route.header.destination_prefix_length == 0;
        let mut gateway = None;

        for attr in &route.attributes {
            if let RouteAttribute::Destination(RouteAddress::Inet(addr)) = attr
                && !addr.is_unspecified()
            {
                is_default = false;
            }
            if let RouteAttribute::Gateway(RouteAddress::Inet(gw)) = attr {
                gateway = Some(*gw);
            }
        }

        if is_default && let Some(gw) = gateway {
            return Ok(Some(gw));
        }
    }

    Ok(None)
}

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
        .context("failed to add default route")
}

pub async fn ensure_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    if let Some(existing_gw) = find_default_gateway(handle).await?
        && existing_gw == gateway
    {
        return Ok(());
    }

    add_default_route(handle, gateway).await
}

// ===========================================================================
// IPv6 Route Operations
// ===========================================================================

pub async fn find_default_gateway_v6(handle: &Handle) -> Result<Option<Ipv6Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await? {
        let mut is_default = route.header.destination_prefix_length == 0;
        let mut gateway = None;

        for attr in &route.attributes {
            if let RouteAttribute::Destination(RouteAddress::Inet6(addr)) = attr
                && !addr.is_unspecified()
            {
                is_default = false;
            }
            if let RouteAttribute::Gateway(RouteAddress::Inet6(gw)) = attr {
                gateway = Some(*gw);
            }
        }

        if is_default && let Some(gw) = gateway {
            return Ok(Some(gw));
        }
    }

    Ok(None)
}

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
        .context("failed to add IPv6 default route")
}

pub async fn ensure_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    if let Some(existing_gw) = find_default_gateway_v6(handle).await?
        && existing_gw == gateway
    {
        return Ok(());
    }

    add_default_route_v6(handle, gateway).await
}

pub async fn remove_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await? {
        let is_default = route.header.destination_prefix_length == 0;
        let mut route_gateway = None;

        for attr in &route.attributes {
            if let RouteAttribute::Gateway(RouteAddress::Inet6(gw)) = attr {
                route_gateway = Some(*gw);
            }
        }

        if is_default && route_gateway == Some(gateway) {
            handle
                .route()
                .del(route)
                .execute()
                .await
                .context("failed to remove IPv6 default route")?;
            return Ok(());
        }
    }

    // Route not found - this is not an error
    Ok(())
}
