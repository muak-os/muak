use crate::log;
use crate::process::{ProcessManager, ProcessStatus};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use tokio::signal::unix::{signal, Signal, SignalKind};

pub struct SignalHandler {
    sigchld: Signal,
    sigterm: Signal,
    sigint: Signal,
}

impl SignalHandler {
    pub fn new() -> Result<Self, std::io::Error> {
        Ok(Self {
            sigchld: signal(SignalKind::child())?,
            sigterm: signal(SignalKind::terminate())?,
            sigint: signal(SignalKind::interrupt())?,
        })
    }

    pub async fn handle_signals(&mut self, process_manager: &ProcessManager) -> ! {
        loop {
            tokio::select! {
                _ = self.sigchld.recv() => {
                    self.handle_sigchld(process_manager);
                }
                _ = self.sigterm.recv() => {
                    self.handle_sigterm();
                }
                _ = self.sigint.recv() => {
                    self.handle_sigint();
                }
            }
        }
    }

    fn handle_sigchld(&self, process_manager: &ProcessManager) {
        loop {
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, status)) => {
                    process_manager.update_status(pid.as_raw(), ProcessStatus::Exited(status));
                }
                Ok(WaitStatus::Signaled(pid, sig, _)) => {
                    process_manager.update_status(pid.as_raw(), ProcessStatus::Signaled(sig as i32));
                }
                Ok(WaitStatus::StillAlive) => break,
                Err(_) => break,
                _ => {}
            }
        }
    }

    fn handle_sigterm(&self) -> ! {
        log!("granola", "SIGTERM received, exiting");
        std::process::exit(0);
    }

    fn handle_sigint(&self) -> ! {
        log!("granola", "SIGINT received, exiting");
        std::process::exit(0);
    }
}
