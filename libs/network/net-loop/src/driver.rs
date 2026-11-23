//! Driver construction, stepping, and wake deadline.

use alloc::collections::VecDeque;
use std::io;
use std::time::{Instant, SystemTime};

use crate::pending::Pending;
use crate::{Progress, interface_effects, supervisor_effects};

impl<R: net_runtime::Runtime> crate::Loop<R> {
    /// Creates a driver loop for the runtime and supervisor configuration.
    #[must_use = "the loop owns the FSMs and must be stepped"]
    pub fn new(runtime: R, config: net_supervisor::Config) -> Self {
        Self {
            runtime,
            supervisor: net_supervisor::Supervisor::new(config),
            interfaces: Vec::new(),
            names: Vec::new(),
            sockets: crate::SocketTable::new(),
            pending: VecDeque::new(),
        }
    }

    /// Adds an interface controller for the given link.
    pub fn add_interface(&mut self, link: ip_method::LinkInfo) {
        self.names.push(link.name.clone());
        self.interfaces.push(iface::Controller::new(link));
    }

    /// Returns the interface controllers in insertion order.
    #[must_use = "callers need the interfaces to inspect their state"]
    pub fn interfaces(&self) -> &[iface::Controller] {
        &self.interfaces
    }

    /// Returns the logical socket table.
    #[must_use = "callers need the table to inspect the open sockets"]
    pub const fn sockets(&self) -> &crate::SocketTable {
        &self.sockets
    }

    /// Earliest instant at which the driver must step again.
    ///
    /// Queued pending inputs are ready immediately, so they force a wake at
    /// `now` instead of waiting for the next FSM deadline.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if !self.pending.is_empty() {
            return Some(now);
        }

        let mut next = self.supervisor.next_wake(now);
        for controller in &self.interfaces {
            next = next.min(controller.next_wake(now));
        }
        next
    }

    /// Feeds at most one input to the FSMs and executes the resulting effect.
    ///
    /// Pending inputs win over polling, the supervisor wins over interfaces,
    /// and interfaces are polled in insertion order. One step never consumes
    /// more than one input nor executes more than one effect.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while executing an effect.
    pub async fn step(&mut self, now: Instant, wall: SystemTime) -> io::Result<Progress> {
        if let Some(pending) = self.pending.pop_front() {
            self.feed(pending, now, wall).await?;
            return Ok(Progress::Worked);
        }

        // Executing an effect counts as work even when dispatch is a no-op,
        // such as `Publish` or `WriteResolver`, because the FSM advanced.
        if self
            .poll_supervisor(now, net_supervisor::Input::Poll)
            .await?
        {
            return Ok(Progress::Worked);
        }

        for (name, controller) in self.names.iter().zip(&mut self.interfaces) {
            let effect = match controller.step(now, wall, iface::Input::Poll) {
                iface::Step::Effect(effect) => effect,
                iface::Step::Wait => continue,
            };
            interface_effects::apply(&self.runtime, name, &mut self.sockets, effect).await?;
            return Ok(Progress::Worked);
        }

        Ok(Progress::Idle)
    }

    async fn feed(&mut self, pending: Pending, now: Instant, wall: SystemTime) -> io::Result<()> {
        match pending {
            Pending::Links(links) => {
                self.poll_supervisor(now, net_supervisor::Input::Links(&links))
                    .await?;
            }
            Pending::Carriers(carriers) => {
                self.poll_supervisor(now, net_supervisor::Input::Carriers(&carriers))
                    .await?;
            }
            Pending::ConfigureInterface { name, v4, v6 } => {
                self.command(now, wall, &name, iface::Command::Configure { v4, v6 })
                    .await?;
            }
            Pending::ShutdownInterface { name } => {
                self.command(now, wall, &name, iface::Command::Shutdown)
                    .await?;
            }
        }

        Ok(())
    }

    async fn poll_supervisor(
        &mut self,
        now: Instant,
        input: net_supervisor::Input<'_>,
    ) -> io::Result<bool> {
        let net_supervisor::Step::Effect(effect) = self.supervisor.step(now, input) else {
            return Ok(false);
        };
        supervisor_effects::apply(&self.runtime, &mut self.pending, effect).await?;

        Ok(true)
    }

    async fn command(
        &mut self,
        now: Instant,
        wall: SystemTime,
        name: &netmodel::Name,
        command: iface::Command,
    ) -> io::Result<()> {
        let Some(controller) = self
            .interfaces
            .iter_mut()
            .find(|controller| &controller.link().name == name)
        else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no interface controller for the command",
            ));
        };

        let effect = match controller.step(now, wall, iface::Input::Command(command)) {
            iface::Step::Effect(effect) => effect,
            iface::Step::Wait => return Ok(()),
        };
        interface_effects::apply(&self.runtime, name, &mut self.sockets, effect).await
    }
}
