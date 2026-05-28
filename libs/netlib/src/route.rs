//! IPv4 and IPv6 default route management via rtnetlink.

use core::future::Future;
use core::net::{Ipv4Addr, Ipv6Addr};

use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{Handle, RouteMessageBuilder};
use thiserror::Error;
use tokio_stream::StreamExt as _;

use crate::netlink::Rtnl;

#[derive(Debug, Error)]
pub enum Failure {
    #[error("failed to add default route: {0}")]
    AddDefaultRoute(#[source] rtnetlink::Error),
    #[error("failed to add IPv6 default route: {0}")]
    AddDefaultRouteV6(#[source] rtnetlink::Error),
    #[error("failed to remove IPv6 default route: {0}")]
    RemoveDefaultRouteV6(#[source] rtnetlink::Error),
    #[error("failed to enumerate routes: {0}")]
    List(#[source] rtnetlink::Error),
}

pub type Result<T> = core::result::Result<T, Failure>;

/// Trait covering all route-layer netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Adds or confirms the default IPv4 route via a gateway.
    fn ensure_default_route(&self, gateway: Ipv4Addr) -> impl Future<Output = Result<()>> + Send;

    /// Adds or confirms the default IPv6 route via a gateway.
    fn ensure_default_route_v6(&self, gateway: Ipv6Addr)
    -> impl Future<Output = Result<()>> + Send;

    /// Removes the default IPv6 route via a gateway.
    fn remove_default_route_v6(&self, gateway: Ipv6Addr)
    -> impl Future<Output = Result<()>> + Send;
}

impl Ops for Rtnl {
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
        .map_err(Failure::AddDefaultRoute)
}

async fn ensure_default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();
    while let Some(route) = routes.try_next().await.map_err(Failure::List)? {
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
    while let Some(route) = routes.try_next().await.map_err(Failure::List)? {
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
        .map_err(Failure::AddDefaultRouteV6)
}

async fn remove_default_route_v6(handle: &Handle, gateway: Ipv6Addr) -> Result<()> {
    let mut routes = handle
        .route()
        .get(RouteMessageBuilder::<Ipv6Addr>::new().build())
        .execute();
    while let Some(route) = routes.try_next().await.map_err(Failure::List)? {
        let is_default = route.header.destination_prefix_length == 0;
        let (_, route_gateway) = extract_ipv6_gateway(&route.attributes);
        if is_default && route_gateway == Some(gateway) {
            handle
                .route()
                .del(route)
                .execute()
                .await
                .map_err(Failure::RemoveDefaultRouteV6)?;
            return Ok(());
        }
    }
    Ok(())
}

fn extract_ipv4_gateway(attrs: &[RouteAttribute]) -> (bool, Option<Ipv4Addr>) {
    let mut has_nonzero_dest = false;
    let mut gateway = None;
    for attr in attrs {
        if let &RouteAttribute::Destination(RouteAddress::Inet(addr)) = attr
            && !addr.is_unspecified()
        {
            has_nonzero_dest = true;
        }
        if let &RouteAttribute::Gateway(RouteAddress::Inet(gateway_addr)) = attr {
            gateway = Some(gateway_addr);
        }
    }
    (has_nonzero_dest, gateway)
}

fn extract_ipv6_gateway(attrs: &[RouteAttribute]) -> (bool, Option<Ipv6Addr>) {
    let mut has_nonzero_dest = false;
    let mut gateway = None;
    for attr in attrs {
        if let &RouteAttribute::Destination(RouteAddress::Inet6(addr)) = attr
            && !addr.is_unspecified()
        {
            has_nonzero_dest = true;
        }
        if let &RouteAttribute::Gateway(RouteAddress::Inet6(gateway_addr)) = attr {
            gateway = Some(gateway_addr);
        }
    }
    (has_nonzero_dest, gateway)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_ipv4_gateway_returns_gateway_without_destination() {
        // ARRANGE
        let gateway = Ipv4Addr::new(192, 0, 2, 1);
        let attributes = [RouteAttribute::Gateway(RouteAddress::Inet(gateway))];

        // ACT
        let (has_nonzero_dest, extracted) = extract_ipv4_gateway(&attributes);

        // ASSERT
        assert!(!has_nonzero_dest);
        assert_eq!(extracted, Some(gateway));
    }

    #[test]
    fn extract_ipv4_gateway_detects_nonzero_destination() {
        // ARRANGE
        let destination = Ipv4Addr::new(198, 51, 100, 0);
        let attributes = [RouteAttribute::Destination(RouteAddress::Inet(destination))];

        // ACT
        let (has_nonzero_dest, gateway) = extract_ipv4_gateway(&attributes);

        // ASSERT
        assert!(has_nonzero_dest);
        assert!(gateway.is_none());
    }

    #[test]
    fn extract_ipv4_gateway_ignores_unspecified_destination() {
        // ARRANGE
        let attributes = [RouteAttribute::Destination(RouteAddress::Inet(
            Ipv4Addr::UNSPECIFIED,
        ))];

        // ACT
        let (has_nonzero_dest, gateway) = extract_ipv4_gateway(&attributes);

        // ASSERT
        assert!(!has_nonzero_dest);
        assert!(gateway.is_none());
    }

    #[test]
    fn extract_ipv6_gateway_returns_gateway_without_destination() {
        // ARRANGE
        let gateway = Ipv6Addr::LOCALHOST;
        let attributes = [RouteAttribute::Gateway(RouteAddress::Inet6(gateway))];

        // ACT
        let (has_nonzero_dest, extracted) = extract_ipv6_gateway(&attributes);

        // ASSERT
        assert!(!has_nonzero_dest);
        assert_eq!(extracted, Some(gateway));
    }

    #[test]
    fn extract_ipv6_gateway_detects_nonzero_destination() {
        // ARRANGE
        let destination = Ipv6Addr::new(0x2001, 0xdb8, 1, 0, 0, 0, 0, 0);
        let attributes = [RouteAttribute::Destination(RouteAddress::Inet6(
            destination,
        ))];

        // ACT
        let (has_nonzero_dest, gateway) = extract_ipv6_gateway(&attributes);

        // ASSERT
        assert!(has_nonzero_dest);
        assert!(gateway.is_none());
    }

    #[test]
    fn extract_ipv6_gateway_ignores_unspecified_destination() {
        // ARRANGE
        let attributes = [RouteAttribute::Destination(RouteAddress::Inet6(
            Ipv6Addr::UNSPECIFIED,
        ))];

        // ACT
        let (has_nonzero_dest, gateway) = extract_ipv6_gateway(&attributes);

        // ASSERT
        assert!(!has_nonzero_dest);
        assert!(gateway.is_none());
    }
}
