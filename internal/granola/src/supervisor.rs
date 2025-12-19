//! Supervisor module for managing child services.
//!
//! Granola (PID 1) acts as a minimal supervisor that:
//! - Spawns child services (networkd, grpcd, vmd) as separate binaries
//! - Receives READY/STATUS/STOPPING notifications via Unix datagram socket
//! - Reaps all zombie processes (as PID 1 must)
//! - Only restarts KNOWN services - orphan processes are reaped but not restarted
//! - Handles service dependencies (e.g., grpcd depends on networkd)

use nix::sys::signal::Signal;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, execv, fork};
use prost::Message;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::net::UnixDatagram;
use std::time::{Duration, Instant};
use tokio::signal::unix::{SignalKind, signal};

#[allow(dead_code)]
mod proto {
    include!(concat!(env!("OUT_DIR"), "/muak.internal.supervisor.rs"));
}

use proto::{Notify, notify::Notification};

const NOTIFY_SOCKET: &str = "/run/granola.sock";
const RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_ATTEMPTS: u32 = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// Service definition - describes how to spawn a service
#[derive(Clone, Debug)]
pub struct ServiceDef {
    pub name: String,
    pub binary: String,
    pub args: Vec<String>,
    pub depends_on: Vec<String>,
}

/// Status of a supervised service
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum ServiceStatus {
    /// Waiting for dependencies to become ready
    Pending,
    /// Spawned, waiting for READY notification
    Starting,
    /// Received READY notification, service is operational
    Ready,
    /// Running but reported unhealthy
    Degraded,
    /// Graceful shutdown in progress
    Stopping,
    /// Exited cleanly
    Stopped,
    /// Crashed or failed to start after max retries
    Failed,
}

/// Runtime state of a supervised service
struct ServiceState {
    def: ServiceDef,
    pid: Option<i32>,
    status: ServiceStatus,
    socket_path: Option<String>,
    restart_count: u32,
    last_restart: Option<Instant>,
}

/// The main supervisor that manages service lifecycles
pub struct Supervisor {
    services: HashMap<String, ServiceState>,
    notify_socket: UnixDatagram,
    /// Services that need to be restarted after a delay
    pending_restarts: Vec<(String, Instant)>,
}

impl Supervisor {
    /// Create a new supervisor with the given service definitions
    pub fn new(service_defs: Vec<ServiceDef>) -> Result<Self, std::io::Error> {
        // Remove old socket if exists
        let _ = std::fs::remove_file(NOTIFY_SOCKET);

        // Ensure /run exists
        let _ = std::fs::create_dir_all("/run");

        // Create notify socket (Unix datagram, non-blocking)
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

    /// Main supervisor loop with proper signal handling
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Set up signal handlers
        let mut sigchld = signal(SignalKind::child())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        kmsg::info!("Signal handlers installed (SIGCHLD, SIGTERM, SIGINT)");

        // Initial startup - spawn services whose dependencies are met
        self.start_ready_services()?;

        // Create interval for periodic tasks (notification polling, pending restarts)
        let mut interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                // SIGCHLD - a child process exited
                _ = sigchld.recv() => {
                    self.reap_children()?;
                }

                // SIGTERM - graceful shutdown requested
                _ = sigterm.recv() => {
                    kmsg::warn!("Received SIGTERM, initiating graceful shutdown");
                    self.shutdown().await?;
                    // As PID 1, we don't actually exit - just log the request
                    // In a real system, this would trigger system shutdown
                }

                // SIGINT - interrupt (usually from terminal)
                _ = sigint.recv() => {
                    kmsg::warn!("Received SIGINT, initiating graceful shutdown");
                    self.shutdown().await?;
                }

                // Periodic tasks
                _ = interval.tick() => {
                    // Poll for notifications from services (non-blocking)
                    self.poll_notifications()?;

                    // Process pending restarts
                    self.process_pending_restarts()?;
                }
            }
        }
    }

    /// Graceful shutdown - send SIGTERM to all services
    async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        kmsg::info!("Shutting down all services...");

        // Send SIGTERM to all running services
        for (name, state) in &mut self.services {
            if let Some(pid) = state.pid {
                if state.status == ServiceStatus::Ready
                    || state.status == ServiceStatus::Starting
                    || state.status == ServiceStatus::Degraded
                {
                    kmsg::info!("Sending SIGTERM to {} (PID {})", name, pid);
                    state.status = ServiceStatus::Stopping;
                    let _ = nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGTERM);
                }
            }
        }

        // Wait for services to exit (with timeout)
        let shutdown_timeout = Duration::from_secs(10);
        let start = Instant::now();

        while start.elapsed() < shutdown_timeout {
            // Reap any exited children
            self.reap_children()?;

            // Check if all services have stopped
            let all_stopped = self
                .services
                .values()
                .all(|s| s.pid.is_none() || s.status == ServiceStatus::Stopped);

            if all_stopped {
                kmsg::info!("All services stopped");
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Force kill any remaining services
        for (name, state) in &mut self.services {
            if let Some(pid) = state.pid {
                kmsg::warn!("Force killing {} (PID {})", name, pid);
                let _ = nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGKILL);
            }
        }

        // Final reap
        self.reap_children()?;

        Ok(())
    }

    /// Start all services whose dependencies are satisfied
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

    /// Check if all dependencies of a service are ready
    fn dependencies_ready(&self, def: &ServiceDef) -> bool {
        def.depends_on.iter().all(|dep| {
            self.services
                .get(dep)
                .map(|s| s.status == ServiceStatus::Ready)
                .unwrap_or(false)
        })
    }

    /// Spawn a service using fork+exec
    fn spawn_service(&mut self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.services.get_mut(name).ok_or("Service not found")?;

        // Check if binary exists
        if !std::path::Path::new(&state.def.binary).exists() {
            return Err(format!("Binary not found: {}", state.def.binary).into());
        }

        kmsg::info!("Spawning service: {} ({})", name, state.def.binary);

        let binary = CString::new(state.def.binary.clone())?;
        let args: Vec<CString> = std::iter::once(state.def.binary.clone())
            .chain(state.def.args.clone())
            .map(|s| CString::new(s).unwrap())
            .collect();

        match unsafe { fork() }? {
            ForkResult::Parent { child } => {
                state.pid = Some(child.as_raw());
                state.status = ServiceStatus::Starting;
                kmsg::info!("Spawned {} with PID {}", name, child.as_raw());
            }
            ForkResult::Child => {
                // In child process - exec the binary
                // execv only returns on error, so we always exit(1) after
                let args_refs: Vec<&std::ffi::CStr> = args.iter().map(|s| s.as_c_str()).collect();
                let _ = execv(&binary, &args_refs);
                // If we get here, exec failed
                eprintln!("execv failed for {}", state.def.binary);
                std::process::exit(1);
            }
        }

        Ok(())
    }

    /// Poll for notifications from services (non-blocking)
    fn poll_notifications(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = [0u8; 4096];

        // Read all available notifications
        while let Ok((len, _)) = self.notify_socket.recv_from(&mut buf) {
            if let Ok(notify) = Notify::decode(&buf[..len]) {
                if let Err(e) = self.handle_notification(notify) {
                    kmsg::warn!("Error handling notification: {}", e);
                }
            }
        }

        // After processing notifications, check if new services can start
        self.start_ready_services()?;

        Ok(())
    }

    /// Handle a notification from a service
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
                // Reset restart count on successful ready
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

    /// Reap all zombie processes.
    ///
    /// As PID 1, granola receives SIGCHLD for:
    /// 1. Its direct children (networkd, grpcd, vmd) - these get restarted
    /// 2. Orphaned processes (e.g., Firecracker VMs whose parent vmd crashed) - these are
    ///    only reaped, NOT restarted
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
                    // No more children to reap
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Handle a child process exit
    fn handle_child_exit(
        &mut self,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<Signal>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check if this PID belongs to a known service
        let service_name = self
            .services
            .iter()
            .find(|(_, s)| s.pid == Some(pid))
            .map(|(name, _)| name.clone());

        match service_name {
            Some(name) => {
                // Known service - handle restart logic
                self.handle_service_exit(&name, pid, exit_code, signal)?;
            }
            None => {
                // Unknown PID - this is an orphaned process (e.g., a VM whose vmd crashed)
                // Just log and reap it, do NOT restart
                match (exit_code, signal) {
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
                }
            }
        }

        Ok(())
    }

    /// Handle a known service exiting
    fn handle_service_exit(
        &mut self,
        name: &str,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<Signal>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.services.get_mut(name).unwrap();

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

        state.pid = None;
        state.socket_path = None;

        // Decide whether to restart
        if self.should_restart(name) {
            let state = self.services.get_mut(name).unwrap();
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

            // Schedule restart after delay
            self.pending_restarts
                .push((name.to_string(), Instant::now() + RESTART_DELAY));
        } else {
            let state = self.services.get_mut(name).unwrap();
            state.status = ServiceStatus::Failed;
            kmsg::error!("Service {} failed permanently", name);
        }

        Ok(())
    }

    /// Check if a service should be restarted
    fn should_restart(&self, name: &str) -> bool {
        let state = match self.services.get(name) {
            Some(s) => s,
            None => return false,
        };

        // Don't restart if we're deliberately stopping
        if state.status == ServiceStatus::Stopping {
            return false;
        }

        // Reset restart count if we're outside the restart window
        if let Some(last) = state.last_restart {
            if last.elapsed() > RESTART_WINDOW {
                return true;
            }
        }

        state.restart_count < MAX_RESTART_ATTEMPTS
    }

    /// Process pending restarts that are due
    fn process_pending_restarts(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let now = Instant::now();
        let due_restarts: Vec<String> = self
            .pending_restarts
            .iter()
            .filter(|(_, when)| now >= *when)
            .map(|(name, _)| name.clone())
            .collect();

        // Remove processed entries
        self.pending_restarts.retain(|(_, when)| now < *when);

        // Restart due services
        for name in due_restarts {
            // Check if dependencies are still ready
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
                // Re-queue for later if dependencies aren't ready
                self.pending_restarts
                    .push((name, now + Duration::from_secs(1)));
            }
        }

        Ok(())
    }

    /// Get the current status of all services
    #[allow(dead_code)]
    pub fn status(&self) -> Vec<(&str, &ServiceStatus)> {
        self.services
            .iter()
            .map(|(name, state)| (name.as_str(), &state.status))
            .collect()
    }
}
