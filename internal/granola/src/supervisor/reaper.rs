use std::collections::HashMap;
use std::future::Future;

use rustix::process::{Pid, WaitOptions, WaitStatus, waitpid};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// Exit information for a reaped child process.
pub struct ChildExit {
    pub pid: i32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

/// Abstraction over child-process reaping.
pub trait ChildReaper {
    /// Registers a PID as belonging to a named supervised service.
    fn track(&mut self, pid: i32, name: &'static str);

    /// Waits for the next batch of child exits (may block).
    fn wait_for_exits(&mut self) -> impl Future<Output = Vec<(&'static str, ChildExit)>> + Send;

    /// Non-blocking drain of all already-terminated children.
    fn reap_all(&mut self) -> Vec<(&'static str, ChildExit)>;
}

/// PID 1 child reaper using SIGCHLD and `waitpid(-1, WNOHANG)`.
pub struct Reaper {
    sigchld: Signal,
    known_pids: HashMap<i32, &'static str>,
}

impl Reaper {
    pub fn new() -> anyhow::Result<Self> {
        let sigchld = signal(SignalKind::child())?;
        Ok(Self {
            sigchld,
            known_pids: HashMap::new(),
        })
    }
}

impl ChildReaper for Reaper {
    fn track(&mut self, pid: i32, name: &'static str) {
        self.known_pids.insert(pid, name);
    }

    async fn wait_for_exits(&mut self) -> Vec<(&'static str, ChildExit)> {
        self.sigchld.recv().await;
        self.reap_all()
    }

    fn reap_all(&mut self) -> Vec<(&'static str, ChildExit)> {
        let mut service_exits = Vec::new();
        let Some(any_child) = Pid::from_raw(-1) else {
            return service_exits;
        };

        while let Ok(Some((child_pid, status))) = waitpid(Some(any_child), WaitOptions::NOHANG) {
            let raw_pid = child_pid.as_raw_nonzero().get();
            let Some(exit) = decode_wait_status(raw_pid, &status) else {
                continue;
            };
            self.dispatch_exit(&mut service_exits, raw_pid, exit);
        }

        service_exits
    }
}

impl Reaper {
    fn dispatch_exit(
        &mut self,
        service_exits: &mut Vec<(&'static str, ChildExit)>,
        pid: i32,
        exit: ChildExit,
    ) {
        if let Some(name) = self.known_pids.remove(&pid) {
            service_exits.push((name, exit));
        } else {
            kmsg::debug!(
                "Reaped orphan process PID {} (exit_code={:?}, signal={:?})",
                pid,
                exit.exit_code,
                exit.signal
            );
        }
    }
}

fn decode_wait_status(pid: i32, status: &WaitStatus) -> Option<ChildExit> {
    if status.exited() {
        Some(ChildExit {
            pid,
            exit_code: status.exit_status(),
            signal: None,
        })
    } else if status.signaled() {
        Some(ChildExit {
            pid,
            exit_code: None,
            signal: status.terminating_signal(),
        })
    } else {
        None
    }
}
