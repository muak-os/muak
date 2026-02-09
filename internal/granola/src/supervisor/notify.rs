use std::os::unix::net::UnixDatagram;

use anyhow::{Context, Result};
use prost::Message;

use super::service::ServiceStatus;

#[allow(clippy::excessive_nesting)]
mod proto {
    include!(concat!(env!("OUT_DIR"), "/muak.internal.supervisor.rs"));
}

pub use proto::{Health, Notify, notify::Notification};

const NOTIFY_SOCKET: &str = "/run/granola-notify.sock";

/// Processed notification ready for the supervisor to act on.
pub enum ServiceNotification {
    Ready {
        service_name: String,
        socket_path: String,
    },
    StatusUpdate {
        service_name: String,
        new_status: ServiceStatus,
    },
    Stopping {
        service_name: String,
    },
}

/// Listens for protobuf notifications from supervised services over a UNIX datagram socket.
pub struct NotifyListener {
    socket: UnixDatagram,
}

impl NotifyListener {
    pub fn new() -> Result<Self> {
        let _ = std::fs::remove_file(NOTIFY_SOCKET);

        let socket = UnixDatagram::bind(NOTIFY_SOCKET).context("Failed to bind notify socket")?;
        socket
            .set_nonblocking(true)
            .context("Failed to set notify socket to non-blocking")?;

        kmsg::info!("Supervisor listening on {}", NOTIFY_SOCKET);

        Ok(Self { socket })
    }

    /// Drains all pending notifications from the socket.
    pub fn poll(&self) -> Vec<ServiceNotification> {
        let mut notifications = Vec::new();
        let mut buf = [0u8; 4096];

        while let Ok((len, _)) = self.socket.recv_from(&mut buf) {
            let Ok(notify) = Notify::decode(&buf[..len]) else {
                continue;
            };

            if let Some(notification) = self.decode_notification(notify) {
                notifications.push(notification);
            }
        }

        notifications
    }

    /// Converts a raw `Notify` protobuf message into a higher-level `ServiceNotification`.
    fn decode_notification(&self, notify: Notify) -> Option<ServiceNotification> {
        let notification = notify.notification?;

        match notification {
            Notification::Ready(ready) => {
                kmsg::info!(
                    "Service {} ready (PID {}, socket: {})",
                    notify.service_name,
                    ready.pid,
                    ready.socket_path
                );
                Some(ServiceNotification::Ready {
                    service_name: notify.service_name,
                    socket_path: ready.socket_path,
                })
            }
            Notification::Status(status) => {
                let health = Health::try_from(status.health).unwrap_or(Health::Healthy);
                kmsg::info!(
                    "Service {} status: {} (health: {:?})",
                    notify.service_name,
                    status.message,
                    health
                );
                let new_status = if health == Health::Degraded {
                    ServiceStatus::Degraded
                } else {
                    return None;
                };
                Some(ServiceNotification::StatusUpdate {
                    service_name: notify.service_name,
                    new_status,
                })
            }
            Notification::Stopping(stopping) => {
                kmsg::info!(
                    "Service {} stopping: {}",
                    notify.service_name,
                    stopping.reason
                );
                Some(ServiceNotification::Stopping {
                    service_name: notify.service_name,
                })
            }
            Notification::Watchdog(_) => None,
        }
    }
}
