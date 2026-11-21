//! Address operations executed against a shared netlink connection handle.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use rtnetlink::Handle;
use rtnetlink::packet_route::address::AddressAttribute;
use tokio_stream::StreamExt as _;

use super::{Failure, Result};
use crate::retry;

/// Finds the first non-link-local IPv4 address and prefix length on a given interface.
pub(crate) async fn find_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    retry::while_retriable(
        || scan_ipv4(handle, index),
        is_dump_busy,
        retry::DUMP_ATTEMPTS,
        retry::DUMP_RETRY_DELAY_MS,
    )
    .await
}

/// Returns true if the interface has at least one IPv4 address assigned.
pub(crate) async fn has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    retry::while_retriable(
        || scan_has_ipv4(handle, index),
        is_dump_busy,
        retry::DUMP_ATTEMPTS,
        retry::DUMP_RETRY_DELAY_MS,
    )
    .await
}

async fn scan_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v4) = find_v4_in_attributes(&addr.attributes) {
            return Ok(Some((v4, addr.header.prefix_len)));
        }
    }

    Ok(None)
}

async fn scan_has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let has_v4 = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V4(_))));
        if has_v4 {
            return Ok(true);
        }
    }

    Ok(false)
}

fn is_dump_busy(failure: &Failure) -> bool {
    matches!(failure, Failure::List(inner) if retry::is_busy(inner))
}

/// Adds an IPv4 address with prefix length to the given interface.
pub(crate) async fn add_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv4)
}

/// Removes a specific IPv4 address from the given interface (no-op if absent).
pub(crate) async fn remove_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    retry::while_retriable(
        || scan_remove_ipv4(handle, index, ip),
        is_dump_busy,
        retry::DUMP_ATTEMPTS,
        retry::DUMP_RETRY_DELAY_MS,
    )
    .await
}

async fn scan_remove_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let matches = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V4(v4)) if *v4 == ip));
        if matches {
            handle
                .address()
                .del(addr)
                .execute()
                .await
                .map_err(Failure::RemoveIpv4)?;
            return Ok(());
        }
    }

    Ok(())
}

/// Adds an IPv4 address only if it is not already present with the same prefix.
pub(crate) async fn ensure_ipv4(
    handle: &Handle,
    index: u32,
    ip: Ipv4Addr,
    prefix: u8,
) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv4(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv4)
}

/// Adds an IPv6 address only if it is not already present with the same prefix.
pub(crate) async fn ensure_ipv6(
    handle: &Handle,
    index: u32,
    ip: Ipv6Addr,
    prefix: u8,
) -> Result<()> {
    retry::while_retriable(
        || scan_ensure_ipv6(handle, index, ip, prefix),
        is_dump_busy,
        retry::DUMP_ATTEMPTS,
        retry::DUMP_RETRY_DELAY_MS,
    )
    .await
}

async fn scan_ensure_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    let mut addrs = handle.address().get().execute();
    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v6) = find_v6_in_attributes(&addr.attributes)
            && v6.segments()[0] != 0xfe80
            && v6 == ip
            && addr.header.prefix_len == prefix
        {
            return Ok(());
        }
    }
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv6)
}

/// Removes a specific IPv6 address from the given interface (no-op if absent).
pub(crate) async fn remove_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr) -> Result<()> {
    retry::while_retriable(
        || scan_remove_ipv6(handle, index, ip),
        is_dump_busy,
        retry::DUMP_ATTEMPTS,
        retry::DUMP_RETRY_DELAY_MS,
    )
    .await
}

async fn scan_remove_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();
    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let matches = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V6(v6)) if *v6 == ip));
        if matches {
            handle
                .address()
                .del(addr)
                .execute()
                .await
                .map_err(Failure::RemoveIpv6)?;
            return Ok(());
        }
    }
    Ok(())
}

fn find_v4_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv4Addr> {
    attributes.iter().find_map(|attr| {
        if let &AddressAttribute::Address(IpAddr::V4(v4)) = attr {
            Some(v4)
        } else {
            None
        }
    })
}

fn find_v6_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv6Addr> {
    attributes.iter().find_map(|attr| {
        if let &AddressAttribute::Address(IpAddr::V6(v6)) = attr {
            Some(v6)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use core::num::NonZeroI32;

    use rtnetlink::packet_core::ErrorMessage;
    use rtnetlink::packet_route::address::AddressAttribute;

    use super::{find_v4_in_attributes, find_v6_in_attributes, is_dump_busy};
    use crate::address::Failure;

    #[test]
    fn is_dump_busy_matches_busy_error() {
        // ARRANGE
        let mut message = ErrorMessage::default();
        message.code = NonZeroI32::new(-16_i32);
        let failure = Failure::List(rtnetlink::Error::NetlinkError(message));

        // ACT
        let busy = is_dump_busy(&failure);

        // ASSERT
        assert!(busy);
    }

    #[test]
    fn is_dump_busy_rejects_other_errors() {
        // ARRANGE
        let mut message = ErrorMessage::default();
        message.code = NonZeroI32::new(-95_i32);
        let failure = Failure::List(rtnetlink::Error::NetlinkError(message));

        // ACT
        let busy = is_dump_busy(&failure);

        // ASSERT
        assert!(!busy);
    }

    #[test]
    fn find_v4_in_attributes_returns_first_ipv4_address() {
        // ARRANGE
        let attributes = vec![
            AddressAttribute::Address(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11))),
        ];

        // ACT
        let address = find_v4_in_attributes(&attributes);

        // ASSERT
        assert_eq!(address, Some(Ipv4Addr::new(192, 0, 2, 10)));
    }

    #[test]
    fn find_v6_in_attributes_returns_first_ipv6_address() {
        // ARRANGE
        let expected = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let attributes = vec![
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
            AddressAttribute::Address(IpAddr::V6(expected)),
        ];

        // ACT
        let address = find_v6_in_attributes(&attributes);

        // ASSERT
        assert_eq!(address, Some(expected));
    }

    #[test]
    fn find_v4_in_attributes_returns_none_without_ipv4() {
        // ARRANGE
        let attributes = vec![AddressAttribute::Address(IpAddr::V6(Ipv6Addr::LOCALHOST))];

        // ACT
        let address = find_v4_in_attributes(&attributes);

        // ASSERT
        assert!(address.is_none());
    }

    #[test]
    fn find_v6_in_attributes_returns_none_without_ipv6() {
        // ARRANGE
        let attributes = vec![AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(
            192, 0, 2, 10,
        )))];

        // ACT
        let address = find_v6_in_attributes(&attributes);

        // ASSERT
        assert!(address.is_none());
    }
}
