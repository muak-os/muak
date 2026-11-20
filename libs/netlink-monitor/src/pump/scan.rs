//! Link enumeration used to seed the pump with initial interface state.

use std::collections::HashMap;

use netlink::link;
use netlink::link::device::{self, Device};
use rtnetlink::packet_route::link::LinkMessage;

use crate::Result;

pub(super) async fn initial_scan(
    link: &impl link::Ops,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    for Device {
        index,
        name,
        carrier,
        admin_up,
        ..
    } in link.devices().await?
    {
        if !device::is_ethernet(&name) {
            continue;
        }
        link_states.insert(index, carrier);
        println!(
            "Initial state: {name} (index {index}) = admin:{} carrier:{}",
            if admin_up { "up" } else { "down" },
            if carrier { "yes" } else { "no" }
        );
    }

    Ok(())
}

pub(super) fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = device::extract_name(msg)?;
    let mac = device::extract_mac(msg);

    Some((name, index, mac))
}
