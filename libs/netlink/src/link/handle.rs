//! Link operations executed against a shared netlink connection handle.

use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::{Handle, LinkUnspec};
use tokio_stream::StreamExt as _;

use super::{Failure, Result};

/// Finds a link by name, returning its full message or an error if not found.
pub(crate) async fn find_by_name(handle: &Handle, name: &str) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();

    links
        .try_next()
        .await
        .map_err(Failure::Query)?
        .ok_or_else(|| Failure::NotFound(name.to_owned()))
}

/// Returns the kernel interface index for the named link.
pub(crate) async fn get_index(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;

    Ok(link.header.index)
}

/// Checks whether a link with the given name exists.
pub(crate) async fn exists(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();

    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(rtnetlink::Error::NetlinkError(ref msg))
            if msg
                .code
                .is_some_and(|code| matches!(code.get(), -19_i32 | -2_i32)) =>
        {
            Ok(false)
        }
        Err(error) => Err(Failure::Query(error)),
    }
}

/// Sets the `IFF_UP` flag on the link identified by index.
pub(crate) async fn bring_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .map_err(Failure::BringUp)
}

/// Clears the `IFF_UP` flag on the link identified by index.
pub(crate) async fn bring_down(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).down().build())
        .execute()
        .await
        .map_err(Failure::BringDown)
}

/// Sets the master (bridge/bond) for a slave link.
pub(crate) async fn set_master(handle: &Handle, slave_index: u32, master_index: u32) -> Result<()> {
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(slave_index)
                .controller(master_index)
                .build(),
        )
        .execute()
        .await
        .map_err(Failure::SetMaster)
}

/// Deletes a network link by its index.
pub(crate) async fn delete(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .del(index)
        .execute()
        .await
        .map_err(Failure::Delete)
}

/// Returns the master (bridge/bond) interface index for a link, if any.
pub(crate) async fn master_index(handle: &Handle, index: u32) -> Result<Option<u32>> {
    let mut links = handle.link().get().match_index(index).execute();
    let link = links
        .try_next()
        .await
        .map_err(Failure::Query)?
        .ok_or_else(|| Failure::NotFound(format!("index {index}")))?;
    for attr in &link.attributes {
        if let &LinkAttribute::Controller(master) = attr {
            return Ok(Some(master));
        }
    }

    Ok(None)
}

/// Checks whether a link with the given name exists, mapping a missing link to `false`.
pub(crate) async fn exists_by_name(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();
    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(rtnetlink::Error::NetlinkError(ref msg))
            if msg
                .code
                .is_some_and(|code| matches!(code.get(), -19_i32 | -2_i32)) =>
        {
            Ok(false)
        }
        Err(error) => Err(Failure::Query(error)),
    }
}

/// Brings a named link up when it is administratively down, returning its index.
pub(crate) async fn ensure_up_by_name(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;
    let index = link.header.index;
    if !link.header.flags.contains(LinkFlags::Up) {
        println!("Bringing up interface {name} (index {index})");
        bring_up(handle, index).await?;
    }

    Ok(index)
}
