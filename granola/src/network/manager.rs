use crate::log;
use rtnetlink::{new_connection, Handle};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

use super::bridge;
use super::bridge_mode;
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
    bridge_mode_initialized: Arc<Mutex<bool>>,
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
            bridge_mode_initialized: Arc::new(Mutex::new(false)),
        })
    }

    pub async fn initialize_host(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Initializing host networking");

        host::setup_loopback(&self.handle).await?;

        if let Ok(iface) = host::find_ethernet_interface(&self.handle).await {
            match host::run_dhcp_client(&iface, &self.handle).await {
                Ok(()) => {
                    *self.wan_interface.lock().unwrap() = Some(iface);
                }
                Err(e) => {
                    log!(
                        "network",
                        "DHCP failed on {}: {} (continuing without WAN)",
                        iface,
                        e
                    );
                    *self.wan_interface.lock().unwrap() = Some(iface);
                }
            }
        }

        Ok(())
    }

    pub async fn setup_bridge(&self) -> Result<(), Box<dyn std::error::Error>> {
        bridge::create_bridge(&self.handle, BRIDGE_NAME, BRIDGE_IP, BRIDGE_PREFIX_LEN).await?;

        nat::enable_ip_forwarding().await?;

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

    pub async fn ensure_bridge_mode(&self) -> Result<(), Box<dyn std::error::Error>> {
        {
            let initialized = self.bridge_mode_initialized.lock().unwrap();
            if *initialized {
                log!("network", "Bridge mode already initialized");
                return Ok(());
            }
        }

        log!("network", "Initializing bridge mode for the first time");

        let wan_iface = {
            let wan_guard = self.wan_interface.lock().unwrap();
            wan_guard.clone()
        };

        let wan_iface = if let Some(iface) = wan_iface {
            iface
        } else {
            match host::find_ethernet_interface(&self.handle).await {
                Ok(iface) => {
                    *self.wan_interface.lock().unwrap() = Some(iface.clone());
                    iface
                }
                Err(e) => {
                    return Err(format!("Failed to find physical interface: {}", e).into());
                }
            }
        };

        bridge_mode::setup_lan_bridge(&self.handle, LAN_BRIDGE_NAME, &wan_iface).await?;

        *self.bridge_mode_initialized.lock().unwrap() = true;
        log!("network", "Bridge mode initialization complete");

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Shutting down network manager");

        if let Some(task) = self.dhcp_task.lock().unwrap().take() {
            task.abort();
        }

        nat::teardown_nat().await?;

        bridge::delete_bridge(&self.handle, BRIDGE_NAME).await?;

        // Clean up bridge mode if it was initialized
        if *self.bridge_mode_initialized.lock().unwrap() {
            bridge_mode::teardown_lan_bridge(&self.handle, LAN_BRIDGE_NAME).await?;
        }

        log!("network", "Network manager shutdown complete");
        Ok(())
    }
}
