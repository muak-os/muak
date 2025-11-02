use crate::log;
use rtnetlink::{new_connection, Handle};
use std::sync::{Arc, Mutex};

use super::bridge;
use super::config::*;
use super::dhcp;
use super::interface;

pub struct NetworkManager {
    handle: Handle,
    wan_interface: Arc<Mutex<Option<String>>>,
}

impl NetworkManager {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        log!("network", "Initializing network manager");

        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        Ok(Self {
            handle,
            wan_interface: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn initialize_host(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Initializing host networking");

        interface::setup_loopback(&self.handle).await?;

        if let Ok(iface) = interface::find_ethernet_interface(&self.handle).await {
            match interface::bring_up_interface(&iface, &self.handle).await {
                Ok(link_index) => {
                    match dhcp::run_dhcp_client(&iface, &self.handle, link_index).await {
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
                Err(e) => {
                    log!("network", "Failed to bring up interface {}: {}", iface, e);
                }
            }
        }

        Ok(())
    }

    pub fn get_handle(&self) -> Handle {
        self.handle.clone()
    }

    pub async fn initialize_bridge(&self) -> Result<(), Box<dyn std::error::Error>> {
        log!("network", "Initializing network bridge");

        let wan_iface = {
            let wan_guard = self.wan_interface.lock().unwrap();
            wan_guard.clone()
        };

        let wan_iface = if let Some(iface) = wan_iface {
            iface
        } else {
            match interface::find_ethernet_interface(&self.handle).await {
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

        log!("network", "Network bridge initialization complete");

        Ok(())
    }
}
