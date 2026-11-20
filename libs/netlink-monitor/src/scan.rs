//! Link enumeration used to seed the monitor with initial interface state.

use std::collections::HashMap;

use netlink::link;
use netlink::link::device::{self, Device};

use crate::Result;

pub(super) async fn state(
    link: &impl link::Ops,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    for Device {
        index,
        name,
        carrier,
        ..
    } in link.devices().await?
    {
        if !device::is_ethernet(&name) {
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
    struct FailingOps;

    impl FailingOps {
        fn failure() -> link::Failure {
            link::Failure::NotFound("mock".to_owned())
        }
    }

    impl link::Ops for FailingOps {
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
            std::future::ready(Err(Self::failure()))
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

    #[tokio::test]
    async fn state_propagates_enumeration_failure() {
        // ARRANGE
        let ops = FailingOps;
        let mut link_states = HashMap::new();

        // ACT
        let result = state(&ops, &mut link_states).await;

        // ASSERT
        assert!(result.is_err());
        assert!(link_states.is_empty());
    }
}
