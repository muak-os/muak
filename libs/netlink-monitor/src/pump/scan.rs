//! Link enumeration used to seed the pump with initial interface state.

use std::collections::HashMap;

use netlink::link::{self, is_ethernet};
use rtnetlink::Handle;
use rtnetlink::packet_route::link::{LinkFlags, LinkMessage};
use tokio_stream::StreamExt as _;

use crate::{Failure, Result};

pub(super) async fn initial_scan(
    handle: &Handle,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    let mut links = handle.link().get().execute();
    while let Some(link_msg) = links.try_next().await.map_err(Failure::List)? {
        let Some((name, index, _)) = extract_link_info(&link_msg) else {
            continue;
        };
        if !is_ethernet(&name) {
            continue;
        }
        let has_carrier = link_msg.header.flags.contains(LinkFlags::LowerUp);
        let is_admin_up = link_msg.header.flags.contains(LinkFlags::Up);
        link_states.insert(index, has_carrier);
        println!(
            "Initial state: {name} (index {index}) = admin:{} carrier:{}",
            if is_admin_up { "up" } else { "down" },
            if has_carrier { "yes" } else { "no" }
        );
    }

    Ok(())
}

pub(super) fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = link::extract_name(msg)?;
    let mac = link::extract_mac(msg);

    Some((name, index, mac))
}
