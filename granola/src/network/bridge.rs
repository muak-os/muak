use crate::log;
use futures::stream::TryStreamExt;
use rtnetlink::Handle;

pub async fn create_bridge(
    handle: &Handle,
    bridge_name: &str,
    bridge_ip: std::net::Ipv4Addr,
    prefix_len: u8,
) -> Result<u32, Box<dyn std::error::Error>> {
    log!("network", "Creating bridge: {}", bridge_name);

    let mut links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();
    match links.try_next().await {
        Ok(Some(link)) => {
            log!("network", "Bridge {} already exists, reusing", bridge_name);
            return Ok(link.header.index);
        }
        Ok(None) => {}
        Err(_) => {}
    }

    handle
        .link()
        .add()
        .bridge(bridge_name.to_string())
        .execute()
        .await?;

    let mut links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();
    let bridge_index = if let Some(link) = links.try_next().await? {
        link.header.index
    } else {
        return Err("Failed to find created bridge".into());
    };

    handle.link().set(bridge_index).up().execute().await?;
    log!("network", "Bridge {} is up", bridge_name);

    handle
        .address()
        .add(bridge_index, bridge_ip.into(), prefix_len)
        .execute()
        .await?;

    log!(
        "network",
        "Assigned IP {}/{} to bridge {}",
        bridge_ip,
        prefix_len,
        bridge_name
    );

    Ok(bridge_index)
}

pub async fn delete_bridge(
    handle: &Handle,
    bridge_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Deleting bridge: {}", bridge_name);

    let mut links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();
    if let Some(link) = links.try_next().await? {
        handle.link().del(link.header.index).execute().await?;
        log!("network", "Bridge {} deleted", bridge_name);
    } else {
        log!("network", "Bridge {} does not exist", bridge_name);
    }

    Ok(())
}

pub async fn attach_to_bridge(
    handle: &Handle,
    tap_name: &str,
    bridge_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!(
        "network",
        "Attaching {} to bridge {}",
        tap_name,
        bridge_name
    );

    let mut tap_links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();
    let tap_index = if let Some(link) = tap_links.try_next().await? {
        link.header.index
    } else {
        return Err(format!("TAP device {} not found", tap_name).into());
    };

    let mut bridge_links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();
    let bridge_index = if let Some(link) = bridge_links.try_next().await? {
        link.header.index
    } else {
        return Err(format!("Bridge {} not found", bridge_name).into());
    };

    handle
        .link()
        .set(tap_index)
        .controller(bridge_index)
        .execute()
        .await?;

    log!("network", "{} attached to bridge {}", tap_name, bridge_name);

    Ok(())
}
