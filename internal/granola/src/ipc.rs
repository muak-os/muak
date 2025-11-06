use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::process::{ProcessManager, ProcessStatus};
use crate::vm::{VmConfig, VmManager};

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    UpdateStatus { pid: i32, status: String },
    ListProcesses,
    StartProcess { command: String, args: Vec<String> },
    StopProcess { pid: i32, signal: i32 },
    CreateVm { name: String, config: VmConfig },
    StartVm { vm_id: String },
    StopVm { vm_id: String, force: bool },
    DeleteVm { vm_id: String },
    ListVms,
    GetVm { vm_id: String },
    GetVmSerialLog { vm_id: String, tail_lines: i64 },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok,
    ProcessList(Vec<u8>),
    ProcessStarted { pid: i32 },
    VmCreated { vm_id: String },
    VmList(Vec<u8>),
    Vm(Vec<u8>),
    VmSerialLog(String),
    Error(String),
}

pub struct IpcServer {
    listener: UnixListener,
}

impl IpcServer {
    pub fn new() -> Result<Self, String> {
        let _ = std::fs::remove_file(crate::config::GRANOLA_SOCKET_PATH);

        let listener = UnixListener::bind(crate::config::GRANOLA_SOCKET_PATH)
            .map_err(|e| format!("Failed to bind socket: {}", e))?;

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
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| format!("Failed to read message length: {}", e))?;

        let msg_len = u32::from_le_bytes(len_buf) as usize;

        if msg_len > crate::config::IPC_MAX_MESSAGE_SIZE {
            return Err(format!("Message too large: {} bytes", msg_len));
        }

        let mut buf = vec![0u8; msg_len];
        stream
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("Failed to read message: {}", e))?;

        bincode::deserialize(&buf).map_err(|e| format!("Failed to deserialize: {}", e))
    }

    pub async fn send_response(
        &self,
        stream: &mut UnixStream,
        response: &IpcResponse,
    ) -> Result<(), String> {
        let data =
            bincode::serialize(response).map_err(|e| format!("Failed to serialize: {}", e))?;

        let len = data.len() as u32;
        let len_bytes = len.to_le_bytes();

        stream
            .write_all(&len_bytes)
            .await
            .map_err(|e| format!("Failed to write length: {}", e))?;
        stream
            .write_all(&data)
            .await
            .map_err(|e| format!("Failed to write: {}", e))?;
        Ok(())
    }

    pub async fn handle_message(
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
            IpcMessage::StartProcess { command, args } => {
                match process_manager.spawn_external(command, args) {
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
            IpcMessage::StartVm { vm_id } => match vm_manager.start(&vm_id).await {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error(e),
            },
            IpcMessage::StopVm { vm_id, force } => match vm_manager.stop(&vm_id, force).await {
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
            IpcMessage::GetVmSerialLog { vm_id, tail_lines } => {
                let log_path = format!("/run/{}-serial.log", vm_id);
                match std::fs::read_to_string(&log_path) {
                    Ok(content) => {
                        if tail_lines > 0 {
                            let lines: Vec<&str> = content.lines().collect();
                            let start = lines.len().saturating_sub(tail_lines as usize);
                            let output = lines[start..].join("\n");
                            IpcResponse::VmSerialLog(output)
                        } else {
                            IpcResponse::VmSerialLog(content)
                        }
                    }
                    Err(e) => IpcResponse::Error(format!("Failed to read serial log: {}", e)),
                }
            }
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
        let stream = StdUnixStream::connect(crate::config::GRANOLA_SOCKET_PATH)
            .map_err(|e| format!("Failed to connect: {}", e))?;
        self.socket = Some(stream);
        Ok(())
    }

    pub fn send_message(&mut self, message: &IpcMessage) -> Result<IpcResponse, String> {
        if self.socket.is_none() {
            self.connect()?;
        }

        let result = self.try_send_message(message);

        if let Err(ref e) = result {
            if self.should_reconnect(e) {
                self.socket = None;
                self.connect()?;
                return self.try_send_message(message);
            }
        }

        result
    }

    fn should_reconnect(&self, error: &str) -> bool {
        error.contains("Broken pipe")
            || error.contains("Connection reset")
            || error.contains("Not connected")
    }

    fn try_send_message(&mut self, message: &IpcMessage) -> Result<IpcResponse, String> {
        let socket = self.socket.as_mut().ok_or("Not connected")?;

        let data =
            bincode::serialize(message).map_err(|e| format!("Failed to serialize: {}", e))?;

        let len = data.len() as u32;
        let len_bytes = len.to_le_bytes();

        socket
            .write_all(&len_bytes)
            .map_err(|e| format!("Failed to write length: {}", e))?;
        socket
            .write_all(&data)
            .map_err(|e| format!("Failed to write: {}", e))?;
        socket
            .flush()
            .map_err(|e| format!("Failed to flush: {}", e))?;

        let mut len_buf = [0u8; 4];
        socket
            .read_exact(&mut len_buf)
            .map_err(|e| format!("Failed to read response length: {}", e))?;

        let msg_len = u32::from_le_bytes(len_buf) as usize;

        if msg_len > crate::config::IPC_MAX_MESSAGE_SIZE {
            return Err(format!("Response too large: {} bytes", msg_len));
        }

        let mut buf = vec![0u8; msg_len];
        socket
            .read_exact(&mut buf)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let response: IpcResponse = bincode::deserialize(&buf)
            .map_err(|e| format!("Failed to deserialize response: {}", e))?;

        Ok(response)
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        self.socket.take();
    }
}
