//! Runtime loop that drives the network supervisor and writes the resolver.

use std::io;
use std::time::{Duration, Instant, SystemTime};

use granola::runtime::notify::{Health, Notifier};
use granola::runtime::signal::shutdown;
use net_loop::{Loop, Progress};
use net_runtime::Runtime;

use crate::dns::Resolver;
use crate::logging;

/// Status text a ready network reports to the supervisor.
pub const READY_STATUS: &str = "Network operational";

/// Status text a degraded network reports to the supervisor.
pub const DEGRADED_STATUS: &str = "Network degraded";

/// Outcome of waiting for the network to become usable at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The network reached `Operational` or `Ready` within the timeout.
    Ready,
    /// The timeout elapsed while the network was still unavailable.
    Degraded,
    /// A shutdown signal arrived while waiting.
    Shutdown,
}

/// Network service owning the driver loop and the resolver file.
pub struct Service<R: Runtime> {
    driver: Loop<R>,
    resolver: Resolver,
    reported: Option<Health>,
}

impl<R: Runtime> Service<R> {
    /// Creates the service over the runtime and supervisor configuration.
    #[must_use = "the service owns the driver loop and must be started"]
    pub fn new(runtime: R, config: net_supervisor::Config) -> Self {
        Self {
            driver: Loop::new(runtime, config),
            resolver: Resolver::default(),
            reported: None,
        }
    }

    /// Starts the supervisor and drains the work it produces immediately.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while draining the loop.
    pub async fn start(&mut self) -> io::Result<()> {
        self.driver.start(Instant::now()).await;

        self.settle().await
    }

    /// Drains the loop until the network reports usable or the timeout elapses.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while stepping or waiting.
    pub async fn wait_ready(&mut self, timeout: Duration) -> io::Result<Readiness> {
        let deadline = Instant::now().checked_add(timeout);
        self.settle().await?;

        self.await_ready(deadline).await
    }

    /// Steps and waits until a shutdown signal is received.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while stepping, waiting, or notifying the supervisor.
    pub async fn run(&mut self, notifier: &Notifier) -> io::Result<()> {
        self.settle().await?;
        self.report(notifier)?;

        while self.wait(None).await? {
            self.settle().await?;
            self.report(notifier)?;
        }

        Ok(())
    }

    async fn await_ready(&mut self, deadline: Option<Instant>) -> io::Result<Readiness> {
        let mut outcome = None;
        while outcome.is_none() && !ready(self.driver.status().state) {
            outcome = self.wait_within(deadline).await?;
        }

        Ok(outcome.unwrap_or(Readiness::Ready))
    }

    async fn wait_within(&mut self, deadline: Option<Instant>) -> io::Result<Option<Readiness>> {
        let Some(deadline) = deadline else {
            return Ok(Some(Readiness::Degraded));
        };
        if Instant::now() >= deadline {
            return Ok(Some(Readiness::Degraded));
        }
        if !self.wait(Some(deadline)).await? {
            return Ok(Some(Readiness::Shutdown));
        }
        self.settle().await?;

        Ok(None)
    }

    async fn settle(&mut self) -> io::Result<()> {
        let now = Instant::now();
        let wall = SystemTime::now();
        while self.drain_step(now, wall).await? == Progress::Worked {}

        let (v4, v6) = self.driver.resolver();
        if self.resolver.is_unchanged(v4, v6) {
            return Ok(());
        }
        if let Err(error) = self.resolver.update(v4.to_vec(), v6.to_vec()) {
            kmsg::warn!("Failed to write DNS: {error}");
        }

        Ok(())
    }

    async fn drain_step(&mut self, now: Instant, wall: SystemTime) -> io::Result<Progress> {
        let stepped = self.driver.step(now, wall).await;
        self.flush_observations();

        stepped
    }

    fn flush_observations(&mut self) {
        for observation in self.driver.drain_observations() {
            logging::emit(observation);
        }
    }

    fn report(&mut self, notifier: &Notifier) -> io::Result<()> {
        let status = status(self.driver.status().state);
        if status.map(|(_, health)| health) == self.reported {
            return Ok(());
        }

        if let Some((text, health)) = status {
            notifier.status(text, health)?;
        }
        self.reported = status.map(|(_, health)| health);

        Ok(())
    }

    async fn wait(&mut self, limit: Option<Instant>) -> io::Result<bool> {
        let wake = match (self.driver.next_wake(Instant::now()), limit) {
            (Some(wake), Some(limit)) => Some(wake.min(limit)),
            (wake, None) | (None, wake) => wake,
        };

        if let Some(wake) = wake {
            tokio::select! {
                biased;

                () = shutdown() => return Ok(false),
                result = self.driver.wait_event() => {
                    result?;
                }
                () = tokio::time::sleep_until(wake.into()) => {}
            }
        } else {
            tokio::select! {
                biased;

                () = shutdown() => return Ok(false),
                result = self.driver.wait_event() => {
                    result?;
                }
            }
        }

        Ok(true)
    }
}

const fn ready(state: net_supervisor::State) -> bool {
    matches!(
        state,
        net_supervisor::State::Operational | net_supervisor::State::Ready
    )
}

const fn status(state: net_supervisor::State) -> Option<(&'static str, Health)> {
    match state {
        net_supervisor::State::Operational | net_supervisor::State::Ready => {
            Some((READY_STATUS, Health::Healthy))
        }
        net_supervisor::State::Degraded => Some((DEGRADED_STATUS, Health::Degraded)),
        net_supervisor::State::Initializing | net_supervisor::State::Uninitialized => None,
    }
}
