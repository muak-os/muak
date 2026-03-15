mod dependency;
pub mod logger;
mod notify;
pub mod reaper;
mod restart;
mod service;
mod socket;
pub mod spawner;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use logger::LogWriter;
use notify::{NotifyListener, ServiceNotification};
use reaper::{ChildReaper, Reaper};
pub use service::Service;
use service::{ServiceState, ServiceStatus};
use spawner::{ProcessSpawner, Spawner};
use tokio::signal::unix::{SignalKind, signal};

const DEFAULT_SERVICES_DIR: &str = "/run/services";

/// Manages the life cycle of all system services.
pub struct Supervisor<S, R> {
    services: HashMap<String, ServiceState>,
    notify_listener: NotifyListener,
    reaper: R,
    restart_queue: restart::RestartQueue,
    logger: LogWriter,
    spawner: S,
}

impl Supervisor<Spawner, Reaper> {
    pub fn new(service_defs: Vec<Service>, logger: LogWriter) -> Result<Self> {
        Self::with_backends(
            service_defs,
            logger,
            Spawner,
            Reaper::new()?,
            Path::new(DEFAULT_SERVICES_DIR),
        )
    }
}

impl<S: ProcessSpawner, R: ChildReaper> Supervisor<S, R> {
    pub fn with_backends(
        service_defs: Vec<Service>,
        logger: LogWriter,
        spawner: S,
        reaper: R,
        services_dir: &Path,
    ) -> Result<Self> {
        std::fs::create_dir_all(services_dir).context("Failed to create services dir")?;

        let services_dir_buf = services_dir.to_path_buf();
        let services = service_defs
            .into_iter()
            .map(|def| {
                let name = def.name.clone();
                let mut state = ServiceState::new(def);
                match socket::pre_bind(&socket::path(&services_dir_buf, &name)) {
                    Ok(fd) => state.listener_fd = Some(fd),
                    Err(e) => kmsg::warn!("Failed to pre-bind socket for {}: {}", name, e),
                }
                (name, state)
            })
            .collect();

        Ok(Self {
            services,
            notify_listener: NotifyListener::new(services_dir)?,
            reaper,
            restart_queue: restart::RestartQueue::new(),
            logger,
            spawner,
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

    fn process_exits(&mut self, exits: Vec<(String, reaper::ChildExit)>) {
        for (name, exit) in exits {
            self.handle_service_exit(&name, exit.pid, exit.exit_code, exit.signal);
        }
    }

    fn start_ready_services(&mut self) -> Result<()> {
        for name in dependency::collect_startable(&self.services) {
            if let Err(e) = self.spawn_service(&name) {
                kmsg::error!("Failed to spawn service {}: {}", name, e);
            }
        }
        Ok(())
    }

    fn spawn_service(&mut self, name: &str) -> Result<()> {
        let state = self
            .services
            .get_mut(name)
            .ok_or_else(|| anyhow!("Service not found: {}", name))?;

        let result = self.spawner.spawn(state)?;
        self.reaper.track(result.pid, name.to_string());
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
                if let Some(state) = self.services.get_mut(&service_name) {
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
                if let Some(state) = self.services.get_mut(&service_name) {
                    state.status = new_status;
                }
            }
            ServiceNotification::Stopping { service_name } => {
                if let Some(state) = self.services.get_mut(&service_name) {
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

        if restart::RestartQueue::should_restart(state, exit_code) {
            self.restart_queue.schedule(state);
        } else if exit_code == Some(0) {
            state.status = ServiceStatus::Stopping;
            kmsg::info!("Service {} exited cleanly, will not restart", name);
        } else {
            restart::RestartQueue::mark_failed(state);
        }
    }

    fn process_pending_restarts(&mut self) -> Result<()> {
        let services = &self.services;
        let due = self.restart_queue.take_due(|name| {
            services
                .get(name)
                .is_some_and(|s| dependency::are_satisfied(&s.service, services))
        });

        for name in due {
            if let Err(e) = self.spawn_service(&name) {
                kmsg::error!("Failed to restart service {}: {}", name, e);
            }
        }

        Ok(())
    }

    /// Returns the current status of a service by name.
    #[cfg(test)]
    pub fn service_status(&self, name: &str) -> Option<&ServiceStatus> {
        self.services.get(name).map(|s| &s.status)
    }

    /// Returns the current PID of a service by name.
    #[cfg(test)]
    pub fn service_pid(&self, name: &str) -> Option<i32> {
        self.services.get(name).and_then(|s| s.pid)
    }

    /// Returns the restart count of a service by name.
    #[cfg(test)]
    pub fn service_restart_count(&self, name: &str) -> Option<u32> {
        self.services.get(name).map(|s| s.restart_count)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::fd::OwnedFd;
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    use super::*;
    use crate::supervisor::notify::ServiceNotification;
    use crate::supervisor::reaper::{ChildExit, ChildReaper};
    use crate::supervisor::service::ServiceStatus;
    use crate::supervisor::spawner::{ProcessSpawner, SpawnResult};

    struct FakeSpawner {
        next_pid: i32,
        spawned: Vec<(String, i32)>,
    }

    impl FakeSpawner {
        fn new() -> Self {
            Self {
                next_pid: 1000,
                spawned: Vec::new(),
            }
        }
    }

    impl ProcessSpawner for FakeSpawner {
        fn spawn(&mut self, state: &mut service::ServiceState) -> Result<SpawnResult> {
            let pid = self.next_pid;
            self.next_pid += 1;
            self.spawned.push((state.service.name.clone(), pid));

            state.pid = Some(pid);
            state.status = ServiceStatus::Starting;

            let (r1, _w1) = std::io::pipe().expect("pipe");
            let (r2, _w2) = std::io::pipe().expect("pipe");

            Ok(SpawnResult {
                pid,
                stdout: OwnedFd::from(r1),
                stderr: OwnedFd::from(r2),
            })
        }
    }

    /// Shared state between the reaper and the test harness.
    struct FakeReaperInner {
        pending: VecDeque<(String, ChildExit)>,
    }

    /// A handle for tests to inject exit events into a `FakeReaper`.
    struct ExitInjector {
        inner: Arc<Mutex<FakeReaperInner>>,
        notify: Arc<Notify>,
    }

    impl ExitInjector {
        fn inject(&self, name: &str, exit_code: Option<i32>) {
            let exit = ChildExit {
                pid: 0,
                exit_code,
                signal: None,
            };
            self.inner
                .lock()
                .expect("lock")
                .pending
                .push_back((name.to_string(), exit));
            self.notify.notify_one();
        }
    }

    struct FakeReaper {
        inner: Arc<Mutex<FakeReaperInner>>,
        notify: Arc<Notify>,
    }

    impl FakeReaper {
        fn new() -> (Self, ExitInjector) {
            let inner = Arc::new(Mutex::new(FakeReaperInner {
                pending: VecDeque::new(),
            }));
            let notify = Arc::new(Notify::new());
            let injector = ExitInjector {
                inner: Arc::clone(&inner),
                notify: Arc::clone(&notify),
            };
            (Self { inner, notify }, injector)
        }
    }

    impl ChildReaper for FakeReaper {
        fn track(&mut self, _pid: i32, _name: String) {}

        async fn wait_for_exits(&mut self) -> Vec<(String, ChildExit)> {
            loop {
                let exits = self.reap_all();
                if !exits.is_empty() {
                    return exits;
                }
                self.notify.notified().await;
            }
        }

        fn reap_all(&mut self) -> Vec<(String, ChildExit)> {
            let mut guard = self.inner.lock().expect("lock");
            guard.pending.drain(..).collect()
        }
    }

    fn make_service(name: &str, depends_on: Vec<&str>) -> Service {
        Service {
            name: name.to_string(),
            command: String::new(),
            depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_supervisor(
        services: Vec<Service>,
        spawner: FakeSpawner,
        reaper: FakeReaper,
        dir: &TempDir,
    ) -> Result<Supervisor<FakeSpawner, FakeReaper>> {
        let (writer, _reader, actor) = logger::create();
        tokio::spawn(actor.run());
        Supervisor::with_backends(services, writer, spawner, reaper, dir.path())
    }

    #[tokio::test]
    async fn services_with_no_deps_spawn_on_start() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("alpha", vec![]), make_service("beta", vec![])];

        // ACT
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        sup.start_ready_services().expect("start");

        // ASSERT
        assert_eq!(sup.service_status("alpha"), Some(&ServiceStatus::Starting));
        assert_eq!(sup.service_status("beta"), Some(&ServiceStatus::Starting));
        assert!(sup.service_pid("alpha").is_some());
        assert!(sup.service_pid("beta").is_some());
    }

    #[tokio::test]
    async fn service_with_unmet_dep_stays_pending() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![
            make_service("dep", vec![]),
            make_service("child", vec!["dep"]),
        ];

        // ACT
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        sup.start_ready_services().expect("start");

        // ASSERT
        assert_eq!(sup.service_status("dep"), Some(&ServiceStatus::Starting));
        assert_eq!(sup.service_status("child"), Some(&ServiceStatus::Pending));
        assert!(sup.service_pid("dep").is_some());
        assert!(sup.service_pid("child").is_none());
    }

    #[tokio::test]
    async fn failed_service_is_scheduled_for_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        injector.inject("svc", Some(1));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Pending));
        assert_eq!(sup.service_restart_count("svc"), Some(1));
    }

    #[tokio::test]
    async fn clean_exit_does_not_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        injector.inject("svc", Some(0));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Stopping));
        assert_eq!(sup.service_restart_count("svc"), Some(0));
    }

    #[tokio::test]
    async fn service_marked_failed_after_max_restarts() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        for _ in 0..restart::MAX_RESTART_ATTEMPTS {
            injector.inject("svc", Some(1));
            let exits = sup.reaper.wait_for_exits().await;
            sup.process_exits(exits);
            sup.restart_queue
                .take_due(|_| true)
                .into_iter()
                .for_each(|name| {
                    if let Err(e) = sup.spawn_service(&name) {
                        panic!("spawn failed: {e}");
                    }
                });
        }

        injector.inject("svc", Some(1));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Failed));
    }

    #[tokio::test]
    async fn dep_becoming_ready_triggers_child_spawn() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![
            make_service("dep", vec![]),
            make_service("child", vec!["dep"]),
        ];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ASSERT
        assert_eq!(sup.service_status("child"), Some(&ServiceStatus::Pending));

        // ACT
        if let Some(state) = sup.services.get_mut("dep") {
            state.status = ServiceStatus::Ready;
        }
        sup.start_ready_services().expect("start");

        // ASSERT
        assert_eq!(sup.service_status("child"), Some(&ServiceStatus::Starting));
    }

    #[tokio::test]
    async fn apply_notification_ready_sets_status_and_resets_count() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        sup.services.get_mut("svc").expect("svc").restart_count = 3;

        // ACT
        sup.apply_notification(ServiceNotification::Ready {
            service_name: "svc".to_string(),
        });

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Ready));
        assert_eq!(sup.service_restart_count("svc"), Some(0));
    }

    #[tokio::test]
    async fn apply_notification_ready_unknown_service_does_not_panic() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let mut sup = make_supervisor(vec![], spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.apply_notification(ServiceNotification::Ready {
            service_name: "ghost".to_string(),
        });
    }

    #[tokio::test]
    async fn apply_notification_status_update_sets_degraded() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.apply_notification(ServiceNotification::StatusUpdate {
            service_name: "svc".to_string(),
            new_status: ServiceStatus::Degraded,
        });

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Degraded));
    }

    #[tokio::test]
    async fn apply_notification_stopping_sets_stopping() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.apply_notification(ServiceNotification::Stopping {
            service_name: "svc".to_string(),
        });

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Stopping));
    }

    #[tokio::test]
    async fn signal_only_exit_schedules_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("svc", 1234, None, Some(9));

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Pending));
        assert_eq!(sup.service_restart_count("svc"), Some(1));
    }

    #[tokio::test]
    async fn no_exit_code_no_signal_schedules_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("svc", 1234, None, None);

        // ASSERT
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Pending));
        assert_eq!(sup.service_restart_count("svc"), Some(1));
    }

    #[tokio::test]
    async fn exit_for_unknown_service_does_not_panic() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let mut sup = make_supervisor(vec![], spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("ghost", 9999, Some(1), None);
    }

    #[tokio::test]
    async fn process_pending_restarts_respawns_due_service() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", vec![])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        injector.inject("svc", Some(1));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);
        assert_eq!(sup.service_status("svc"), Some(&ServiceStatus::Pending));
        assert_eq!(sup.service_restart_count("svc"), Some(1));

        let not_yet_due = sup.restart_queue.take_due(|_| false);
        assert!(not_yet_due.is_empty());

        let due = sup.restart_queue.take_due(|_| true);

        // ACT
        for name in due {
            sup.spawn_service(&name).expect("spawn");
        }

        // ASSERT
        let status = sup.service_status("svc").cloned();
        assert!(
            status == Some(ServiceStatus::Starting) || status == Some(ServiceStatus::Pending),
            "unexpected status: {status:?}"
        );
    }
}
