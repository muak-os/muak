use anyhow::{Context, Result, anyhow};
use prost::Message;
use std::collections::HashMap;
use std::os::unix::net::UnixDatagram;
use std::os::unix::process::ExitStatusExt;
use std::time::{Duration, Instant};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

#[allow(clippy::excessive_nesting)]
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
    exit_tx: mpsc::UnboundedSender<(String, std::process::ExitStatus)>,
    exit_rx: mpsc::UnboundedReceiver<(String, std::process::ExitStatus)>,
}

impl Supervisor {
    pub fn new(service_defs: Vec<ServiceDef>) -> Result<Self> {
        let _ = std::fs::remove_file(NOTIFY_SOCKET);

        let notify_socket =
            UnixDatagram::bind(NOTIFY_SOCKET).context("Failed to bind notify socket")?;
        notify_socket
            .set_nonblocking(true)
            .context("Failed to set notify socket to non-blocking")?;

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

        let (exit_tx, exit_rx) = mpsc::unbounded_channel();

        Ok(Self {
            services,
            notify_socket,
            pending_restarts: Vec::new(),
            exit_tx,
            exit_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        kmsg::info!("Signal handlers installed (SIGTERM, SIGINT)");

        self.start_ready_services().await?;

        let mut interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                recv_result = self.exit_rx.recv() => {
                    if let Some((name, status)) = recv_result {
                        self.handle_exit_event(&name, status)?;
                    }
                }

                _ = sigterm.recv() => {
                    kmsg::warn!("Received SIGTERM, initiating graceful shutdown");
                }

                _ = sigint.recv() => {
                    kmsg::warn!("Received SIGINT, initiating graceful shutdown");
                }

                _ = interval.tick() => {
                    self.poll_notifications().await?;
                    self.process_pending_restarts().await?;
                }
            }
        }
    }

    async fn start_ready_services(&mut self) -> Result<()> {
        let ready_to_start: Vec<String> = self
            .services
            .iter()
            .filter(|(_, state)| {
                state.status == ServiceStatus::Pending && self.dependencies_ready(&state.def)
            })
            .map(|(name, _)| name.clone())
            .collect();

        for name in ready_to_start {
            let result = self.spawn_service(&name).await;
            if let Err(e) = result {
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

    async fn spawn_service(&mut self, name: &str) -> Result<()> {
        let state = self
            .services
            .get_mut(name)
            .ok_or_else(|| anyhow!("Service not found: {}", name))?;

        if !std::path::Path::new(&state.def.binary).exists() {
            return Err(anyhow!("Binary not found: {}", state.def.binary));
        }

        kmsg::info!("Spawning service: {} ({})", name, state.def.binary);

        let mut command = tokio::process::Command::new(&state.def.binary);
        command.args(&state.def.args);
        let mut child = command.spawn().context("Failed to spawn service")?;

        let pid = child
            .id()
            .ok_or_else(|| anyhow!("Failed to get child PID"))?;
        state.pid = Some(pid as i32);
        state.status = ServiceStatus::Starting;
        kmsg::info!("Spawned {} with PID {}", name, pid);

        let name_clone = name.to_string();
        let exit_tx = self.exit_tx.clone();
        tokio::spawn(async move {
            if let Ok(status) = child.wait().await {
                let _ = exit_tx.send((name_clone, status));
            }
        });

        Ok(())
    }

    async fn poll_notifications(&mut self) -> Result<()> {
        let mut buf = [0u8; 4096];

        while let Ok((len, _)) = self.notify_socket.recv_from(&mut buf) {
            let Ok(notify) = Notify::decode(&buf[..len]) else {
                continue;
            };
            if let Err(e) = self.handle_notification(notify) {
                kmsg::warn!("Error handling notification: {}", e);
            }
        }

        self.start_ready_services().await?;

        Ok(())
    }

    fn handle_exit_event(&mut self, name: &str, status: std::process::ExitStatus) -> Result<()> {
        let pid = self.services.get(name).and_then(|s| s.pid).unwrap_or(0);
        let exit_code = status.code();
        let signal = status.signal();
        self.handle_service_exit(name, pid, exit_code, signal)
    }

    fn handle_notification(&mut self, notify: Notify) -> Result<()> {
        let Some(state) = self.services.get_mut(&notify.service_name) else {
            kmsg::warn!("Notification from unknown service: {}", notify.service_name);
            return Ok(());
        };

        let Some(notification) = notify.notification else {
            return Ok(());
        };

        match notification {
            Notification::Ready(ready) => {
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
            Notification::Status(status) => {
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
            Notification::Stopping(stopping) => {
                kmsg::info!(
                    "Service {} stopping: {}",
                    notify.service_name,
                    stopping.reason
                );
                state.status = ServiceStatus::Stopping;
            }
            Notification::Watchdog(_) => {}
        }

        Ok(())
    }

    fn handle_service_exit(
        &mut self,
        name: &str,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<i32>,
    ) -> Result<()> {
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
            .ok_or_else(|| anyhow!("Service not found: {}", name))?;

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

    async fn process_pending_restarts(&mut self) -> Result<()> {
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

            if !deps_ready {
                self.pending_restarts
                    .push((name, now + Duration::from_secs(1)));
                continue;
            }

            let result = self.spawn_service(&name).await;
            if let Err(e) = result {
                kmsg::error!("Failed to restart service {}: {}", name, e);
            }
        }

        Ok(())
    }
}
