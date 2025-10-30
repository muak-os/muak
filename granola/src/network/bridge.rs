use crate::log;
use futures::stream::TryStreamExt;
use netlink_packet_route::address::AddressAttribute;
use netlink_packet_route::route::RouteAddress;
use netlink_packet_route::route::RouteAttribute;
use rtnetlink::Handle;
use std::net::Ipv4Addr;

pub async fn setup_lan_bridge(
    handle: &Handle,
    bridge_name: &str,
    physical_iface: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!(
        "network",
        "Setting up LAN bridge mode: bridge={}, physical={}",
        bridge_name,
        physical_iface
    );

    log!(
        "network",
        "Checking if bridge {} already exists",
        bridge_name
    );
    let mut links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();

    log!("network", "Executed netlink query, checking results");
    match links.try_next().await {
        Ok(Some(_)) => {
            log!(
                "network",
                "Bridge {} already exists, skipping setup",
                bridge_name
            );
            return Ok(());
        }
        Ok(None) => {
            log!(
                "network",
                "Bridge {} does not exist, will create it",
                bridge_name
            );
        }
        Err(e) => {
            // rtnetlink returns "No such device" error when device doesn't exist
            // This is expected, treat it as "device doesn't exist"
            let err_str = e.to_string();
            if err_str.contains("No such device") {
                log!(
                    "network",
                    "Bridge {} does not exist (caught 'No such device' error), will create it",
                    bridge_name
                );
            } else {
                log!("network", "ERROR during bridge check: {}", e);
                return Err(e.into());
            }
        }
    }

    // Get the physical interface index
    log!(
        "network",
        "Looking up physical interface {}",
        physical_iface
    );
    let mut links = handle
        .link()
        .get()
        .match_name(physical_iface.to_string())
        .execute();

    log!(
        "network",
        "Executed physical interface query, checking results"
    );
    let physical_index = match links.try_next().await {
        Ok(Some(link)) => {
            log!(
                "network",
                "Found physical interface {} with index {}",
                physical_iface,
                link.header.index
            );
            link.header.index
        }
        Ok(None) => {
            log!(
                "network",
                "ERROR: Physical interface {} not found",
                physical_iface
            );
            return Err(format!("Physical interface {} not found", physical_iface).into());
        }
        Err(e) => {
            log!("network", "ERROR during physical interface lookup: {}", e);
            return Err(e.into());
        }
    };

    // Get current IP configuration from physical interface
    log!(
        "network",
        "Getting IP configuration from {}",
        physical_iface
    );
    let mut ip_config: Option<(Ipv4Addr, u8)> = None;
    let mut addresses = handle.address().get().execute();
    while let Some(addr) = addresses.try_next().await? {
        if addr.header.index == physical_index {
            for attr in addr.attributes.iter() {
                if let AddressAttribute::Address(ipaddr) = attr {
                    if let std::net::IpAddr::V4(ipv4) = ipaddr {
                        ip_config = Some((*ipv4, addr.header.prefix_len));
                        log!(
                            "network",
                            "Found IP {}/{} on {}",
                            ipv4,
                            addr.header.prefix_len,
                            physical_iface
                        );
                        break;
                    }
                }
            }
        }
    }

    // Get default gateway
    log!("network", "Getting default gateway");
    let mut gateway: Option<Ipv4Addr> = None;
    let mut routes = handle.route().get(rtnetlink::IpVersion::V4).execute();
    while let Some(route) = routes.try_next().await? {
        for attr in route.attributes.iter() {
            if let RouteAttribute::Gateway(RouteAddress::Inet(addr)) = attr {
                gateway = Some(*addr);
                log!("network", "Found default gateway: {}", addr);
                break;
            }
        }
    }

    // Create bridge
    log!("network", "Creating bridge {}", bridge_name);
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

    // Bring up bridge
    handle.link().set(bridge_index).up().execute().await?;
    log!("network", "Bridge {} is up", bridge_name);

    // If we have an IP config, transfer it from physical to bridge
    if let Some((ip, prefix_len)) = ip_config {
        log!(
            "network",
            "Transferring IP {}/{} from {} to {}",
            ip,
            prefix_len,
            physical_iface,
            bridge_name
        );

        // Remove IP from physical interface
        let mut addresses = handle.address().get().execute();
        while let Some(addr) = addresses.try_next().await? {
            if addr.header.index == physical_index {
                for attr in addr.attributes.iter() {
                    if let AddressAttribute::Address(ipaddr) = attr {
                        if let std::net::IpAddr::V4(ipv4) = ipaddr {
                            if *ipv4 == ip {
                                handle.address().del(addr).execute().await?;
                                log!(
                                    "network",
                                    "Removed IP {}/{} from {}",
                                    ip,
                                    prefix_len,
                                    physical_iface
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Add IP to bridge
        handle
            .address()
            .add(bridge_index, ip.into(), prefix_len)
            .execute()
            .await?;
        log!(
            "network",
            "Added IP {}/{} to {}",
            ip,
            prefix_len,
            bridge_name
        );

        // Re-add default gateway via bridge if we had one
        if let Some(gw) = gateway {
            log!(
                "network",
                "Setting default gateway {} via {}",
                gw,
                bridge_name
            );
            // Note: Routes should automatically update when IP moves, but we can ensure it
            handle.route().add().v4().gateway(gw).execute().await.ok();
        }
    }

    // Attach physical interface to bridge
    log!(
        "network",
        "Attaching {} to bridge {}",
        physical_iface,
        bridge_name
    );

    // Need to bring down the interface before attaching to bridge
    handle.link().set(physical_index).down().execute().await?;
    log!(
        "network",
        "Brought down {} before attaching to bridge",
        physical_iface
    );

    handle
        .link()
        .set(physical_index)
        .controller(bridge_index)
        .execute()
        .await?;

    // Bring the interface back up after attaching to bridge
    handle.link().set(physical_index).up().execute().await?;
    log!(
        "network",
        "Brought up {} after attaching to bridge",
        physical_iface
    );

    log!(
        "network",
        "Bridge mode setup complete: {} attached to {}",
        physical_iface,
        bridge_name
    );

    Ok(())
}

pub async fn teardown_lan_bridge(
    handle: &Handle,
    bridge_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Tearing down LAN bridge: {}", bridge_name);

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
