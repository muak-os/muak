use nix::sys::socket::{
    accept, bind, connect, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
};
use nix::unistd::{read, write};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

const SOCKET_PATH: &str = "/run/granola.sock";

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    RegisterProcess {
        pid: i32,
        command: String,
        args: Vec<String>,
    },
    UpdateStatus {
        pid: i32,
        status: String,
    },
    ListProcesses,
    StartProcess {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    StopProcess {
        pid: i32,
        signal: i32,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok,
    ProcessList(Vec<u8>),
    ProcessStarted { pid: i32 },
    Error(String),
}

pub struct IpcServer {
    socket_fd: OwnedFd,
}

impl IpcServer {
    pub fn new() -> Result<Self, String> {
        let _ = std::fs::remove_file(SOCKET_PATH);

        let socket_fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .map_err(|e| format!("Failed to create socket: {}", e))?;

        let addr = UnixAddr::new(SOCKET_PATH)
            .map_err(|e| format!("Failed to create socket address: {}", e))?;

        bind(socket_fd.as_raw_fd(), &addr).map_err(|e| format!("Failed to bind socket: {}", e))?;

        listen(&socket_fd, Backlog::new(128).unwrap())
            .map_err(|e| format!("Failed to listen on socket: {}", e))?;

        Ok(Self { socket_fd })
    }

    pub fn accept_connection(&self) -> Result<OwnedFd, String> {
        let raw_fd = accept(self.socket_fd.as_raw_fd())
            .map_err(|e| format!("Failed to accept connection: {}", e))?;
        Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
    }

    pub fn read_message(&self, client_fd: &OwnedFd) -> Result<IpcMessage, String> {
        let mut buf = [0u8; 4096];
        let size = read(client_fd.as_raw_fd(), &mut buf).map_err(|e| format!("Failed to read: {}", e))?;

        if size == 0 {
            return Err("Connection closed".to_string());
        }

        let data = &buf[..size];
        bincode::deserialize(data).map_err(|e| format!("Failed to deserialize: {}", e))
    }

    pub fn send_response(&self, client_fd: &OwnedFd, response: &IpcResponse) -> Result<(), String> {
        let data =
            bincode::serialize(response).map_err(|e| format!("Failed to serialize: {}", e))?;
        write(client_fd.as_fd(), &data).map_err(|e| format!("Failed to write: {}", e))?;
        Ok(())
    }
}

pub struct IpcClient {
    socket_fd: Option<OwnedFd>,
}

impl IpcClient {
    pub fn new() -> Self {
        Self { socket_fd: None }
    }

    pub fn connect(&mut self) -> Result<(), String> {
        let socket_fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .map_err(|e| format!("Failed to create socket: {}", e))?;

        let addr = UnixAddr::new(SOCKET_PATH)
            .map_err(|e| format!("Failed to create socket address: {}", e))?;

        connect(socket_fd.as_raw_fd(), &addr).map_err(|e| format!("Failed to connect: {}", e))?;

        self.socket_fd = Some(socket_fd);
        Ok(())
    }

    pub fn send_message(&self, message: &IpcMessage) -> Result<IpcResponse, String> {
        let fd = self.socket_fd.as_ref().ok_or("Not connected")?;

        let data =
            bincode::serialize(message).map_err(|e| format!("Failed to serialize: {}", e))?;
        write(fd.as_fd(), &data).map_err(|e| format!("Failed to write: {}", e))?;

        let mut buf = [0u8; 65536];
        let size = read(fd.as_raw_fd(), &mut buf).map_err(|e| format!("Failed to read: {}", e))?;

        let response: IpcResponse = bincode::deserialize(&buf[..size])
            .map_err(|e| format!("Failed to deserialize response: {}", e))?;

        Ok(response)
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        self.socket_fd.take();
    }
}
