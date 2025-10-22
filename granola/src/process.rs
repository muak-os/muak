use nix::sys::signal::{kill, Signal};
use nix::unistd::{fork, ForkResult, Pid};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Process {
    pub pid: i32,
    pub command: String,
    pub args: Vec<String>,
    pub status: ProcessStatus,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Running,
    Exited(i32),
    Signaled(i32),
}

impl ToString for ProcessStatus {
    fn to_string(&self) -> String {
        match self {
            ProcessStatus::Running => "running".to_string(),
            ProcessStatus::Exited(code) => format!("exited({})", code),
            ProcessStatus::Signaled(sig) => format!("signaled({})", sig),
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

    pub fn spawn(
        &self,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<i32, String> {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                let pid = child.as_raw();
                let process = Process {
                    pid,
                    command: command.clone(),
                    args: args.clone(),
                    status: ProcessStatus::Running,
                    started_at: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                };

                let mut processes = self.processes.lock().unwrap();
                processes.insert(pid, process);

                Ok(pid)
            }
            Ok(ForkResult::Child) => {
                let cmd = CString::new(command.as_str()).unwrap();
                let c_args: Vec<CString> = std::iter::once(command.clone())
                    .chain(args)
                    .map(|s| CString::new(s).unwrap())
                    .collect();

                for (key, value) in env {
                    std::env::set_var(key, value);
                }

                nix::unistd::execv(&cmd, &c_args).ok();
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
        let processes = self.processes.lock().unwrap();
        processes.values().cloned().collect()
    }

    pub fn update_status(&self, pid: i32, status: ProcessStatus) {
        let mut processes = self.processes.lock().unwrap();
        if let Some(process) = processes.get_mut(&pid) {
            process.status = status;
        }
    }


}
