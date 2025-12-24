use nix::sys::signal::Signal;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, execv, fork};
use prost::Message;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::net::UnixDatagram;
use std::time::{Duration, Instant};
use tokio::signal::unix::{SignalKind, signal};

mod proto {
    include!(concat!(env!("OUT_DIR"), "/muak.internal.supervisor.rs"));
}

use proto::{Notify, notify::Notification};

const NOTIFY_SOCKET: &str = "/run/granola-notify.sock";
const RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct ServiceDef {
    pub name: String,
    pub binary: String,
    pub args: Vec<String>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Pending,
    Starting,
    Ready,
    Degraded,
    Stopping,
    Failed,
}

struct ServiceState {
    def: ServiceDef,
    pid: Option<i32>,
    status: ServiceStatus,
    socket_path: Option<String>,
    restart_count: u32,
    last_restart: Option<Instant>,
}

pub struct Supervisor {
    services: HashMap<String, ServiceState>,
    notify_socket: UnixDatagram,
    pending_restarts: Vec<(String, Instant)>,
}

impl Supervisor {
    pub fn new(service_defs: Vec<ServiceDef>) -> Result<Self, std::io::Error> {
        let _ = std::fs::remove_file(NOTIFY_SOCKET);

        let notify_socket = UnixDatagram::bind(NOTIFY_SOCKET)?;
        notify_socket.set_nonblocking(true)?;

        kmsg::info!("Supervisor listening on {}", NOTIFY_SOCKET);

        let services = service_defs
            .into_iter()
            .map(|def| {
                let name = def.name.clone();
                let state = ServiceState {
                    def,
                    pid: None,
                    status: ServiceStatus::Pending,
                    socket_path: None,
                    restart_count: 0,
                    last_restart: None,
                };
                (name, state)
            })
            .collect();

        Ok(Self {
            services,
            notify_socket,
            pending_restarts: Vec::new(),
        })
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut sigchld = signal(SignalKind::child())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        kmsg::info!("Signal handlers installed (SIGCHLD, SIGTERM, SIGINT)");

        self.start_ready_services()?;

        let mut interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                _ = sigchld.recv() => {
                    self.reap_children()?;
                }

                _ = sigterm.recv() => {
                    kmsg::warn!("Received SIGTERM, initiating graceful shutdown");
                }

                _ = sigint.recv() => {
                    kmsg::warn!("Received SIGINT, initiating graceful shutdown");
                }

                _ = interval.tick() => {
                    self.poll_notifications()?;
                    self.process_pending_restarts()?;
                }
            }
        }
    }

    fn start_ready_services(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let ready_to_start: Vec<String> = self
            .services
            .iter()
            .filter(|(_, state)| {
                state.status == ServiceStatus::Pending && self.dependencies_ready(&state.def)
            })
            .map(|(name, _)| name.clone())
            .collect();

        for name in ready_to_start {
            if let Err(e) = self.spawn_service(&name) {
                kmsg::error!("Failed to spawn service {}: {}", name, e);
            }
        }

        Ok(())
    }

    fn dependencies_ready(&self, def: &ServiceDef) -> bool {
        def.depends_on.iter().all(|dep| {
            self.services
                .get(dep)
                .map(|s| s.status == ServiceStatus::Ready)
                .unwrap_or(false)
        })
    }

    fn spawn_service(&mut self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.services.get_mut(name).ok_or("Service not found")?;

        if !std::path::Path::new(&state.def.binary).exists() {
            return Err(format!("Binary not found: {}", state.def.binary).into());
        }

        kmsg::info!("Spawning service: {} ({})", name, state.def.binary);

        let binary = CString::new(state.def.binary.clone())?;
        let args: Result<Vec<CString>, _> = std::iter::once(state.def.binary.clone())
            .chain(state.def.args.clone())
            .map(CString::new)
            .collect();
        let args = args?;

        match unsafe { fork() }? {
            ForkResult::Parent { child } => {
                state.pid = Some(child.as_raw());
                state.status = ServiceStatus::Starting;
                kmsg::info!("Spawned {} with PID {}", name, child.as_raw());
            }
            ForkResult::Child => {
                let args_refs: Vec<&std::ffi::CStr> = args.iter().map(|s| s.as_c_str()).collect();
                let _ = execv(&binary, &args_refs);
                eprintln!("execv failed for {}", state.def.binary);
                std::process::exit(1);
            }
        }

        Ok(())
    }

    fn poll_notifications(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = [0u8; 4096];

        while let Ok((len, _)) = self.notify_socket.recv_from(&mut buf) {
            if let Ok(notify) = Notify::decode(&buf[..len])
                && let Err(e) = self.handle_notification(notify)
            {
                kmsg::warn!("Error handling notification: {}", e);
            }
        }

        self.start_ready_services()?;

        Ok(())
    }

    fn handle_notification(&mut self, notify: Notify) -> Result<(), Box<dyn std::error::Error>> {
        let state = match self.services.get_mut(&notify.service_name) {
            Some(s) => s,
            None => {
                kmsg::warn!("Notification from unknown service: {}", notify.service_name);
                return Ok(());
            }
        };

        match notify.notification {
            Some(Notification::Ready(ready)) => {
                kmsg::info!(
                    "Service {} ready (PID {}, socket: {})",
                    notify.service_name,
                    ready.pid,
                    ready.socket_path
                );
                state.status = ServiceStatus::Ready;
                state.socket_path = Some(ready.socket_path);
                state.restart_count = 0;
            }
            Some(Notification::Status(status)) => {
                let health =
                    proto::Health::try_from(status.health).unwrap_or(proto::Health::Healthy);
                kmsg::info!(
                    "Service {} status: {} (health: {:?})",
                    notify.service_name,
                    status.message,
                    health
                );
                if health == proto::Health::Degraded {
                    state.status = ServiceStatus::Degraded;
                }
            }
            Some(Notification::Stopping(stopping)) => {
                kmsg::info!(
                    "Service {} stopping: {}",
                    notify.service_name,
                    stopping.reason
                );
                state.status = ServiceStatus::Stopping;
            }
            Some(Notification::Watchdog(_)) => {
                // Heartbeat received, service is alive
                // Could implement watchdog timeout tracking here
            }
            None => {}
        }

        Ok(())
    }

    fn reap_children(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => {
                    self.handle_child_exit(pid.as_raw(), Some(code), None)?;
                }
                Ok(WaitStatus::Signaled(pid, signal, _)) => {
                    self.handle_child_exit(pid.as_raw(), None, Some(signal))?;
                }
                Ok(WaitStatus::StillAlive) | Err(nix::errno::Errno::ECHILD) => {
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_child_exit(
        &mut self,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<Signal>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let service_name = self
            .services
            .iter()
            .find(|(_, s)| s.pid == Some(pid))
            .map(|(name, _)| name.clone());

        match service_name {
            Some(name) => {
                self.handle_service_exit(&name, pid, exit_code, signal)?;
            }
            None => match (exit_code, signal) {
                (Some(code), _) => {
                    kmsg::info!("Reaped orphan process PID {} (exit code {})", pid, code);
                }
                (_, Some(sig)) => {
                    kmsg::info!(
                        "Reaped orphan process PID {} (killed by signal {:?})",
                        pid,
                        sig
                    );
                }
                _ => {
                    kmsg::info!("Reaped orphan process PID {}", pid);
                }
            },
        }

        Ok(())
    }

    fn handle_service_exit(
        &mut self,
        name: &str,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<Signal>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match (exit_code, signal) {
            (Some(code), _) => {
                kmsg::warn!("Service {} (PID {}) exited with code {}", name, pid, code);
            }
            (_, Some(sig)) => {
                kmsg::warn!("Service {} (PID {}) killed by signal {:?}", name, pid, sig);
            }
            _ => {
                kmsg::warn!("Service {} (PID {}) exited", name, pid);
            }
        }

        let should_restart = self.should_restart(name);

        let state = self
            .services
            .get_mut(name)
            .ok_or_else(|| format!("Service not found: {}", name))?;

        state.pid = None;
        state.socket_path = None;

        if should_restart {
            state.status = ServiceStatus::Pending;
            state.restart_count += 1;
            state.last_restart = Some(Instant::now());

            kmsg::info!(
                "Will restart {} (attempt {}/{}) after {:?}",
                name,
                state.restart_count,
                MAX_RESTART_ATTEMPTS,
                RESTART_DELAY
            );

            self.pending_restarts
                .push((name.to_string(), Instant::now() + RESTART_DELAY));
        } else {
            state.status = ServiceStatus::Failed;
            kmsg::error!("Service {} failed permanently", name);
        }

        Ok(())
    }

    fn should_restart(&self, name: &str) -> bool {
        let state = match self.services.get(name) {
            Some(s) => s,
            None => return false,
        };

        if state.status == ServiceStatus::Stopping {
            return false;
        }

        if let Some(last) = state.last_restart
            && last.elapsed() > RESTART_WINDOW
        {
            return true;
        }

        state.restart_count < MAX_RESTART_ATTEMPTS
    }

    fn process_pending_restarts(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let now = Instant::now();
        let due_restarts: Vec<String> = self
            .pending_restarts
            .iter()
            .filter(|(_, when)| now >= *when)
            .map(|(name, _)| name.clone())
            .collect();

        self.pending_restarts.retain(|(_, when)| now < *when);

        for name in due_restarts {
            let deps_ready = self
                .services
                .get(&name)
                .map(|s| self.dependencies_ready(&s.def))
                .unwrap_or(false);

            if deps_ready {
                if let Err(e) = self.spawn_service(&name) {
                    kmsg::error!("Failed to restart service {}: {}", name, e);
                }
            } else {
                self.pending_restarts
                    .push((name, now + Duration::from_secs(1)));
            }
        }

        Ok(())
    }
}
