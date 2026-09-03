use std::os::unix::net::UnixDatagram;
use std::path::Path;

use anyhow::{Context as _, Result};
use granola::runtime::notify::Health;

use super::Supervisor;
use super::reaper::Reap;
use super::service::ServiceStatus;
use super::spawner::Spawn;

/// Maximum size of a notification datagram.
const MAX_DATAGRAM_SIZE: usize = 4096;

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
        drop(std::fs::remove_file(&socket_path));

        let socket = UnixDatagram::bind(&socket_path).context("Failed to bind notify socket")?;
        socket
            .set_nonblocking(true)
            .context("Failed to set notify socket to non-blocking")?;

        kmsg::info!("Supervisor listening on {}", socket_path.display());

        Ok(Self { socket })
    }

    /// Drains all pending notifications from the socket.
    pub fn poll(&self) -> Vec<ServiceNotification> {
        let mut buf = [0_u8; MAX_DATAGRAM_SIZE];
        let mut notifications = Vec::new();

        while let Ok((len, _)) = self.socket.recv_from(&mut buf)
            && let Some(text) = buf
                .get(..len)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
            && let Some(notification) = parse_notification(text)
        {
            notifications.push(notification);
        }

        notifications
    }
}

/// Applies a single service notification to the corresponding service state.
pub(super) fn apply<S: Spawn, R: Reap>(
    supervisor: &mut Supervisor<S, R>,
    notification: ServiceNotification,
) {
    match notification {
        ServiceNotification::Ready { service_name } => {
            apply_ready(supervisor, &service_name);
        }
        ServiceNotification::StatusUpdate {
            service_name,
            new_status,
        } => {
            apply_status_update(supervisor, &service_name, new_status);
        }
        ServiceNotification::Stopping { service_name } => {
            apply_stopping(supervisor, &service_name);
        }
    }
}

/// Marks a service as ready, resetting its restart counter.
fn apply_ready<S: Spawn, R: Reap>(supervisor: &mut Supervisor<S, R>, service_name: &str) {
    let Some(state) = supervisor.services.get_mut(service_name) else {
        kmsg::warn!("Notification from unknown service: {service_name}");
        return;
    };
    state.status = ServiceStatus::Ready;
    state.restart_count = 0;
}

/// Records a status update for a service.
fn apply_status_update<S: Spawn, R: Reap>(
    supervisor: &mut Supervisor<S, R>,
    service_name: &str,
    new_status: ServiceStatus,
) {
    if let Some(state) = supervisor.services.get_mut(service_name) {
        state.status = new_status;
    }
}

/// Marks a service as stopping.
fn apply_stopping<S: Spawn, R: Reap>(supervisor: &mut Supervisor<S, R>, service_name: &str) {
    if let Some(state) = supervisor.services.get_mut(service_name) {
        state.status = ServiceStatus::Stopping;
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
        if let Some(name) = line.strip_prefix("SERVICE_NAME=") {
            service_name = Some(name);
        } else if let Some(pid_text) = line.strip_prefix("READY=") {
            ready_pid = pid_text.parse().ok();
        } else if let Some(status_text) = line.strip_prefix("STATUS=") {
            status_msg = Some(status_text);
        } else if let Some(health_text) = line.strip_prefix("HEALTH=") {
            health = health_text.parse().ok();
        } else if let Some(reason) = line.strip_prefix("STOPPING=") {
            stopping_reason = Some(reason);
        } else if line == "WATCHDOG=1" {
            is_watchdog = true;
        } else {
            // Unknown fields are ignored for forward compatibility.
        }
    }

    let name = service_name?.to_owned();

    if let Some(pid) = ready_pid {
        kmsg::info!("Service {} ready (PID {})", name, pid);
        return Some(ServiceNotification::Ready { service_name: name });
    }

    if let Some(msg) = status_msg {
        let health = health.unwrap_or(Health::Healthy);
        kmsg::info!("Service {} status: {} (health: {:?})", name, msg, health);
        if health == Health::Degraded {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_notification_parsed() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nREADY=42";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        let Some(ServiceNotification::Ready { service_name }) = result else {
            panic!("expected Ready notification");
        };
        assert_eq!(service_name, "myservice");
    }

    #[test]
    fn ready_with_invalid_pid_falls_through() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nREADY=not-a-number";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn status_update_degraded_health() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nSTATUS=Things are bad\nHEALTH=degraded";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        let Some(ServiceNotification::StatusUpdate {
            service_name,
            new_status,
        }) = result
        else {
            panic!("expected StatusUpdate notification");
        };
        assert_eq!(service_name, "myservice");
        assert_eq!(new_status, ServiceStatus::Degraded);
    }

    #[test]
    fn status_update_healthy_returns_none() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nSTATUS=All good\nHEALTH=healthy";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn status_update_unhealthy_returns_none() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nSTATUS=Very bad\nHEALTH=unhealthy";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn status_without_health_defaults_to_healthy_returns_none() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nSTATUS=Some status";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn stopping_notification_parsed() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nSTOPPING=graceful shutdown";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        let Some(ServiceNotification::Stopping { service_name }) = result else {
            panic!("expected Stopping notification");
        };
        assert_eq!(service_name, "myservice");
    }

    #[test]
    fn watchdog_only_returns_none() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nWATCHDOG=1";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn missing_service_name_returns_none() {
        // ARRANGE
        let text = "READY=42";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn empty_text_returns_none() {
        // ARRANGE
        let text = "";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn ready_takes_priority_over_status() {
        // ARRANGE
        let text = "SERVICE_NAME=myservice\nREADY=1\nSTATUS=Also present\nHEALTH=degraded";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(matches!(result, Some(ServiceNotification::Ready { .. })));
    }

    #[test]
    fn all_fields_but_no_service_name_returns_none() {
        // ARRANGE
        let text = "READY=1\nSTATUS=msg\nHEALTH=degraded\nSTOPPING=reason\nWATCHDOG=1";

        // ACT
        let result = parse_notification(text);

        // ASSERT
        assert!(result.is_none());
    }
}
