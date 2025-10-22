mod grpc_server;
mod process;

use nix::libc;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use process::{ProcessManager, ProcessStatus};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

static KMSG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static PROCESS_MANAGER: OnceLock<ProcessManager> = OnceLock::new();

fn log(msg: &str) {
    if let Some(kmsg) = KMSG.get() {
        if let Ok(mut file) = kmsg.lock() {
            let log_line = format!("<6>[granola] {}\n", msg);
            let _ = file.write_all(log_line.as_bytes());
        }
    }
}

fn log_error(msg: &str) {
    if let Some(kmsg) = KMSG.get() {
        if let Ok(mut file) = kmsg.lock() {
            let log_line = format!("<3>[granola] ERROR: {}\n", msg);
            let _ = file.write_all(log_line.as_bytes());
        }
    }
}

extern "C" fn handle_sigchld(_: libc::c_int) {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, status)) => {
                log(&format!(
                    "<6>[granola] Process {} exited with status {}\n",
                    pid, status
                ));
                if let Some(pm) = PROCESS_MANAGER.get() {
                    pm.update_status(pid.as_raw(), ProcessStatus::Exited(status));
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                log(&format!("Process {} killed by signal {:?}", pid, sig));
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
    log("Received SIGTERM, shutting down gracefully");
    std::process::exit(0);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    log("Received SIGINT, shutting down gracefully");
    std::process::exit(0);
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Granola init failed: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let kmsg = OpenOptions::new().write(true).open("/dev/kmsg")?;
    KMSG.set(Mutex::new(kmsg))
        .map_err(|_| "Failed to initialize kmsg")?;

    log("Granola init system starting");

    if let Ok(extensions) = read_extensions() {
        if !extensions.is_empty() {
            log(&format!("Loaded extensions: {}", extensions.join(", ")));
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
    log("PID 1 process reaping enabled");

    let pm = process_manager.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            log("Starting gRPC server on 0.0.0.0:50051");
            if let Err(e) = grpc_server::run_grpc_server(pm).await {
                log_error(&format!("gRPC server error: {}", e));
            }
        });
    });

    log("System ready");

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn read_extensions() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    use serde::Deserialize;
    use std::fs;

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

    let content = fs::read_to_string(manifest_path)?;
    let manifest: ExtensionManifest = serde_yaml::from_str(&content)?;
    Ok(manifest.extensions.into_iter().map(|e| e.name).collect())
}
