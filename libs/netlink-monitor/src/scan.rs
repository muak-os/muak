//! Link enumeration used to seed the monitor with initial interface state.

use std::collections::HashMap;

use netlink::link;
use netlink::link::device::{self, Device};

use crate::{Failure, Result};

pub(super) async fn state(
    link: &impl link::Ops,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    for Device {
        index,
        name,
        carrier,
        is_virtual,
        ..
    } in link.devices().await.map_err(Failure::List)?
    {
        if !device::is_ethernet(&name) || is_virtual {
            continue;
        }
        link_states.insert(index, carrier);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::future::Future;
    use core::time::Duration;

    use super::*;

    #[derive(Clone)]
    struct MockOps {
        devices: Option<Vec<Device>>,
    }

    impl MockOps {
        fn failing() -> Self {
            Self { devices: None }
        }

        fn listing(devices: Vec<Device>) -> Self {
            Self {
                devices: Some(devices),
            }
        }

        fn failure() -> link::Failure {
            link::Failure::NotFound("mock".to_owned())
        }
    }

    impl link::Ops for MockOps {
        fn bring_down(&self, _index: u32) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }

        fn bring_up(&self, _index: u32) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }

        fn delete(&self, _index: u32) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }

        fn devices(&self) -> impl Future<Output = link::Result<Vec<Device>>> {
            std::future::ready(self.devices.clone().ok_or_else(Self::failure))
        }

        fn ensure_up(&self, _name: &str) -> impl Future<Output = link::Result<u32>> {
            std::future::ready(Err(Self::failure()))
        }

        fn exists(&self, _name: &str) -> impl Future<Output = link::Result<bool>> {
            std::future::ready(Err(Self::failure()))
        }

        fn index(&self, _name: &str) -> impl Future<Output = link::Result<u32>> {
            std::future::ready(Err(Self::failure()))
        }

        fn master(&self, _index: u32) -> impl Future<Output = link::Result<Option<u32>>> {
            std::future::ready(Err(Self::failure()))
        }

        fn detach(&self, _index: u32) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }

        fn set_address(
            &self,
            _index: u32,
            _mac: [u8; 6],
        ) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }

        fn probe_carriers(
            &self,
            _interfaces: &[(u32, &str)],
            _timeout: Duration,
        ) -> impl Future<Output = link::Result<HashMap<u32, bool>>> {
            std::future::ready(Err(Self::failure()))
        }

        fn set_master(
            &self,
            _slave_index: u32,
            _master_index: u32,
        ) -> impl Future<Output = link::Result<()>> {
            std::future::ready(Err(Self::failure()))
        }
    }

    fn device(index: u32, name: &str, is_virtual: bool) -> Device {
        Device {
            index,
            name: name.to_owned(),
            mac: [0xAA; 6],
            is_virtual,
            admin_up: true,
            carrier: true,
            master: None,
        }
    }

    #[tokio::test]
    async fn state_propagates_enumeration_failure() {
        // ARRANGE
        let ops = MockOps::failing();
        let mut link_states = HashMap::new();

        // ACT
        let result = state(&ops, &mut link_states).await;

        // ASSERT
        assert!(result.is_err());
        assert!(link_states.is_empty());
    }

    #[tokio::test]
    async fn state_seeds_only_physical_ethernet_links() {
        // ARRANGE
        let ops = MockOps::listing(vec![device(1, "eth0", false), device(2, "eth1", true)]);
        let mut link_states = HashMap::new();

        // ACT
        let result = state(&ops, &mut link_states).await;

        // ASSERT
        result.expect("enumeration should succeed");
        assert_eq!(
            link_states,
            HashMap::from([(1, true)]),
            "a virtual device with an Ethernet name must not be seeded"
        );
    }
}
