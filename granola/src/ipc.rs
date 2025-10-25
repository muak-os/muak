use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::process::{ProcessManager, ProcessStatus};
use crate::vm::{VmConfig, VmManager};

const SOCKET_PATH: &str = "/run/granola.sock";

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
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
    CreateVm {
        name: String,
        config: VmConfig,
    },
    StartVm {
        vm_id: String,
    },
    StopVm {
        vm_id: String,
        force: bool,
    },
    DeleteVm {
        vm_id: String,
    },
    ListVms,
    GetVm {
        vm_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok,
    ProcessList(Vec<u8>),
    ProcessStarted { pid: i32 },
    VmCreated { vm_id: String },
    VmList(Vec<u8>),
    Vm(Vec<u8>),
    Error(String),
}

pub struct IpcServer {
    listener: UnixListener,
}

impl IpcServer {
    pub fn new() -> Result<Self, String> {
        let _ = std::fs::remove_file(SOCKET_PATH);

        let listener =
            UnixListener::bind(SOCKET_PATH).map_err(|e| format!("Failed to bind socket: {}", e))?;

        Ok(Self { listener })
    }

    pub async fn accept_connection(&self) -> Result<UnixStream, String> {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .map_err(|e| format!("Failed to accept connection: {}", e))?;
        Ok(stream)
    }

    pub async fn read_message(&self, stream: &mut UnixStream) -> Result<IpcMessage, String> {
        let mut buf = [0u8; 4096];
        let size = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("Failed to read: {}", e))?;

        if size == 0 {
            return Err("Connection closed".to_string());
        }

        let data = &buf[..size];
        bincode::deserialize(data).map_err(|e| format!("Failed to deserialize: {}", e))
    }

    pub async fn send_response(
        &self,
        stream: &mut UnixStream,
        response: &IpcResponse,
    ) -> Result<(), String> {
        let data =
            bincode::serialize(response).map_err(|e| format!("Failed to serialize: {}", e))?;
        stream
            .write_all(&data)
            .await
            .map_err(|e| format!("Failed to write: {}", e))?;
        Ok(())
    }

    pub fn handle_message(
        &self,
        message: IpcMessage,
        process_manager: &ProcessManager,
        vm_manager: &VmManager,
    ) -> IpcResponse {
        match message {
            IpcMessage::UpdateStatus { pid, status } => {
                let process_status = match status.as_str() {
                    s if s.starts_with("exited(") => {
                        let code = s
                            .trim_start_matches("exited(")
                            .trim_end_matches(')')
                            .parse::<i32>()
                            .unwrap_or(0);
                        ProcessStatus::Exited(code)
                    }
                    s if s.starts_with("signaled(") => {
                        let sig = s
                            .trim_start_matches("signaled(")
                            .trim_end_matches(')')
                            .parse::<i32>()
                            .unwrap_or(0);
                        ProcessStatus::Signaled(sig)
                    }
                    _ => ProcessStatus::Running,
                };
                process_manager.update_status(pid, process_status);
                IpcResponse::Ok
            }
            IpcMessage::ListProcesses => {
                let processes = process_manager.list();
                match bincode::serialize(&processes) {
                    Ok(data) => IpcResponse::ProcessList(data),
                    Err(e) => IpcResponse::Error(format!("Serialization error: {}", e)),
                }
            }
            IpcMessage::StartProcess { command, args, env } => {
                match process_manager.spawn_external(command, args, env) {
                    Ok(pid) => IpcResponse::ProcessStarted { pid },
                    Err(e) => IpcResponse::Error(e),
                }
            }
            IpcMessage::StopProcess { pid, signal } => match process_manager.stop(pid, signal) {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::CreateVm { name, config } => match vm_manager.create(name, config) {
                Ok(vm_id) => IpcResponse::VmCreated { vm_id },
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::StartVm { vm_id } => match vm_manager.start(&vm_id) {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::StopVm { vm_id, force } => match vm_manager.stop(&vm_id, force) {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::DeleteVm { vm_id } => match vm_manager.delete(&vm_id) {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::ListVms => {
                let vms = vm_manager.list();
                match bincode::serialize(&vms) {
                    Ok(data) => IpcResponse::VmList(data),
                    Err(e) => IpcResponse::Error(format!("Serialization error: {}", e)),
                }
            }
            IpcMessage::GetVm { vm_id } => match vm_manager.get(&vm_id) {
                Some(vm) => match bincode::serialize(&vm) {
                    Ok(data) => IpcResponse::Vm(data),
                    Err(e) => IpcResponse::Error(format!("Serialization error: {}", e)),
                },
                None => IpcResponse::Error("VM not found".to_string()),
            },
        }
    }
}

pub struct IpcClient {
    socket: Option<StdUnixStream>,
}

impl IpcClient {
    pub fn new() -> Self {
        Self { socket: None }
    }

    pub fn connect(&mut self) -> Result<(), String> {
        let stream =
            StdUnixStream::connect(SOCKET_PATH).map_err(|e| format!("Failed to connect: {}", e))?;
        self.socket = Some(stream);
        Ok(())
    }

    pub fn send_message(&mut self, message: &IpcMessage) -> Result<IpcResponse, String> {
        if self.socket.is_none() {
            self.connect()?;
        }

        let result = self.try_send_message(message);
        
        if result.is_err() {
            self.socket = None;
            self.connect()?;
            self.try_send_message(message)
        } else {
            result
        }
    }

    fn try_send_message(&mut self, message: &IpcMessage) -> Result<IpcResponse, String> {
        let socket = self.socket.as_mut().ok_or("Not connected")?;

        let data =
            bincode::serialize(message).map_err(|e| format!("Failed to serialize: {}", e))?;
        socket
            .write_all(&data)
            .map_err(|e| format!("Failed to write: {}", e))?;
        socket.flush().map_err(|e| format!("Failed to flush: {}", e))?;

        let mut buf = [0u8; 65536];
        let size = socket
            .read(&mut buf)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        if size == 0 {
            return Err("Connection closed by server".to_string());
        }

        let response: IpcResponse = bincode::deserialize(&buf[..size])
            .map_err(|e| format!("Failed to deserialize response: {}", e))?;

        Ok(response)
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        self.socket.take();
    }
}
