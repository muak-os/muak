use prost::Message;
pub use proto::Health;
use proto::{Notify, Ready, Status, Stopping, Watchdog, notify::Notification};
use std::io;
use std::os::unix::net::UnixDatagram;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/muak.internal.supervisor.rs"));
}

const NOTIFY_SOCKET: &str = "/run/granola.sock";

pub struct NotifyClient {
    socket: UnixDatagram,
    service_name: String,
}

impl NotifyClient {
    pub fn new(service_name: &str) -> Result<Self, io::Error> {
        let socket = UnixDatagram::unbound()?;
        Ok(Self {
            socket,
            service_name: service_name.to_string(),
        })
    }

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

    pub fn stopping(&self, reason: &str) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Stopping(Stopping {
                reason: reason.to_string(),
            })),
        };
        self.send(&notify)
    }

    pub fn watchdog(&self) -> Result<(), io::Error> {
        let notify = Notify {
            service_name: self.service_name.clone(),
            notification: Some(Notification::Watchdog(Watchdog {})),
        };
        self.send(&notify)
    }

    fn send(&self, msg: &Notify) -> Result<(), io::Error> {
        let bytes = msg.encode_to_vec();
        match self.socket.send_to(&bytes, NOTIFY_SOCKET) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}
