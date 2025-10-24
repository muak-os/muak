mod grpc_server;
mod network;
mod process;

use nix::libc;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use process::{ProcessManager, ProcessStatus};
use serde::Deserialize;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(extensions) = read_extensions() {
        if !extensions.is_empty() {
            log(&format!("Loaded extensions (count > 0): {:?}", extensions));
        }
    }

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
    log("Setting up network");

    let network_handle = network::setup_networking().await?;

    log("Starting gRPC server on 0.0.0.0:50051");
    let server_result = grpc_server::run_grpc_server(process_manager).await;

    drop(network_handle);
    server_result?;

    Ok(())
}

fn read_extensions() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    #[derive(Deserialize)]
    struct ExtensionManifest {
        #[serde(default)]
        extensions: Vec<ExtensionEntry>,
    }

    #[derive(Deserialize)]
    struct ExtensionEntry {
        name: String,
    }

    let manifest_path = "/etc/extensions.yaml";
    if !std::path::Path::new(manifest_path).exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(manifest_path)?;
    let manifest: ExtensionManifest = serde_yaml::from_str(&content)?;
    Ok(manifest.extensions.into_iter().map(|e| e.name).collect())
}
