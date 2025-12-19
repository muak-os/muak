use anyhow::{Context, Result};
use futures_util::stream::TryStreamExt;
use netlink_packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use std::net::Ipv4Addr;

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
