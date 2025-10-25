mod grpc;
mod ipc;
mod network_manager;
mod process;

use ipc::{IpcMessage, IpcResponse, IpcServer};
use nix::libc;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, ForkResult, Pid};
use process::{ProcessManager, ProcessStatus};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;

static PROCESS_MANAGER: OnceLock<ProcessManager> = OnceLock::new();

pub fn log(message: &str) {
    if let Ok(mut file) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = file.write_all(format!("<6>[granola] {}\n", message).as_bytes());
    }
}

extern "C" fn handle_sigchld(_: libc::c_int) {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, status)) => {
                if let Some(pm) = PROCESS_MANAGER.get() {
                    pm.update_status(pid.as_raw(), ProcessStatus::Exited(status));
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                if let Some(pm) = PROCESS_MANAGER.get() {
                    pm.update_status(pid.as_raw(), ProcessStatus::Signaled(sig as i32));
                }
            }
            Ok(WaitStatus::StillAlive) => break,
            Err(_) => break,
            _ => {}
        }
    }
}

extern "C" fn handle_sigterm(_: libc::c_int) {
    log("SIGTERM received, exiting");
    std::process::exit(0);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    log("SIGINT received, exiting");
    std::process::exit(0);
}

fn spawn_network_manager() -> Result<i32, String> {
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            let pid = child.as_raw();
            log(&format!("Spawned network-manager (PID {})", pid));
            Ok(pid)
        }
        Ok(ForkResult::Child) => {
            let _ = std::env::set_var("PROCESS_NAME", "network-manager");

            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                if let Err(e) = network_manager::network_manager_main().await {
                    log(&format!("Network manager error: {}", e));
                    std::process::exit(1);
                }
            });

            std::process::exit(0);
        }
        Err(e) => Err(format!("Failed to fork network-manager: {}", e)),
    }
}

fn spawn_grpc_server() -> Result<i32, String> {
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            let pid = child.as_raw();
            log(&format!("Spawned grpc-server (PID {})", pid));
            Ok(pid)
        }
        Ok(ForkResult::Child) => {
            let _ = std::env::set_var("PROCESS_NAME", "grpc-server");

            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                if let Err(e) = grpc::grpc_server_main().await {
                    log(&format!("gRPC server error: {}", e));
                    std::process::exit(1);
                }
            });

            std::process::exit(0);
        }
        Err(e) => Err(format!("Failed to fork grpc-server: {}", e)),
    }
}

fn handle_ipc_message(message: IpcMessage) -> IpcResponse {
    let pm = match PROCESS_MANAGER.get() {
        Some(pm) => pm,
        None => return IpcResponse::Error("Process manager not initialized".to_string()),
    };

    match message {
        IpcMessage::RegisterProcess { pid, command, args } => {
            pm.register(pid, command, args);
            IpcResponse::Ok
        }
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
            pm.update_status(pid, process_status);
            IpcResponse::Ok
        }
        IpcMessage::ListProcesses => {
            let processes = pm.list();
            match bincode::serialize(&processes) {
                Ok(data) => IpcResponse::ProcessList(data),
                Err(e) => IpcResponse::Error(format!("Serialization error: {}", e)),
            }
        }
        IpcMessage::StartProcess { command, args, env } => match pm.spawn(command, args, env) {
            Ok(pid) => IpcResponse::ProcessStarted { pid },
            Err(e) => IpcResponse::Error(e),
        },
        IpcMessage::StopProcess { pid, signal } => match pm.stop(pid, signal) {
            Ok(_) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error(e),
        },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    log("PID 1 init started");

    let process_manager = ProcessManager::new();
    PROCESS_MANAGER
        .set(process_manager.clone())
        .map_err(|_| "Failed to initialize process manager")?;

    unsafe {
        signal(Signal::SIGCHLD, SigHandler::Handler(handle_sigchld))?;
        signal(Signal::SIGTERM, SigHandler::Handler(handle_sigterm))?;
        signal(Signal::SIGINT, SigHandler::Handler(handle_sigint))?;
    }

    log("Signal handlers installed");

    let ipc_server = IpcServer::new()?;
    log("IPC server listening on /run/granola.sock");

    std::thread::sleep(std::time::Duration::from_millis(100));

    let network_pid = spawn_network_manager()?;
    process_manager.register(network_pid, "network-manager".to_string(), vec![]);

    let grpc_pid = spawn_grpc_server()?;
    process_manager.register(
        grpc_pid,
        "grpc-server".to_string(),
        vec!["0.0.0.0:50051".to_string()],
    );

    loop {
        let client_fd = match ipc_server.accept_connection() {
            Ok(fd) => fd,
            Err(_) => continue,
        };

        match ipc_server.read_message(&client_fd) {
            Ok(message) => {
                let response = handle_ipc_message(message);
                let _ = ipc_server.send_response(&client_fd, &response);
            }
            Err(_) => {}
        }
    }
}
