mod dependency;
pub mod logger;
mod notify;
mod reaper;
mod restart;
mod service;
mod socket;
mod spawner;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use logger::LogWriter;
use notify::{NotifyListener, ServiceNotification};
use reaper::Reaper;
use restart::RestartQueue;
pub use service::ServiceDef;
use service::{ServiceState, ServiceStatus};
use tokio::signal::unix::{SignalKind, signal};

const SERVICES_DIR: &str = "/run/services";

/// Manages the life cycle of all system services.
pub struct Supervisor {
    services: HashMap<&'static str, ServiceState>,
    notify_listener: NotifyListener,
    reaper: Reaper,
    restart_queue: RestartQueue,
    logger: LogWriter,
}

impl Supervisor {
    pub fn new(service_defs: Vec<ServiceDef>, logger: LogWriter) -> Result<Self> {
        std::fs::create_dir_all(SERVICES_DIR).context("Failed to create services dir")?;

        let services = service_defs
            .into_iter()
            .map(|def| {
                let name = def.name;
                let mut state = ServiceState::new(def);
                match socket::pre_bind(&socket::path(name)) {
                    Ok(fd) => state.listener_fd = Some(fd),
                    Err(e) => kmsg::warn!("Failed to pre-bind socket for {}: {}", name, e),
                }
                (name, state)
            })
            .collect();

        Ok(Self {
            services,
            notify_listener: NotifyListener::new()?,
            reaper: Reaper::new()?,
            restart_queue: RestartQueue::new(),
            logger,
        })
    }

    /// Main event loop. Runs until the system shuts down.
    pub async fn run(&mut self) -> Result<()> {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        self.start_ready_services()?;

        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut sweep = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                exits = self.reaper.wait_for_exits() => {
                    self.process_exits(exits);
                }

                _ = sigterm.recv() => {
                    kmsg::warn!("Received SIGTERM");
                }

                _ = sigint.recv() => {
                    kmsg::warn!("Received SIGINT");
                }

                _ = tick.tick() => {
                    self.process_notifications()?;
                    self.process_pending_restarts()?;
                }

                _ = sweep.tick() => {
                    let exits = self.reaper.reap_all();
                    self.process_exits(exits);
                }
            }
        }
    }

    fn process_exits(&mut self, exits: Vec<(&'static str, reaper::ChildExit)>) {
        for (name, exit) in exits {
            self.handle_service_exit(name, exit.pid, exit.exit_code, exit.signal);
        }
    }

    fn start_ready_services(&mut self) -> Result<()> {
        for name in dependency::collect_startable(&self.services) {
            if let Err(e) = self.spawn_service(name) {
                kmsg::error!("Failed to spawn service {}: {}", name, e);
            }
        }
        Ok(())
    }

    fn spawn_service(&mut self, name: &'static str) -> Result<()> {
        let state = self
            .services
            .get_mut(name)
            .ok_or_else(|| anyhow!("Service not found: {}", name))?;

        let result = spawner::spawn(state)?;
        self.reaper.track(result.pid, name);
        logger::capture(name, result.stdout, result.stderr, &self.logger);

        Ok(())
    }

    fn process_notifications(&mut self) -> Result<()> {
        for notification in self.notify_listener.poll() {
            self.apply_notification(notification);
        }

        self.start_ready_services()
    }

    fn apply_notification(&mut self, notification: ServiceNotification) {
        match notification {
            ServiceNotification::Ready { service_name } => {
                if let Some(state) = self.services.get_mut(service_name.as_str()) {
                    state.status = ServiceStatus::Ready;
                    state.restart_count = 0;
                } else {
                    kmsg::warn!("Notification from unknown service: {}", service_name);
                }
            }
            ServiceNotification::StatusUpdate {
                service_name,
                new_status,
            } => {
                if let Some(state) = self.services.get_mut(service_name.as_str()) {
                    state.status = new_status;
                }
            }
            ServiceNotification::Stopping { service_name } => {
                if let Some(state) = self.services.get_mut(service_name.as_str()) {
                    state.status = ServiceStatus::Stopping;
                }
            }
        }
    }

    fn handle_service_exit(
        &mut self,
        name: &str,
        pid: i32,
        exit_code: Option<i32>,
        signal: Option<i32>,
    ) {
        match (exit_code, signal) {
            (Some(code), _) => {
                kmsg::warn!("Service {} (PID {}) exited with code {}", name, pid, code);
            }
            (_, Some(sig)) => {
                kmsg::warn!("Service {} (PID {}) killed by signal {}", name, pid, sig);
            }
            _ => {
                kmsg::warn!("Service {} (PID {}) exited", name, pid);
            }
        }

        let Some(state) = self.services.get_mut(name) else {
            return;
        };

        state.pid = None;

        if RestartQueue::should_restart(state, exit_code) {
            self.restart_queue.schedule(state);
        } else if exit_code == Some(0) {
            // Clean exit, not a failure.
            state.status = ServiceStatus::Stopping;
            kmsg::info!("Service {} exited cleanly, will not restart", name);
        } else {
            RestartQueue::mark_failed(state);
        }
    }

    fn process_pending_restarts(&mut self) -> Result<()> {
        let services = &self.services;
        let due = self.restart_queue.take_due(|name| {
            services
                .get(name)
                .is_some_and(|s| dependency::are_satisfied(&s.def, services))
        });

        for name in due {
            if let Err(e) = self.spawn_service(name) {
                kmsg::error!("Failed to restart service {}: {}", name, e);
            }
        }

        Ok(())
    }
}
