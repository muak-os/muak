//! Simulates three approved peers without networking or workload mutations.

use core::error::Error;
use core::time::Duration;
use std::time::Instant;

use coordination::inventory::{Limits, Snapshot};
use coordination::registry::{AuthenticatedPeer, Registry};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn Error>> {
    let domain = Uuid::new_v4();
    let mut registry = Registry::new(domain, Limits::new(3, 16, 255)?)?;
    for number in 1_u8..=3 {
        let node = Uuid::new_v4();
        let fingerprint = [number; 32];
        registry.approve(node, fingerprint)?;
        let peer = AuthenticatedPeer {
            domain,
            node,
            fingerprint,
        };
        let first = registry.connect(peer)?;
        registry.report(
            first,
            0,
            Snapshot {
                hostname: format!("host-{number}"),
                vms: Vec::new(),
            },
        )?;
        if number == 1 {
            registry.disconnect(first)?;
        }
    }
    for (id, node) in registry.nodes() {
        println!(
            "node={id} stale={}",
            node.is_stale(Instant::now(), Duration::from_secs(30))
        );
    }
    Ok(())
}
