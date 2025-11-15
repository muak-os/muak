use crate::log;
use anyhow::Result;
use rtnetlink::{Handle, new_connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use super::bridge;
use super::config::*;
use super::dhcp;
use super::interface::{self, Interface};
use super::selection::InterfaceSelector;
use super::state::NetworkState;

pub struct NetworkManager {
    handle: Handle,
    state: Arc<RwLock<NetworkState>>,
    interfaces: Arc<RwLock<HashMap<String, Interface>>>,
    primary_interface: Arc<Mutex<Option<String>>>,
    backup_interfaces: Arc<Mutex<Vec<String>>>,
}

impl NetworkManager {
    pub async fn new() -> Result<Self> {
        log!("network", "Initializing network manager");

        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        Ok(Self {
            handle,
            state: Arc::new(RwLock::new(NetworkState::Uninitialized)),
            interfaces: Arc::new(RwLock::new(HashMap::new())),
            primary_interface: Arc::new(Mutex::new(None)),
            backup_interfaces: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn get_state(&self) -> NetworkState {
        self.state.read().unwrap().clone()
    }

    fn set_state(&self, new_state: NetworkState) {
        let old_state = self.state.read().unwrap().clone();
        *self.state.write().unwrap() = new_state.clone();

        if old_state != new_state {
            log!(
                "network",
                "Network state transition: {} -> {}",
                old_state,
                new_state
            );
        }
    }

    pub async fn initialize_host(&self) {
        match interface::setup_loopback(&self.handle).await {
            Ok(()) => {
                log!("network", "Loopback interface configured");
            }
            Err(e) => {
                log!(
                    "network",
                    "WARNING: Failed to setup loopback interface: {}",
                    e
                );
            }
        }

        let mut attempt = 0u32;
        let base_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(10);

        loop {
            attempt += 1;
            self.set_state(NetworkState::Initializing);

            log!("network", "Network initialization attempt {}", attempt);

            match self.try_initialize_host().await {
                Ok(()) => {
                    log!(
                        "network",
                        "Network initialized successfully on attempt {}",
                        attempt
                    );
                    self.set_state(NetworkState::Ready);
                    return;
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    self.set_state(NetworkState::Degraded);

                    log!(
                        "network",
                        "Network initialization failed (attempt {}): {}",
                        attempt,
                        error_msg
                    );

                    let delay =
                        std::cmp::min(base_delay * 2u32.pow(attempt.saturating_sub(1)), max_delay);

                    log!("network", "Retrying in {:?}...", delay);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn try_initialize_host(&self) -> Result<()> {
        let discovered = interface::discover_ethernet_interfaces(&self.handle)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if discovered.is_empty() {
            return Err(anyhow::anyhow!("No ethernet interfaces found"));
        }

        let primary = InterfaceSelector::select_primary(&discovered)
            .ok_or_else(|| anyhow::anyhow!("Failed to select primary interface"))?;

        log!(
            "network",
            "Selected primary interface: {} (state: {})",
            primary.name,
            primary.link_state
        );

        let backups = InterfaceSelector::select_backups(&discovered, &primary.name);
        log!("network", "Found {} backup interface(s)", backups.len());

        *self.primary_interface.lock().unwrap() = Some(primary.name.clone());
        *self.backup_interfaces.lock().unwrap() = backups.iter().map(|i| i.name.clone()).collect();

        let link_index = interface::bring_up_interface(&primary.name, &self.handle)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        match dhcp::run_dhcp_client(&primary.name, &self.handle, link_index).await {
            Ok(()) => {
                log!(
                    "network",
                    "DHCP configuration successful on {}",
                    primary.name
                );

                // Update interface state
                let mut interfaces = self.interfaces.write().unwrap();
                let iface_state = Interface::new(
                    primary.name.clone(),
                    primary.index,
                    primary.mac_address,
                    primary.link_state.clone(),
                );
                // TODO: Extract IP config from DHCP result and store in iface_state
                interfaces.insert(primary.name.clone(), iface_state);
            }
            Err(e) => {
                log!(
                    "network",
                    "DHCP failed on {}: {} (continuing without WAN)",
                    primary.name,
                    e
                );
                return Err(anyhow::anyhow!("DHCP failed: {}", e));
            }
        }

        Ok(())
    }

    pub fn get_handle(&self) -> Handle {
        self.handle.clone()
    }

    pub async fn initialize_bridge(&self) -> Result<()> {
        log!("network", "Initializing network bridge");

        let primary_iface = {
            let primary_guard = self.primary_interface.lock().unwrap();
            primary_guard.clone()
        };

        let primary_iface = if let Some(iface) = primary_iface {
            iface
        } else {
            return Err(anyhow::anyhow!(
                "No primary interface available for bridge setup"
            ));
        };

        bridge::setup_lan_bridge(&self.handle, LAN_BRIDGE_NAME, &primary_iface)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        log!("network", "Network bridge initialization complete");

        Ok(())
    }
}
