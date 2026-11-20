//! Network link operations and link state types.

mod carrier;
pub mod device;
pub(crate) mod handle;
pub mod ifname;
pub mod state;

use alloc::string::String;
use core::future::Future;
use core::time::Duration;
use std::collections::HashMap;

use thiserror::Error;
use tokio_stream::StreamExt as _;

use self::device::Device;
use crate::Rtnl;

/// Link operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Link not found.
    #[error("link '{0}' not found")]
    NotFound(String),
    /// Failed to query link.
    #[error("failed to query link: {0}")]
    Query(#[source] rtnetlink::Error),
    /// Failed to subscribe to kernel link notifications.
    #[error("failed to subscribe to link notifications: {0}")]
    Subscribe(#[source] std::io::Error),
    /// Failed to bring link up.
    #[error("failed to bring link up: {0}")]
    BringUp(#[source] rtnetlink::Error),
    /// Failed to bring link down.
    #[error("failed to bring link down: {0}")]
    BringDown(#[source] rtnetlink::Error),
    /// Failed to set link master.
    #[error("failed to set link master: {0}")]
    SetMaster(#[source] rtnetlink::Error),
    /// Failed to delete link.
    #[error("failed to delete link: {0}")]
    Delete(#[source] rtnetlink::Error),
}

/// Link operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Trait covering all link-layer netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Brings a link down by its index.
    fn bring_down(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Brings a link up by its index.
    fn bring_up(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Deletes a link by its index.
    fn delete(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Enumerates every link currently known to the kernel.
    fn devices(&self) -> impl Future<Output = Result<Vec<Device>>> + Send;

    /// Brings a named link up, returning its index.
    fn ensure_up(&self, name: &str) -> impl Future<Output = Result<u32>> + Send;

    /// Returns whether a link with the given name exists.
    fn exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Returns the kernel interface index for a named link.
    fn index(&self, name: &str) -> impl Future<Output = Result<u32>> + Send;

    /// Polls interfaces for carrier, returning a map of index to carrier-present or a subscribe failure.
    fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        timeout: Duration,
    ) -> impl Future<Output = Result<HashMap<u32, bool>>> + Send;

    /// Attaches a slave interface to a master (bridge) interface.
    fn set_master(
        &self,
        slave_index: u32,
        master_index: u32,
    ) -> impl Future<Output = Result<()>> + Send;
}

impl Ops for Rtnl {
    async fn devices(&self) -> Result<Vec<Device>> {
        let mut devices = Vec::new();
        let mut links = self.handle.link().get().execute();
        while let Some(message) = links.try_next().await.map_err(Failure::Query)? {
            devices.push(Device::from_message(&message));
        }
        Ok(devices)
    }

    async fn exists(&self, name: &str) -> Result<bool> {
        handle::exists_by_name(&self.handle, name).await
    }

    async fn index(&self, name: &str) -> Result<u32> {
        Ok(handle::find_by_name(&self.handle, name).await?.header.index)
    }

    async fn ensure_up(&self, name: &str) -> Result<u32> {
        handle::ensure_up_by_name(&self.handle, name).await
    }

    async fn bring_up(&self, index: u32) -> Result<()> {
        handle::bring_up(&self.handle, index).await
    }

    async fn bring_down(&self, index: u32) -> Result<()> {
        handle::bring_down(&self.handle, index).await
    }

    async fn set_master(&self, slave_index: u32, master_index: u32) -> Result<()> {
        handle::set_master(&self.handle, slave_index, master_index).await
    }

    async fn delete(&self, index: u32) -> Result<()> {
        handle::delete(&self.handle, index).await
    }

    async fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        timeout: Duration,
    ) -> Result<HashMap<u32, bool>> {
        carrier::probe_carriers(&self.handle, interfaces, timeout).await
    }
}
