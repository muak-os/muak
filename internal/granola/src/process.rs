use nix::fcntl::{OFlag, open};
use nix::sys::signal::{Signal, kill};
use nix::sys::stat::Mode;
use nix::unistd::{ForkResult, Pid, dup2, fork};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CString;
use std::fmt;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Process {
    pub pid: i32,
    pub command: String,
    pub args: Vec<String>,
    pub status: ProcessStatus,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProcessStatus {
    Running,
    Exited(i32),
    Signaled(i32),
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessStatus::Running => write!(f, "running"),
            ProcessStatus::Exited(code) => write!(f, "exited({})", code),
            ProcessStatus::Signaled(sig) => write!(f, "signaled({})", sig),
        }
    }
}

#[derive(Clone)]
pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<i32, Process>>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn register_process(&self, pid: i32, command: String, args: Vec<String>) {
        let process = Process {
            pid,
            command,
            args,
            status: ProcessStatus::Running,
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("FATAL: system time is before UNIX epoch")
                .as_secs() as i64,
        };

        let mut processes = self
            .processes
            .lock()
            .expect("FATAL: ProcessManager mutex poisoned - this is a critical PID 1 failure");
        processes.insert(pid, process);
    }

    pub fn spawn_service<F, Fut>(
        &self,
        name: &str,
        args: Vec<String>,
        service_main: F,
    ) -> Result<i32, String>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + 'static,
    {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                let pid = child.as_raw();
                self.register_process(pid, name.to_string(), args);
                Ok(pid)
            }
            Ok(ForkResult::Child) => {
                let runtime = tokio::runtime::Runtime::new()
                    .expect("FATAL: failed to create tokio runtime in child process");
                runtime.block_on(async {
                    if let Err(e) = service_main().await {
                        eprintln!("{} error: {}", name, e);
                        std::process::exit(1);
                    }
                });

                std::process::exit(0);
            }
            Err(e) => Err(format!("Failed to fork {}: {}", name, e)),
        }
    }

    pub fn spawn_external(&self, command: String, args: Vec<String>) -> Result<i32, String> {
        self.spawn_external_with_redirect(command, args, None, None)
    }

    pub fn spawn_external_with_redirect(
        &self,
        command: String,
        args: Vec<String>,
        stdout_path: Option<String>,
        stderr_path: Option<String>,
    ) -> Result<i32, String> {
        if !std::path::Path::new(&command).exists() {
            return Err(format!("Command not found: {}", command));
        }

        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                let pid = child.as_raw();
                self.register_process(pid, command.clone(), args.clone());
                crate::log!(
                    "process",
                    "Spawned external process: {} (PID: {}) with args: {:?}",
                    command,
                    pid,
                    args
                );
                Ok(pid)
            }
            Ok(ForkResult::Child) => {
                // Redirect stdout if requested
                if let Some(stdout_file) = stdout_path {
                    let path = Path::new(&stdout_file);
                    match open(
                        path,
                        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC,
                        Mode::from_bits_truncate(0o644),
                    ) {
                        Ok(fd) => {
                            let _ = dup2(fd as RawFd, 1); // Redirect stdout
                            let _ = nix::unistd::close(fd as RawFd);
                        }
                        Err(e) => {
                            eprintln!("Failed to open stdout file {}: {}", stdout_file, e);
                            std::process::exit(127);
                        }
                    }
                }

                // Redirect stderr if requested
                if let Some(stderr_file) = stderr_path {
                    let path = Path::new(&stderr_file);
                    match open(
                        path,
                        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC,
                        Mode::from_bits_truncate(0o644),
                    ) {
                        Ok(fd) => {
                            let _ = dup2(fd as RawFd, 2); // Redirect stderr
                            let _ = nix::unistd::close(fd as RawFd);
                        }
                        Err(e) => {
                            eprintln!("Failed to open stderr file {}: {}", stderr_file, e);
                            std::process::exit(127);
                        }
                    }
                }

                let cmd =
                    CString::new(command.as_str()).expect("FATAL: command contains null byte");
                let c_args: Vec<CString> = std::iter::once(command.clone())
                    .chain(args)
                    .map(|s| CString::new(s).expect("FATAL: arg contains null byte"))
                    .collect();

                let exec_result = nix::unistd::execv(&cmd, &c_args);
                eprintln!("Failed to exec {}: {:?}", command, exec_result);
                std::process::exit(127);
            }
            Err(e) => Err(format!("Failed to fork: {}", e)),
        }
    }

    pub fn stop(&self, pid: i32, signal: i32) -> Result<(), String> {
        let sig = Signal::try_from(signal).map_err(|e| format!("Invalid signal: {}", e))?;
        kill(Pid::from_raw(pid), sig).map_err(|e| format!("Failed to kill process: {}", e))
    }

    pub fn list(&self) -> Vec<Process> {
        let processes = self
            .processes
            .lock()
            .expect("FATAL: ProcessManager mutex poisoned - this is a critical PID 1 failure");
        processes.values().cloned().collect()
    }

    pub fn update_status(&self, pid: i32, status: ProcessStatus) {
        let mut processes = self
            .processes
            .lock()
            .expect("FATAL: ProcessManager mutex poisoned - this is a critical PID 1 failure");
        if let Some(process) = processes.get_mut(&pid) {
            process.status = status;
        }
    }
}
