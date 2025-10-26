use crate::log;
use rtnetlink::{new_connection, Handle};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

use super::bridge;
use super::config::*;
use super::dhcp_server::DhcpServer;
use super::host;
use super::ip_allocator::IpAllocator;
use super::nat;

pub struct NetworkManager {
    handle: Handle,
    allocator: Arc<IpAllocator>,
    dhcp_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    wan_interface: Arc<Mutex<Option<String>>>,
}

impl NetworkManager {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        log!("network", "Initializing network manager");

        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        let allocator = Arc::new(IpAllocator::new(DHCP_POOL_START, DHCP_POOL_END));

        Ok(Self {
            handle,
            allocator,
            dhcp_task: Arc::new(Mutex::new(None)),
            wan_interface: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn initialize_host(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Initializing host networking");

        host::setup_loopback(&self.handle).await?;

        // Find and bring up WAN interface
        if let Ok(iface) = host::find_ethernet_interface(&self.handle).await {
            let mut links = self.handle.link().get().match_name(iface.clone()).execute();
            use futures::stream::TryStreamExt;
            if let Some(link) = links.try_next().await? {
                self.handle.link().set(link.header.index).up().execute().await?;
            }
            *self.wan_interface.lock().unwrap() = Some(iface);
        }

        Ok(())
    }

    pub async fn setup_bridge(&self) -> Result<(), Box<dyn std::error::Error>> {
        bridge::create_bridge(&self.handle, BRIDGE_NAME, BRIDGE_IP, BRIDGE_PREFIX_LEN).await?;

        nat::enable_ip_forwarding().await?;

        // Setup NAT if we have a WAN interface
        if let Some(ref wan_iface) = *self.wan_interface.lock().unwrap() {
            let subnet = format!("{}/{}", BRIDGE_IP, BRIDGE_PREFIX_LEN);
            nat::setup_nat(wan_iface, BRIDGE_NAME, &subnet).await?;
        }

        Ok(())
    }

    pub async fn start_dhcp_server(&self) -> Result<(), Box<dyn std::error::Error>> {
        let server = DhcpServer::new(
            BRIDGE_IP,
            subnet_mask(),
            BRIDGE_IP,
            DHCP_LEASE_TIME,
            self.allocator.clone(),
        );

        let bridge_name = BRIDGE_NAME.to_string();
        let dhcp_task = tokio::spawn(async move {
            if let Err(e) = server.run(&bridge_name).await {
                log!("dhcp", "DHCP server error: {}", e);
            }
        });

        *self.dhcp_task.lock().unwrap() = Some(dhcp_task);

        Ok(())
    }

    pub fn get_handle(&self) -> Handle {
        self.handle.clone()
    }

    pub fn get_allocator(&self) -> Arc<IpAllocator> {
        self.allocator.clone()
    }

    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Shutting down network manager");

        if let Some(task) = self.dhcp_task.lock().unwrap().take() {
            task.abort();
        }

        nat::teardown_nat().await?;

        bridge::delete_bridge(&self.handle, BRIDGE_NAME).await?;

        log!("network", "Network manager shutdown complete");
        Ok(())
    }
}
