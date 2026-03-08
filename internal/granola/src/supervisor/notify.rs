use std::os::unix::net::UnixDatagram;
use std::path::Path;

use anyhow::{Context, Result};
use notify::Health;

use super::service::ServiceStatus;

/// Processed notification ready for the supervisor to act on.
pub enum ServiceNotification {
    Ready {
        service_name: String,
    },
    StatusUpdate {
        service_name: String,
        new_status: ServiceStatus,
    },
    Stopping {
        service_name: String,
    },
}

/// Listens for notifications from supervised services over a UNIX datagram socket.
pub struct NotifyListener {
    socket: UnixDatagram,
}

impl NotifyListener {
    pub fn new(services_dir: &Path) -> Result<Self> {
        let socket_path = services_dir.join("granola-notify.sock");
        let _ = std::fs::remove_file(&socket_path);

        let socket = UnixDatagram::bind(&socket_path).context("Failed to bind notify socket")?;
        socket
            .set_nonblocking(true)
            .context("Failed to set notify socket to non-blocking")?;

        kmsg::info!("Supervisor listening on {}", socket_path.display());

        Ok(Self { socket })
    }

    /// Drains all pending notifications from the socket.
    pub fn poll(&self) -> Vec<ServiceNotification> {
        let mut notifications = Vec::new();
        let mut buf = [0u8; 4096];

        while let Ok((len, _)) = self.socket.recv_from(&mut buf) {
            let Ok(text) = std::str::from_utf8(&buf[..len]) else {
                continue;
            };
            if let Some(notification) = parse_notification(text) {
                notifications.push(notification);
            }
        }

        notifications
    }
}

/// Parses a text notification datagram into a `ServiceNotification`.
fn parse_notification(text: &str) -> Option<ServiceNotification> {
    let mut service_name: Option<&str> = None;
    let mut ready_pid: Option<u32> = None;
    let mut status_msg: Option<&str> = None;
    let mut health: Option<Health> = None;
    let mut stopping_reason: Option<&str> = None;
    let mut is_watchdog = false;

    for line in text.lines() {
        if let Some(v) = line.strip_prefix("SERVICE_NAME=") {
            service_name = Some(v);
        } else if let Some(v) = line.strip_prefix("READY=") {
            ready_pid = v.parse().ok();
        } else if let Some(v) = line.strip_prefix("STATUS=") {
            status_msg = Some(v);
        } else if let Some(v) = line.strip_prefix("HEALTH=") {
            health = v.parse().ok();
        } else if let Some(v) = line.strip_prefix("STOPPING=") {
            stopping_reason = Some(v);
        } else if line == "WATCHDOG=1" {
            is_watchdog = true;
        }
    }

    let name = service_name?.to_owned();

    if let Some(pid) = ready_pid {
        kmsg::info!("Service {} ready (PID {})", name, pid);
        return Some(ServiceNotification::Ready { service_name: name });
    }

    if let Some(msg) = status_msg {
        let h = health.unwrap_or(Health::Healthy);
        kmsg::info!("Service {} status: {} (health: {:?})", name, msg, h);
        if h == Health::Degraded {
            return Some(ServiceNotification::StatusUpdate {
                service_name: name,
                new_status: ServiceStatus::Degraded,
            });
        }
        return None;
    }

    if let Some(reason) = stopping_reason {
        kmsg::info!("Service {} stopping: {}", name, reason);
        return Some(ServiceNotification::Stopping { service_name: name });
    }

    if is_watchdog {
        return None;
    }

    None
}
