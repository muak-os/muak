use nix::libc;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

static KMSG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn log(msg: &str) {
    if let Some(kmsg) = KMSG.get() {
        if let Ok(mut file) = kmsg.lock() {
            let _ = writeln!(file, "<6>[granola] {}", msg);
        }
    }
}

fn log_error(msg: &str) {
    if let Some(kmsg) = KMSG.get() {
        if let Ok(mut file) = kmsg.lock() {
            let _ = writeln!(file, "<3>[granola] ERROR: {}", msg);
        }
    }
}

extern "C" fn handle_sigchld(_: libc::c_int) {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, status)) => {
                let msg = format!("Process {} exited with status {}", pid, status);
                if let Some(kmsg) = KMSG.get() {
                    if let Ok(mut file) = kmsg.lock() {
                        let _ = writeln!(file, "<6>[granola] {}", msg);
                    }
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                let msg = format!("Process {} killed by signal {:?}", pid, sig);
                if let Some(kmsg) = KMSG.get() {
                    if let Ok(mut file) = kmsg.lock() {
                        let _ = writeln!(file, "<6>[granola] {}", msg);
                    }
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

    unsafe {
        signal(Signal::SIGCHLD, SigHandler::Handler(handle_sigchld))?;
        signal(Signal::SIGTERM, SigHandler::Handler(handle_sigterm))?;
        signal(Signal::SIGINT, SigHandler::Handler(handle_sigint))?;
    }

    log("Signal handlers installed");
    log("PID 1 process reaping enabled");
    log("System ready");

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
