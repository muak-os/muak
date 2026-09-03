//! Notify client for sending notifications to the supervisor.

use std::io;
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;

const DEFAULT_NOTIFY_SOCKET: &str = "/run/services/granola-notify.sock";

/// Health status for a service status notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Degraded,
    Unhealthy,
}

impl Health {
    /// Returns the wire representation of this health value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unhealthy => "unhealthy",
        }
    }
}

impl FromStr for Health {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "healthy" => Ok(Self::Healthy),
            "degraded" => Ok(Self::Degraded),
            "unhealthy" => Ok(Self::Unhealthy),
            _ => Err(()),
        }
    }
}

/// Client for sending notifications to the supervisor.
pub struct Notifier {
    socket: UnixDatagram,
    service_name: String,
    socket_path: String,
}

impl Notifier {
    /// Creates a new notify client with default socket path.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying datagram socket cannot be created.
    pub fn new(service_name: &str) -> Result<Self, io::Error> {
        Self::new_with_socket(service_name, DEFAULT_NOTIFY_SOCKET)
    }

    /// Creates a new notify client with custom socket path.
    fn new_with_socket(service_name: &str, socket_path: &str) -> Result<Self, io::Error> {
        let socket = UnixDatagram::unbound()?;
        Ok(Self {
            socket,
            service_name: service_name.to_owned(),
            socket_path: socket_path.to_owned(),
        })
    }

    /// Notifies supervisor that service is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be delivered. Missing or
    /// unresponsive supervisors are tolerated and do not produce an error.
    pub fn ready(&self) -> Result<(), io::Error> {
        let msg = format!(
            "SERVICE_NAME={}\nREADY={}",
            self.service_name,
            std::process::id()
        );
        self.send(msg.as_bytes())
    }

    /// Sends status message to supervisor.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be delivered. Missing or
    /// unresponsive supervisors are tolerated and do not produce an error.
    pub fn status(&self, message: &str, health: Health) -> Result<(), io::Error> {
        let msg = format!(
            "SERVICE_NAME={}\nSTATUS={}\nHEALTH={}",
            self.service_name,
            message,
            health.as_str()
        );
        self.send(msg.as_bytes())
    }

    /// Notifies supervisor that service is stopping.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be delivered. Missing or
    /// unresponsive supervisors are tolerated and do not produce an error.
    pub fn stopping(&self, reason: &str) -> Result<(), io::Error> {
        let msg = format!("SERVICE_NAME={}\nSTOPPING={}", self.service_name, reason);
        self.send(msg.as_bytes())
    }

    /// Sends watchdog keepalive to supervisor.
    ///
    /// # Errors
    ///
    /// Returns an error if the notification cannot be delivered. Missing or
    /// unresponsive supervisors are tolerated and do not produce an error.
    pub fn watchdog(&self) -> Result<(), io::Error> {
        let msg = format!("SERVICE_NAME={}\nWATCHDOG=1", self.service_name);
        self.send(msg.as_bytes())
    }

    fn send(&self, bytes: &[u8]) -> Result<(), io::Error> {
        match self.socket.send_to(bytes, &self.socket_path) {
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

    fn test_client_with_server(service: &str) -> (Notifier, UnixDatagram, TempDir) {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("notify.sock");
        let server = UnixDatagram::bind(&socket_path).expect("Failed to bind");
        let client = Notifier::new_with_socket(service, socket_path.to_str().expect("valid path"))
            .expect("Failed to create client");
        (client, server, dir)
    }

    fn recv_str(socket: &UnixDatagram) -> String {
        let mut buf = vec![0_u8; 4096];
        let (len, _) = socket.recv_from(&mut buf).expect("Failed to receive");
        let bytes = buf.get(..len).expect("received length within buffer");
        String::from_utf8(bytes.to_vec()).expect("Invalid UTF-8")
    }

    #[test]
    fn notify_client_new() {
        // ACT
        let client = Notifier::new("test-service").expect("Failed to create client");

        // ASSERT
        assert_eq!(client.service_name, "test-service");
        assert_eq!(client.socket_path, DEFAULT_NOTIFY_SOCKET);
    }

    #[test]
    fn notify_client_new_with_custom_socket() {
        // ACT
        let client = Notifier::new_with_socket("test-service", "/tmp/test.sock")
            .expect("Failed to create client");

        // ASSERT
        assert_eq!(client.service_name, "test-service");
        assert_eq!(client.socket_path, "/tmp/test.sock");
    }

    #[test]
    fn ready_no_server() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = Notifier::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        // ACT & ASSERT
        client
            .ready()
            .expect("ready should tolerate missing server");
    }

    #[test]
    fn status_no_server() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = Notifier::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        // ACT & ASSERT
        client
            .status("Service is running", Health::Healthy)
            .expect("status should tolerate missing server");
    }

    #[test]
    fn stopping_no_server() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = Notifier::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        // ACT & ASSERT
        client
            .stopping("Shutting down gracefully")
            .expect("stopping should tolerate missing server");
    }

    #[test]
    fn watchdog_no_server() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("nonexistent.sock");
        let client = Notifier::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        // ACT & ASSERT
        client
            .watchdog()
            .expect("watchdog should tolerate missing server");
    }

    #[test]
    fn ready_wire_format() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test-service");
        let pid = std::process::id();

        // ACT
        client.ready().expect("Failed to send ready");

        // ASSERT
        assert_eq!(
            recv_str(&server),
            format!("SERVICE_NAME=test-service\nREADY={pid}")
        );
    }

    #[test]
    fn status_wire_format() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test-service");

        // ACT & ASSERT
        client
            .status("Service is healthy", Health::Healthy)
            .expect("Failed to send status");
        assert_eq!(
            recv_str(&server),
            "SERVICE_NAME=test-service\nSTATUS=Service is healthy\nHEALTH=healthy"
        );

        client
            .status("Degraded", Health::Degraded)
            .expect("Failed to send status");
        assert_eq!(
            recv_str(&server),
            "SERVICE_NAME=test-service\nSTATUS=Degraded\nHEALTH=degraded"
        );

        client
            .status("Down", Health::Unhealthy)
            .expect("Failed to send status");
        assert_eq!(
            recv_str(&server),
            "SERVICE_NAME=test-service\nSTATUS=Down\nHEALTH=unhealthy"
        );
    }

    #[test]
    fn stopping_wire_format() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test-service");

        // ACT
        client.stopping("Received SIGTERM").expect("Failed to send");

        // ASSERT
        assert_eq!(
            recv_str(&server),
            "SERVICE_NAME=test-service\nSTOPPING=Received SIGTERM"
        );
    }

    #[test]
    fn watchdog_wire_format() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test-service");

        // ACT
        client.watchdog().expect("Failed to send");

        // ASSERT
        assert_eq!(recv_str(&server), "SERVICE_NAME=test-service\nWATCHDOG=1");
    }

    #[test]
    fn multiple_notifications() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test-service");

        // ACT
        client.ready().expect("Failed to send ready");
        client
            .status("Running", Health::Healthy)
            .expect("Failed to send status");
        client.watchdog().expect("Failed to send watchdog");

        // ASSERT
        let pid = std::process::id();
        assert_eq!(
            recv_str(&server),
            format!("SERVICE_NAME=test-service\nREADY={pid}")
        );
        assert_eq!(
            recv_str(&server),
            "SERVICE_NAME=test-service\nSTATUS=Running\nHEALTH=healthy"
        );
        assert_eq!(recv_str(&server), "SERVICE_NAME=test-service\nWATCHDOG=1");
    }

    #[test]
    fn socket_permissions_error() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let socket_path = dir.path().join("readonly.sock");

        fs::write(&socket_path, "").expect("Failed to create file");
        let mut perms = fs::metadata(&socket_path)
            .expect("Failed to get metadata")
            .permissions();
        perms.set_readonly(true);
        fs::set_permissions(&socket_path, perms).expect("Failed to set permissions");

        let client = Notifier::new_with_socket("test-service", socket_path.to_str().unwrap())
            .expect("Failed to create client");

        // ACT
        drop(client.watchdog());
        drop(fs::remove_file(&socket_path));
    }

    #[test]
    fn health_from_str_roundtrip() {
        // ACT & ASSERT
        for health in [Health::Healthy, Health::Degraded, Health::Unhealthy] {
            assert_eq!(health.as_str().parse::<Health>(), Ok(health));
        }
        let unknown: Result<Health, ()> = "unknown".parse();
        unknown.expect_err("unknown should not parse");
    }

    #[test]
    fn service_name_special_characters() {
        // ARRANGE
        let (_, server, dir) = test_client_with_server("placeholder");
        let socket_path = dir.path().join("notify.sock");

        // ACT & ASSERT
        for name in ["service-dashes", "service.dots", "service_underscores"] {
            let client = Notifier::new_with_socket(name, socket_path.to_str().expect("valid path"))
                .expect("Failed to create client");
            client.watchdog().expect("Failed to send");
            let msg = recv_str(&server);
            assert!(msg.starts_with(&format!("SERVICE_NAME={name}\n")));
        }
    }

    #[test]
    fn long_status_message() {
        // ARRANGE
        let (client, server, _dir) = test_client_with_server("test");

        // ACT
        let long_msg = "x".repeat(1000);
        client
            .status(&long_msg, Health::Healthy)
            .expect("Failed to send");

        // ASSERT
        let received = recv_str(&server);
        assert!(received.contains(&long_msg));
    }
}
