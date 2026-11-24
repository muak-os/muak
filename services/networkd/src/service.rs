//! Runtime loop that drives the network supervisor and writes the resolver.

use std::io;
use std::time::{Instant, SystemTime};

use granola::runtime::signal::shutdown;
use net_loop::{Loop, Progress};
use net_runtime_linux::Tokio;

use crate::dns::Resolver;
use crate::notices::Logged;

/// Network service owning the driver loop and the resolver file.
pub struct Service {
    driver: Loop<Tokio>,
    resolver: Resolver,
    logged: Logged,
}

impl Service {
    /// Creates the service over the Linux runtime and supervisor configuration.
    #[must_use = "the service owns the driver loop and must be started"]
    pub fn new(runtime: Tokio, config: net_supervisor::Config) -> Self {
        Self {
            driver: Loop::new(runtime, config),
            resolver: Resolver::default(),
            logged: Logged::default(),
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

    /// Drains the loop until the supervisor reports the network as usable.
    ///
    /// Returns `true` when the network reached
    /// [`net_supervisor::State::Operational`] or [`net_supervisor::State::Ready`],
    /// and `false` when a shutdown was requested first.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while stepping or waiting.
    pub async fn wait_ready(&mut self) -> io::Result<bool> {
        self.settle().await?;
        while !ready(self.driver.status().state) && self.wait().await? {
            self.settle().await?;
        }

        Ok(ready(self.driver.status().state))
    }

    /// Steps and waits until a shutdown signal is received.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while stepping or waiting.
    pub async fn run(&mut self) -> io::Result<()> {
        self.settle().await?;
        while self.wait().await? {
            self.settle().await?;
        }

        Ok(())
    }

    async fn settle(&mut self) -> io::Result<()> {
        let now = Instant::now();
        let wall = SystemTime::now();
        while self.driver.step(now, wall).await? == Progress::Worked {}

        for controller in self.driver.interfaces() {
            log(
                &controller.link().name,
                &mut self.logged,
                &controller.snapshot().notices,
            );
        }

        let (v4, v6) = self.driver.resolver();
        if self.resolver.is_unchanged(v4, v6) {
            return Ok(());
        }
        if let Err(error) = self.resolver.update(v4.to_vec(), v6.to_vec()) {
            kmsg::warn!("Failed to write DNS: {error}");
        }

        Ok(())
    }

    async fn wait(&mut self) -> io::Result<bool> {
        let deadline = self.driver.next_wake(Instant::now());
        if let Some(deadline) = deadline {
            tokio::select! {
                biased;

                () = shutdown() => return Ok(false),
                result = self.driver.wait_event() => {
                    result?;
                }
                () = tokio::time::sleep_until(deadline.into()) => {}
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

fn log(name: &netmodel::Name, logged: &mut Logged, notices: &[String]) {
    logged.unseen(name, notices, |notice| kmsg::warn!("{name}: {notice}"));
}

const fn ready(state: net_supervisor::State) -> bool {
    matches!(
        state,
        net_supervisor::State::Operational | net_supervisor::State::Ready
    )
}
