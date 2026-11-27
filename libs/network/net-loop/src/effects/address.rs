//! Address and default route effects with their observations.

use net_observe::journal::Journal;
use net_observe::{Event, Level};
use net_runtime::{Address as _, Route as _};

use crate::error::Result;

pub(super) async fn add<R: net_runtime::Runtime>(
    runtime: &R,
    journal: &mut Journal,
    link: netmodel::Name,
    address: core::net::IpAddr,
    prefix_len: u8,
) -> Result<()> {
    runtime
        .address()
        .add(
            &link,
            netmodel::Address {
                address,
                prefix_len,
            },
        )
        .await?;
    journal.record(
        Level::Debug,
        Event::AddressAssigned {
            name: link,
            address,
            prefix_len,
        },
    );

    Ok(())
}

pub(super) async fn remove<R: net_runtime::Runtime>(
    runtime: &R,
    journal: &mut Journal,
    link: netmodel::Name,
    address: core::net::IpAddr,
    prefix_len: u8,
) -> Result<()> {
    runtime
        .address()
        .remove(
            &link,
            netmodel::Address {
                address,
                prefix_len,
            },
        )
        .await?;
    journal.record(
        Level::Debug,
        Event::AddressRemoved {
            name: link,
            address,
            prefix_len,
        },
    );

    Ok(())
}

pub(super) async fn ensure_route<R: net_runtime::Runtime>(
    runtime: &R,
    journal: &mut Journal,
    link: netmodel::Name,
    gateway: core::net::IpAddr,
) -> Result<()> {
    runtime.route().ensure_default(&link, gateway).await?;
    journal.record(
        Level::Debug,
        Event::RouteEnsured {
            name: link,
            gateway,
        },
    );

    Ok(())
}

pub(super) async fn remove_route<R: net_runtime::Runtime>(
    runtime: &R,
    journal: &mut Journal,
    link: netmodel::Name,
    gateway: core::net::IpAddr,
) -> Result<()> {
    runtime.route().remove_default(&link, gateway).await?;
    journal.record(
        Level::Debug,
        Event::RouteRemoved {
            name: link,
            gateway,
        },
    );

    Ok(())
}
