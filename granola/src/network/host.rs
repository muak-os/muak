use crate::log;
use futures::stream::TryStreamExt;
use netlink_packet_route::link::LinkAttribute;
use rtnetlink::Handle;

pub async fn setup_loopback(handle: &Handle) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Setting up loopback interface");

    let mut links = handle.link().get().match_name("lo".to_string()).execute();
    if let Some(link) = links.try_next().await? {
        handle.link().set(link.header.index).up().execute().await?;
        log!("network", "Loopback interface is up");
    }

    Ok(())
}

pub async fn find_ethernet_interface(
    handle: &Handle,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        for attr in &link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                if name.starts_with("eth") || name.starts_with("enp") {
                    log!("network", "Found ethernet interface: {}", name);
                    return Ok(name.clone());
                }
            }
        }
    }

    Err("No ethernet interface found".into())
}

pub async fn bring_up_interface(
    interface: &str,
    handle: &Handle,
) -> Result<u32, Box<dyn std::error::Error>> {
    log!("network", "Bringing up interface {}", interface);

    let mut links = handle
        .link()
        .get()
        .match_name(interface.to_string())
        .execute();

    let link_index = if let Some(link) = links.try_next().await? {
        let index = link.header.index;
        handle.link().set(index).up().execute().await?;
        log!("network", "Interface {} is up", interface);
        index
    } else {
        return Err("Interface not found".into());
    };

    Ok(link_index)
}
