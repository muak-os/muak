mod dependency;
pub mod logger;
mod notify;
pub mod reaper;
mod restart;
pub(crate) mod service;
mod socket;
pub mod spawner;
pub mod status;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use logger::LogWriter;
use notify::NotifyListener;
use reaper::{Reap, Reaper};
use service::{Service, ServiceState, ServiceStatus};
use spawner::{Spawn, Spawner};
use status::Snapshot;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

const DEFAULT_SERVICES_DIR: &str = "/run/services";

/// Manages the life cycle of all system services.
pub struct Supervisor<S, R> {
    services: HashMap<String, ServiceState>,
    notify_listener: NotifyListener,
    reaper: R,
    restart_queue: restart::RestartQueue,
    logger: LogWriter,
    status: watch::Sender<Snapshot>,
    spawner: S,
}

impl Supervisor<Spawner, Reaper> {
    pub fn new(
        service_defs: Vec<Service>,
        logger: LogWriter,
        status: watch::Sender<Snapshot>,
    ) -> Result<Self> {
        Self::with_backends(
            service_defs,
            logger,
            status,
            Spawner,
            Reaper::new()?,
            Path::new(DEFAULT_SERVICES_DIR),
        )
    }
}

impl<S: Spawn, R: Reap> Supervisor<S, R> {
    pub fn with_backends(
        service_defs: Vec<Service>,
        logger: LogWriter,
        status: watch::Sender<Snapshot>,
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
            status,
            spawner,
        })
    }

    /// Main event loop. Runs until the system shuts down.
    pub async fn run(&mut self) -> Result<()> {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        self.start_ready_services();
        status::publish(&self.services, &self.status);

        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut sweep = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                biased;

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
                    self.process_notifications();
                    self.process_pending_restarts();
                }

                _ = sweep.tick() => {
                    let exits = self.reaper.reap_all();
                    if !exits.is_empty() { self.process_exits(exits); }
                }
            }
        }
    }

    fn process_exits(&mut self, exits: Vec<(String, reaper::ChildExit)>) {
        for (name, exit) in exits {
            self.handle_service_exit(&name, exit.pid, exit.exit_code, exit.signal);
        }
        status::publish(&self.services, &self.status);
    }

    /// Records a service exit and schedules a restart or failure as needed.
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
        } else if exit_code == Some(0_i32) {
            state.status = ServiceStatus::Stopping;
            kmsg::info!("Service {} exited cleanly, will not restart", name);
        } else {
            restart::RestartQueue::mark_failed(state);
        }
    }

    fn start_ready_services(&mut self) {
        let mut startable = dependency::collect_startable(&self.services);
        startable.retain(|name| !self.restart_queue.is_queued(name));
        for name in startable {
            self.spawn_or_log(&name);
        }
    }

    // Spawns a service, scheduling a delayed retry when the spawn fails.
    fn spawn_or_log(&mut self, name: &str) {
        let Err(error) = self.spawn_service(name) else {
            return;
        };
        let message = error.to_string();
        match self.services.get_mut(name) {
            Some(state) => self.restart_queue.record_spawn_failure(state, &message),
            None => kmsg::error!("Failed to spawn unknown service {name}: {message}"),
        }
        status::publish(&self.services, &self.status);
    }

    fn spawn_service(&mut self, name: &str) -> Result<()> {
        let state = self
            .services
            .get_mut(name)
            .ok_or_else(|| anyhow!("Service not found: {name}"))?;

        let result = self.spawner.spawn(state)?;
        self.reaper.track(result.pid, name.to_owned());
        logger::sources::capture(name, result.stdout, result.stderr, &self.logger);
        status::publish(&self.services, &self.status);

        Ok(())
    }

    fn process_notifications(&mut self) {
        let notifications = self.notify_listener.poll();
        let received = !notifications.is_empty();
        for notification in notifications {
            notify::apply(self, notification);
        }

        self.start_ready_services();

        if received {
            status::publish(&self.services, &self.status);
        }
    }

    fn process_pending_restarts(&mut self) {
        let services = &self.services;
        let due = self.restart_queue.take_due(|name| {
            services
                .get(name)
                .is_some_and(|state| dependency::are_satisfied(&state.service, services))
        });

        for name in due {
            self.spawn_or_log(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use alloc::sync::Arc;
    use std::os::fd::OwnedFd;
    use std::sync::Mutex;

    use anyhow::{Result, bail};
    use granola::runtime::notify::Health;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    use super::*;
    use crate::supervisor::notify::ServiceNotification;
    use crate::supervisor::reaper::{ChildExit, Reap};
    use crate::supervisor::service::ServiceStatus;
    use crate::supervisor::spawner::{Spawn, SpawnResult};
    use crate::supervisor::status::View;

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

    impl Spawn for FakeSpawner {
        fn spawn(&mut self, state: &mut service::ServiceState) -> Result<SpawnResult> {
            let pid = self.next_pid;
            self.next_pid = self.next_pid.saturating_add(1);
            self.spawned.push((state.service.name.clone(), pid));

            state.pid = Some(pid);
            state.status = ServiceStatus::Starting;

            let (r1, _w1) = std::io::pipe()?;
            let (r2, _w2) = std::io::pipe()?;

            Ok(SpawnResult {
                pid,
                stdout: OwnedFd::from(r1),
                stderr: OwnedFd::from(r2),
            })
        }
    }

    struct FailingSpawner {
        attempts: u32,
    }

    impl FailingSpawner {
        fn new() -> Self {
            Self { attempts: 0 }
        }
    }

    impl Spawn for FailingSpawner {
        fn spawn(&mut self, _state: &mut service::ServiceState) -> Result<SpawnResult> {
            self.attempts = self.attempts.saturating_add(1);
            bail!("Binary not found: /nonexistent");
        }
    }

    impl<S, R> Supervisor<S, R> {
        fn state(&self, name: &str) -> Option<&ServiceState> {
            self.services.get(name)
        }
    }

    struct FakeReaperInner {
        pending: VecDeque<(String, ChildExit)>,
    }

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
                .push_back((name.to_owned(), exit));
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

    impl Reap for FakeReaper {
        fn track(&mut self, _pid: i32, _name: String) {}

        async fn wait_for_exits(&mut self) -> Vec<(String, ChildExit)> {
            wait_until_exit(self).await
        }

        fn reap_all(&mut self) -> Vec<(String, ChildExit)> {
            let mut guard = self.inner.lock().expect("lock");
            guard.pending.drain(..).collect()
        }
    }

    fn respawn_all(sup: &mut Supervisor<FakeSpawner, FakeReaper>, due: Vec<String>) {
        for name in due {
            sup.spawn_service(&name).expect("spawn should succeed");
        }
    }

    async fn wait_until_exit(reaper: &mut FakeReaper) -> Vec<(String, ChildExit)> {
        let mut exits = reaper.reap_all();
        while exits.is_empty() {
            reaper.notify.notified().await;
            exits = reaper.reap_all();
        }
        exits
    }

    fn make_service(name: &str, depends_on: &[&str]) -> Service {
        Service {
            name: name.to_owned(),
            command: String::new(),
            depends_on: depends_on.iter().copied().map(str::to_owned).collect(),
        }
    }

    fn make_supervisor<S: Spawn>(
        services: Vec<Service>,
        spawner: S,
        reaper: FakeReaper,
        dir: &TempDir,
    ) -> Result<Supervisor<S, FakeReaper>> {
        make_observed_supervisor(services, spawner, reaper, dir).map(|(sup, _rx)| sup)
    }

    fn make_observed_supervisor<S: Spawn>(
        services: Vec<Service>,
        spawner: S,
        reaper: FakeReaper,
        dir: &TempDir,
    ) -> Result<(Supervisor<S, FakeReaper>, watch::Receiver<Snapshot>)> {
        let (writer, _reader, actor) = logger::create();
        tokio::spawn(actor.run());
        let (status, status_rx) = status::channel();
        let sup = Supervisor::with_backends(services, writer, status, spawner, reaper, dir.path())?;
        Ok((sup, status_rx))
    }

    fn fail_spawns(sup: &mut Supervisor<FailingSpawner, FakeReaper>, attempts: u32) {
        for _ in 0..attempts {
            sup.spawn_or_log("svc");
        }
    }

    async fn exhaust_restart_attempts(
        sup: &mut Supervisor<FakeSpawner, FakeReaper>,
        injector: &ExitInjector,
    ) {
        for _ in 0..restart::MAX_RESTART_ATTEMPTS {
            injector.inject("svc", Some(1_i32));
            let exits = sup.reaper.wait_for_exits().await;
            sup.process_exits(exits);
            let due = sup.restart_queue.take_due(|_| true);
            respawn_all(sup, due);
        }
    }

    #[tokio::test]
    async fn services_with_no_deps_spawn_on_start() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("alpha", &[]), make_service("beta", &[])];

        // ACT
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        sup.start_ready_services();

        // ASSERT
        assert_eq!(
            sup.state("alpha").map(|state| &state.status),
            Some(&ServiceStatus::Starting)
        );
        assert_eq!(
            sup.state("beta").map(|state| &state.status),
            Some(&ServiceStatus::Starting)
        );
        assert!(sup.state("alpha").is_some_and(|state| state.pid.is_some()));
        assert!(sup.state("beta").is_some_and(|state| state.pid.is_some()));
    }

    #[tokio::test]
    async fn service_with_unmet_dep_stays_pending() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("dep", &[]), make_service("child", &["dep"])];

        // ACT
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        sup.start_ready_services();

        // ASSERT
        assert_eq!(
            sup.state("dep").map(|state| &state.status),
            Some(&ServiceStatus::Starting)
        );
        assert_eq!(
            sup.state("child").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert!(sup.state("dep").is_some_and(|state| state.pid.is_some()));
        assert!(sup.state("child").is_some_and(|state| state.pid.is_none()));
    }

    #[tokio::test]
    async fn failed_service_is_scheduled_for_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        injector.inject("svc", Some(1_i32));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(1));
    }

    #[tokio::test]
    async fn clean_exit_does_not_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        injector.inject("svc", Some(0_i32));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Stopping)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(0));
    }

    #[tokio::test]
    async fn service_marked_failed_after_max_restarts() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        exhaust_restart_attempts(&mut sup, &injector).await;
        injector.inject("svc", Some(1_i32));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Failed)
        );
    }

    #[tokio::test]
    async fn spawn_failure_queues_retry_instead_of_respawning() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let services = vec![make_service("svc", &[])];
        let mut sup =
            make_supervisor(services, FailingSpawner::new(), reaper, &dir).expect("supervisor");

        // ACT
        sup.start_ready_services();
        sup.start_ready_services();

        // ASSERT
        assert_eq!(sup.spawner.attempts, 1);
        assert!(sup.restart_queue.is_queued("svc"));
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(1));
    }

    #[tokio::test]
    async fn spawn_failure_fails_service_after_max_attempts() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let services = vec![make_service("svc", &[])];
        let mut sup =
            make_supervisor(services, FailingSpawner::new(), reaper, &dir).expect("supervisor");

        // ACT
        fail_spawns(&mut sup, 1);
        // The queued retry only becomes due once the restart delay elapses.
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        sup.process_pending_restarts();
        fail_spawns(&mut sup, restart::MAX_RESTART_ATTEMPTS - 2);
        sup.start_ready_services();
        let due = sup.restart_queue.take_due(|_| true);

        // ASSERT
        assert_eq!(sup.spawner.attempts, restart::MAX_RESTART_ATTEMPTS);
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Failed)
        );
        assert!(!sup.restart_queue.is_queued("svc"));
        assert!(due.is_empty(), "a failed service must not stay queued");
    }

    #[tokio::test]
    async fn dep_becoming_ready_triggers_child_spawn() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("dep", &[]), make_service("child", &["dep"])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ASSERT
        assert_eq!(
            sup.state("child").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );

        // ACT
        sup.services.get_mut("dep").expect("dep exists").status = ServiceStatus::Ready;
        sup.start_ready_services();

        // ASSERT
        assert_eq!(
            sup.state("child").map(|state| &state.status),
            Some(&ServiceStatus::Starting)
        );
    }

    #[tokio::test]
    async fn apply_notification_ready_sets_status_and_resets_count() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        sup.services.get_mut("svc").expect("svc").restart_count = 3;

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::Ready {
                service_name: "svc".to_owned(),
            },
        );

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Ready)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(0));
    }

    #[tokio::test]
    async fn apply_notification_ready_unknown_service_does_not_panic() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let mut sup = make_supervisor(vec![], spawner, reaper, &dir).expect("supervisor");

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::Ready {
                service_name: "ghost".to_owned(),
            },
        );
    }

    #[tokio::test]
    async fn apply_notification_status_update_sets_degraded() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::StatusUpdate {
                service_name: "svc".to_owned(),
                status_text: "Sync failed".to_owned(),
                health: Health::Degraded,
            },
        );

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Degraded)
        );
    }

    #[tokio::test]
    async fn apply_healthy_status_update_preserves_lifecycle_status() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::StatusUpdate {
                service_name: "svc".to_owned(),
                status_text: "Synchronized".to_owned(),
                health: Health::Healthy,
            },
        );

        // ASSERT
        let state = sup.services.get("svc").expect("svc exists");
        assert_eq!(state.status, ServiceStatus::Pending);
        assert_eq!(
            state.last_status,
            Some(("Synchronized".to_owned(), Health::Healthy))
        );
    }

    #[tokio::test]
    async fn apply_identical_status_update_deduplicates_last_status() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        let notification = || ServiceNotification::StatusUpdate {
            service_name: "svc".to_owned(),
            status_text: "Synchronized".to_owned(),
            health: Health::Healthy,
        };

        // ACT
        notify::apply(&mut sup, notification());
        let first = sup
            .services
            .get("svc")
            .expect("svc exists")
            .last_status
            .clone();
        notify::apply(&mut sup, notification());

        // ASSERT
        let state = sup.services.get("svc").expect("svc exists");
        assert_eq!(first, Some(("Synchronized".to_owned(), Health::Healthy)));
        assert_eq!(state.last_status, first);
    }

    #[tokio::test]
    async fn apply_healthy_status_after_degraded_restores_ready() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        notify::apply(
            &mut sup,
            ServiceNotification::StatusUpdate {
                service_name: "svc".to_owned(),
                status_text: "Sync failed".to_owned(),
                health: Health::Degraded,
            },
        );

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::StatusUpdate {
                service_name: "svc".to_owned(),
                status_text: "Synchronized".to_owned(),
                health: Health::Healthy,
            },
        );

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Ready)
        );
    }

    #[tokio::test]
    async fn apply_notification_stopping_sets_stopping() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        notify::apply(
            &mut sup,
            ServiceNotification::Stopping {
                service_name: "svc".to_owned(),
            },
        );

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Stopping)
        );
    }

    #[tokio::test]
    async fn signal_only_exit_schedules_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("svc", 1234, None, Some(9_i32));

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(1));
    }

    #[tokio::test]
    async fn no_exit_code_no_signal_schedules_restart() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("svc", 1234, None, None);

        // ASSERT
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(1));
    }

    #[tokio::test]
    async fn exit_for_unknown_service_does_not_panic() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let mut sup = make_supervisor(vec![], spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.handle_service_exit("ghost", 9999, Some(1_i32), None);
    }

    #[tokio::test]
    async fn process_pending_restarts_respawns_due_service() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let mut sup = make_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        injector.inject("svc", Some(1_i32));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);
        assert_eq!(
            sup.state("svc").map(|state| &state.status),
            Some(&ServiceStatus::Pending)
        );
        assert_eq!(sup.state("svc").map(|state| state.restart_count), Some(1));

        let not_yet_due = sup.restart_queue.take_due(|_| false);
        assert_eq!(not_yet_due, Vec::<alloc::string::String>::new());

        let due = sup.restart_queue.take_due(|_| true);

        // ACT
        respawn_all(&mut sup, due);

        // ASSERT
        let status = sup.state("svc").map(|state| state.status.clone());
        assert!(
            status == Some(ServiceStatus::Starting) || status == Some(ServiceStatus::Pending),
            "unexpected status: {status:?}"
        );
    }

    #[tokio::test]
    async fn empty_exit_batch_publishes_nothing() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, _injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let (mut sup, mut status_rx) =
            make_observed_supervisor(services, spawner, reaper, &dir).expect("supervisor");
        sup.start_ready_services();
        status_rx.borrow_and_update();

        // ACT
        sup.process_exits(Vec::new());

        // ASSERT
        assert!(!status_rx.has_changed().expect("sender alive"));
    }

    #[tokio::test]
    async fn snapshot_reports_start_and_restart_count() {
        // ARRANGE
        let dir = TempDir::new().expect("tempdir");
        let (reaper, injector) = FakeReaper::new();
        let spawner = FakeSpawner::new();
        let services = vec![make_service("svc", &[])];
        let (mut sup, status_rx) =
            make_observed_supervisor(services, spawner, reaper, &dir).expect("supervisor");

        // ACT
        sup.start_ready_services();
        let after_start = status_rx.borrow().clone();
        injector.inject("svc", Some(1_i32));
        let exits = sup.reaper.wait_for_exits().await;
        sup.process_exits(exits);
        let after_exit = status_rx.borrow().clone();

        // ASSERT
        assert_eq!(
            after_start,
            vec![View {
                name: "svc".to_owned(),
                status: ServiceStatus::Starting,
                restarts: 0,
            }]
        );
        assert_eq!(after_exit[0].status, ServiceStatus::Pending);
        assert_eq!(after_exit[0].restarts, 1);
    }
}
