use crate::log;
use rtnetlink::{new_connection, Handle};
use std::sync::{Arc, Mutex};

use super::bridge;
use super::config::*;
use super::host;

pub struct NetworkManager {
    handle: Handle,
    wan_interface: Arc<Mutex<Option<String>>>,
    bridge_mode_initialized: Arc<Mutex<bool>>,
}

impl NetworkManager {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        log!("network", "Initializing network manager");

        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        Ok(Self {
            handle,
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

    pub fn get_handle(&self) -> Handle {
        self.handle.clone()
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

        bridge::setup_lan_bridge(&self.handle, LAN_BRIDGE_NAME, &wan_iface).await?;

        *self.bridge_mode_initialized.lock().unwrap() = true;
        log!("network", "Bridge mode initialization complete");

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Shutting down network manager");

        // Clean up bridge mode if it was initialized
        if *self.bridge_mode_initialized.lock().unwrap() {
            bridge::teardown_lan_bridge(&self.handle, LAN_BRIDGE_NAME).await?;
        }

        log!("network", "Network manager shutdown complete");
        Ok(())
    }
}
