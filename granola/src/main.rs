mod grpc;
mod ipc;
mod log;
mod network;
mod process;

use ipc::IpcServer;
use nix::libc;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use process::{ProcessManager, ProcessStatus};
use std::sync::OnceLock;

static PROCESS_MANAGER: OnceLock<ProcessManager> = OnceLock::new();

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
    log!("granola", "SIGTERM received, exiting");
    std::process::exit(0);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    log!("granola", "SIGINT received, exiting");
    std::process::exit(0);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    log!("granola", "PID 1 init started");

    let process_manager = ProcessManager::new();
    PROCESS_MANAGER
        .set(process_manager.clone())
        .map_err(|_| "Failed to initialize process manager")?;

    unsafe {
        signal(Signal::SIGCHLD, SigHandler::Handler(handle_sigchld))?;
        signal(Signal::SIGTERM, SigHandler::Handler(handle_sigterm))?;
        signal(Signal::SIGINT, SigHandler::Handler(handle_sigint))?;
    }

    log!("granola", "Signal handlers installed");

    let ipc_server = IpcServer::new()?;
    log!("granola", "IPC server listening on /run/granola.sock");

    let pid = process_manager.spawn_service("network-manager", vec![], network::main)?;
    log!("granola", "Spawned network-manager (PID {})", pid);

    let pid = process_manager.spawn_service(
        "grpc-server",
        vec!["0.0.0.0:50051".to_string()],
        grpc::main,
    )?;
    log!("granola", "Spawned grpc-server (PID {})", pid);

    loop {
        let client_fd = match ipc_server.accept_connection() {
            Ok(fd) => fd,
            Err(_) => continue,
        };

        match ipc_server.read_message(&client_fd) {
            Ok(message) => {
                let response = ipc_server.handle_message(message, &process_manager);
                let _ = ipc_server.send_response(&client_fd, &response);
            }
            Err(_) => {}
        }
    }
}
