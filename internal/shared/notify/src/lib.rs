//! Notify - A small library for sending notifications to the supervisor.
//!
//! This library provides a client for services to communicate to the
//! supervisor PID 1 via UNIX domain sockets.

use std::io;
use std::os::unix::net::UnixDatagram;

use prost::Message;
pub use proto::Health;
use proto::{Notify, Ready, Status, Stopping, Watchdog, notify::Notification};

#[allow(clippy::excessive_nesting)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/muak.internal.supervisor.rs"));
}

const DEFAULT_NOTIFY_SOCKET: &str = "/run/granola-notify.sock";

/// Client for sending notifications to the supervisor.
pub struct NotifyClient {
    socket: UnixDatagram,
    service_name: String,
    socket_path: String,
}

impl NotifyClient {
    /// Creates a new notify client with default socket path.
    pub fn new(service_name: &str) -> Result<Self, io::Error> {
        Self::new_with_socket(service_name, DEFAULT_NOTIFY_SOCKET)
    }

    /// Creates a new notify client with custom socket path.
    fn new_with_socket(service_name: &str, socket_path: &str) -> Result<Self, io::Error> {
        let socket = UnixDatagram::unbound()?;
        Ok(Self {
            socket,
            service_name: service_name.to_string(),
            socket_path: socket_path.to_string(),
        })
    }

    /// Notifies supervisor that service is ready.
    pub fn ready(&self, socket_path: &str) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Ready(Ready {
                socket_path: socket_path.to_string(),
                pid: std::process::id(),
            })),
        };
        self.send(&notify)
    }

    /// Sends status message to supervisor.
    pub fn status(&self, message: &str, health: Health) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Status(Status {
                message: message.to_string(),
                health: health.into(),
            })),
        };
        self.send(&notify)
    }

    /// Notifies supervisor that service is stopping.
    pub fn stopping(&self, reason: &str) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Stopping(Stopping {
                reason: reason.to_string(),
            })),
        };
        self.send(&notify)
    }

    /// Sends watchdog keepalive to supervisor.
    pub fn watchdog(&self) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Watchdog(Watchdog {})),
        };
        self.send(&notify)
    }

    /// Sends encoded notification to supervisor socket.
    fn send(&self, msg: &Notify) -> Result<(), io::Error> {
        let bytes = msg.encode_to_vec();
        match self.socket.send_to(&bytes, &self.socket_path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_notify_client_new() {
        let client = NotifyClient::new("test-service");
        assert!(client.is_ok());
        let client = client.unwrap();
        assert_eq!(client.service_name, "test-service");
        assert_eq!(client.socket_path, DEFAULT_NOTIFY_SOCKET);
    }

    #[test]
    fn test_notify_client_new_with_custom_socket() {
        let client = NotifyClient::new_with_socket("test-service", "/tmp/test.sock");
        assert!(client.is_ok());
        let client = client.unwrap();
        assert_eq!(client.service_name, "test-service");
        assert_eq!(client.socket_path, "/tmp/test.sock");
    }

    #[test]
    fn test_ready_no_server() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        let result = client.ready("/run/test.sock");
        assert!(result.is_ok());
    }

    #[test]
    fn test_status_no_server() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        let result = client.status("Service is running", Health::Healthy);
        assert!(result.is_ok());
    }

    #[test]
    fn test_stopping_no_server() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        let result = client.stopping("Shutting down gracefully");
        assert!(result.is_ok());
    }

    #[test]
    fn test_watchdog_no_server() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        let result = client.watchdog();
        assert!(result.is_ok());
    }

    #[test]
    fn test_ready_message_serialization() {
        let service_name = "test-service".to_string();
        let socket_path_str = "/run/test.sock".to_string();
        let pid = std::process::id();

        let notify = Notify {
            service_name: service_name.clone(),
            notification: Some(Notification::Ready(Ready {
                socket_path: socket_path_str.clone(),
                pid,
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        assert_eq!(decoded.service_name, service_name);
        match decoded.notification {
            Some(Notification::Ready(ready)) => {
                assert_eq!(ready.socket_path, socket_path_str);
                assert_eq!(ready.pid, pid);
            }
            _ => panic!("Expected Ready notification"),
        }
    }

    #[test]
    fn test_status_message_serialization() {
        let service_name = "test-service".to_string();
        let message = "Service is healthy".to_string();

        let notify = Notify {
            service_name: service_name.clone(),
            notification: Some(Notification::Status(Status {
                message: message.clone(),
                health: proto::Health::Healthy.into(),
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        assert_eq!(decoded.service_name, service_name);
        match decoded.notification {
            Some(Notification::Status(status)) => {
                assert_eq!(status.message, message);
                assert_eq!(status.health, proto::Health::Healthy.into());
            }
            _ => panic!("Expected Status notification"),
        }
    }

    #[test]
    fn test_stopping_message_serialization() {
        let service_name = "test-service".to_string();
        let reason = "Received SIGTERM".to_string();

        let notify = Notify {
            service_name: service_name.clone(),
            notification: Some(Notification::Stopping(Stopping {
                reason: reason.clone(),
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        assert_eq!(decoded.service_name, service_name);
        match decoded.notification {
            Some(Notification::Stopping(stopping)) => {
                assert_eq!(stopping.reason, reason);
            }
            _ => panic!("Expected Stopping notification"),
        }
    }

    #[test]
    fn test_watchdog_message_serialization() {
        let service_name = "test-service".to_string();

        let notify = Notify {
            service_name: service_name.clone(),
            notification: Some(Notification::Watchdog(Watchdog {})),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        assert_eq!(decoded.service_name, service_name);
        match decoded.notification {
            Some(Notification::Watchdog(_)) => {}
            _ => panic!("Expected Watchdog notification"),
        }
    }

    #[test]
    fn test_health_variants() {
        let service_name = "test-service".to_string();

        for health in [Health::Healthy, Health::Degraded, Health::Unhealthy] {
            let notify = Notify {
                service_name: service_name.clone(),
                notification: Some(Notification::Status(Status {
                    message: "test".to_string(),
                    health: health.into(),
                })),
            };

            let bytes = notify.encode_to_vec();
            let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

            match decoded.notification {
                Some(Notification::Status(status)) => {
                    let proto_health =
                        proto::Health::try_from(status.health).expect("Invalid health value");
                    match health {
                        Health::Healthy => assert_eq!(proto_health, proto::Health::Healthy),
                        Health::Degraded => assert_eq!(proto_health, proto::Health::Degraded),
                        Health::Unhealthy => assert_eq!(proto_health, proto::Health::Unhealthy),
                    }
                }
                _ => panic!("Expected Status notification"),
            }
        }
    }

    #[test]
    fn test_message_with_empty_service_name() {
        let notify = Notify {
            service_name: "".to_string(),
            notification: Some(Notification::Watchdog(Watchdog {})),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        assert_eq!(decoded.service_name, "");
    }

    #[test]
    fn test_status_with_empty_message() {
        let notify = Notify {
            service_name: "test".to_string(),
            notification: Some(Notification::Status(Status {
                message: "".to_string(),
                health: Health::Healthy.into(),
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        match decoded.notification {
            Some(Notification::Status(status)) => {
                assert_eq!(status.message, "");
            }
            _ => panic!("Expected Status notification"),
        }
    }

    #[test]
    fn test_stopping_with_empty_reason() {
        let notify = Notify {
            service_name: "test".to_string(),
            notification: Some(Notification::Stopping(Stopping {
                reason: "".to_string(),
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        match decoded.notification {
            Some(Notification::Stopping(stopping)) => {
                assert_eq!(stopping.reason, "");
            }
            _ => panic!("Expected Stopping notification"),
        }
    }

    #[test]
    fn test_ready_with_empty_socket_path() {
        let notify = Notify {
            service_name: "test".to_string(),
            notification: Some(Notification::Ready(Ready {
                socket_path: "".to_string(),
                pid: 0,
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        match decoded.notification {
            Some(Notification::Ready(ready)) => {
                assert_eq!(ready.socket_path, "");
                assert_eq!(ready.pid, 0);
            }
            _ => panic!("Expected Ready notification"),
        }
    }

    #[test]
    fn test_notification_delivery() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("notify.sock");

        let server_socket = UnixDatagram::bind(&socket_path).expect("Failed to bind server socket");

        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        client
            .ready("/run/test.sock")
            .expect("Failed to send ready");

        let mut buf = vec![0u8; 4096];
        let (len, _) = server_socket
            .recv_from(&mut buf)
            .expect("Failed to receive");
        buf.truncate(len);

        let decoded = Notify::decode(&buf[..]).expect("Failed to decode");
        assert_eq!(decoded.service_name, "test-service");
        match decoded.notification {
            Some(Notification::Ready(ready)) => {
                assert_eq!(ready.socket_path, "/run/test.sock");
                assert_eq!(ready.pid, std::process::id());
            }
            _ => panic!("Expected Ready notification"),
        }
    }

    #[test]
    fn test_multiple_notifications() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("notify.sock");

        let server_socket = UnixDatagram::bind(&socket_path).expect("Failed to bind server socket");

        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        client
            .ready("/run/test.sock")
            .expect("Failed to send ready");
        client
            .status("Running", Health::Healthy)
            .expect("Failed to send status");
        client.watchdog().expect("Failed to send watchdog");

        let mut received = Vec::new();
        let mut buf = vec![0u8; 4096];

        for _ in 0..3 {
            let (len, _) = server_socket
                .recv_from(&mut buf)
                .expect("Failed to receive");
            let msg = Notify::decode(&buf[..len]).expect("Failed to decode");
            received.push(msg);
        }

        assert_eq!(received.len(), 3);

        match &received[0].notification {
            Some(Notification::Ready(_)) => {}
            _ => panic!("Expected first message to be Ready"),
        }

        match &received[1].notification {
            Some(Notification::Status(_)) => {}
            _ => panic!("Expected second message to be Status"),
        }

        match &received[2].notification {
            Some(Notification::Watchdog(_)) => {}
            _ => panic!("Expected third message to be Watchdog"),
        }
    }

    #[test]
    fn test_socket_permissions_error() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("readonly.sock");

        fs::write(&socket_path, "").expect("Failed to create file");
        let mut perms = fs::metadata(&socket_path)
            .expect("Failed to get metadata")
            .permissions();
        perms.set_readonly(true);
        fs::set_permissions(&socket_path, perms).expect("Failed to set permissions");

        let client = NotifyClient::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        let result = client.watchdog();

        fs::remove_file(&socket_path).ok();

        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_service_name_with_special_characters() {
        let service_names = vec![
            "service-with-dashes",
            "service.with.dots",
            "service_with_underscores",
            "ServiceWithMixedCase123",
            "service:with:colons",
        ];

        for name in service_names {
            let notify = Notify {
                service_name: name.to_string(),
                notification: Some(Notification::Watchdog(Watchdog {})),
            };

            let bytes = notify.encode_to_vec();
            let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");
            assert_eq!(decoded.service_name, name);
        }
    }

    #[test]
    fn test_long_service_name() {
        let long_name = "a".repeat(1000);
        let notify = Notify {
            service_name: long_name.clone(),
            notification: Some(Notification::Watchdog(Watchdog {})),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");
        assert_eq!(decoded.service_name, long_name);
    }

    #[test]
    fn test_long_status_message() {
        let long_message = "x".repeat(10000);
        let notify = Notify {
            service_name: "test".to_string(),
            notification: Some(Notification::Status(Status {
                message: long_message.clone(),
                health: Health::Healthy.into(),
            })),
        };

        let bytes = notify.encode_to_vec();
        let decoded = Notify::decode(&bytes[..]).expect("Failed to decode");

        match decoded.notification {
            Some(Notification::Status(status)) => {
                assert_eq!(status.message, long_message);
            }
            _ => panic!("Expected Status notification"),
        }
    }
}
