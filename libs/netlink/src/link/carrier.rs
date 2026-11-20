//! Carrier detection for links being brought up.

use core::time::Duration;
use std::collections::HashMap;

use rtnetlink::packet_core::NetlinkPayload;
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::link::LinkFlags;
use rtnetlink::{Handle, MulticastGroup};
use tokio::time::{Instant, timeout as timeout_after};
use tokio_stream::StreamExt as _;

use super::handle::bring_up;
use super::{Failure, Result};

pub(super) async fn probe_carriers(
    handle: &Handle,
    interfaces: &[(u32, &str)],
    timeout: Duration,
) -> Result<HashMap<u32, bool>> {
    let indices: Vec<u32> = interfaces.iter().map(|&(index, _)| index).collect();

    let (conn, sub_handle, mut messages) =
        rtnetlink::new_multicast_connection(&[MulticastGroup::Link]).map_err(Failure::Subscribe)?;
    tokio::spawn(conn);

    for &index in &indices {
        let _result = bring_up(handle, index).await;
    }

    let mut states: HashMap<u32, bool> = indices.iter().map(|&idx| (idx, false)).collect();

    if let Ok(initial) = get_all_carrier_states(&sub_handle).await {
        seed_carrier_states(&mut states, &indices, &initial);
    }

    if states.values().any(|&carrier_present| carrier_present) {
        return Ok(states);
    }

    let Some(deadline) = Instant::now().checked_add(timeout) else {
        return Ok(states);
    };

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        let next = timeout_after(remaining, messages.next());
        let Ok(Some((message, _))) = next.await else {
            break;
        };

        let NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link_msg)) = message.payload
        else {
            continue;
        };

        let idx = link_msg.header.index;
        if !states.contains_key(&idx) || !link_msg.header.flags.contains(LinkFlags::LowerUp) {
            continue;
        }

        states.insert(idx, true);
        if states.values().all(|&carrier_present| carrier_present) {
            return Ok(states);
        }
    }

    Ok(states)
}

async fn get_all_carrier_states(handle: &Handle) -> Result<HashMap<u32, bool>> {
    let mut states = HashMap::new();
    let mut links = handle.link().get().execute();
    while let Some(link) = links.try_next().await.map_err(Failure::Query)? {
        states.insert(
            link.header.index,
            link.header.flags.contains(LinkFlags::LowerUp),
        );
    }
    Ok(states)
}

fn seed_carrier_states(
    states: &mut HashMap<u32, bool>,
    indices: &[u32],
    initial: &HashMap<u32, bool>,
) {
    for &idx in indices {
        if initial.get(&idx) == Some(&true) {
            states.insert(idx, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::seed_carrier_states;

    #[test]
    fn seed_carrier_states_marks_only_requested_carriers() {
        // ARRANGE
        let mut states = HashMap::new();
        let initial = HashMap::from([(1, true), (2, false), (3, true)]);

        // ACT
        seed_carrier_states(&mut states, &[1, 2], &initial);

        // ASSERT
        assert_eq!(states, HashMap::from([(1, true)]));
    }
}
