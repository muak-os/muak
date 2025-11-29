use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use netlink_packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use std::net::{Ipv4Addr, Ipv6Addr};

pub async fn find_default_gateway(handle: &Handle) -> Result<Option<Ipv4Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await? {
        let mut is_default = route.header.destination_prefix_length == 0;
        let mut gateway = None;

        for attr in &route.attributes {
            match attr {
                RouteAttribute::Destination(RouteAddress::Inet(addr)) => {
                    if !addr.is_unspecified() {
                        is_default = false;
                    }
                }
                RouteAttribute::Gateway(RouteAddress::Inet(gw)) => {
                    gateway = Some(*gw);
                }
                _ => {}
            }
        }

        // Check if this is a default route (destination 0.0.0.0/0)
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

pub async fn restore_default_gateway(handle: &Handle) -> Result<()> {
    if let Some(gateway) = find_default_gateway(handle).await? {
        let _ = add_default_route(handle, gateway).await;
    }
    Ok(())
}

// ============================================================================
// IPv6 Route Operations
// ============================================================================

pub async fn find_default_gateway_v6(handle: &Handle) -> Result<Option<Ipv6Addr>> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();

    while let Some(route) = routes.try_next().await? {
        let mut is_default = route.header.destination_prefix_length == 0;
        let mut gateway = None;

        for attr in &route.attributes {
            match attr {
                RouteAttribute::Destination(RouteAddress::Inet6(addr)) => {
                    if !addr.is_unspecified() {
                        is_default = false;
                    }
                }
                RouteAttribute::Gateway(RouteAddress::Inet6(gw)) => {
                    gateway = Some(*gw);
                }
                _ => {}
            }
        }

        // Check if this is a default route (destination ::/0)
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

pub async fn restore_default_gateway_v6(handle: &Handle) -> Result<()> {
    if let Some(gateway) = find_default_gateway_v6(handle).await? {
        let _ = add_default_route_v6(handle, gateway).await;
    }
    Ok(())
}
