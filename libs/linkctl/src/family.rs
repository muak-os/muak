//! Address family adapter and its controller slot.

use alloc::string::String;
use std::time::Instant;

use addrconf::State;
use netdev::interface::Ethernet;
use tokio::sync::mpsc;

use crate::interface::Notice;

/// Address family an interface controller can drive.
pub trait Family: 'static {
    /// Whether this build enables the family.
    const ENABLED: bool;
    /// Configuration the supervisor sends to this family.
    type Config;
    /// Address method the controller drives for this family.
    type Method: addrconf::Method<View = Self::View> + Send + 'static;
    /// Point-in-time state the family exposes to the supervisor.
    type View: Send + 'static;

    /// Creates the address method for the given configuration and link.
    ///
    /// # Errors
    ///
    /// Returns the warning to report when the method cannot be set up on `link`.
    fn configure(
        config: Self::Config,
        link: &Ethernet,
    ) -> impl Future<Output = Result<Self::Method, String>> + Send;
}

/// Method, state, and notices the controller keeps for one address family.
pub(crate) struct Slot<F: Family> {
    method: Option<F::Method>,
    state: State,
    configured: bool,
    notice_tx: mpsc::Sender<Notice<F::View>>,
}

impl<F: Family> Slot<F> {
    pub(crate) const fn new(notice_tx: mpsc::Sender<Notice<F::View>>) -> Self {
        Self {
            method: None,
            state: State::Stopped,
            configured: false,
            notice_tx,
        }
    }

    pub(crate) async fn configure(&mut self, config: F::Config, link: &Ethernet) {
        if !F::ENABLED {
            return;
        }
        match F::configure(config, link).await {
            Ok(mut method) => {
                addrconf::Method::on_command(&mut method, addrconf::Command::Start);
                self.method = Some(method);
                self.configured = false;
            }
            Err(warning) => {
                self.method = None;
                self.fail(warning);
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        self.transition(State::Stopped);
        self.method = None;
    }

    pub(crate) fn forward(&mut self, command: addrconf::Command) {
        if let Some(method) = self.method.as_mut() {
            addrconf::Method::on_command(method, command);
        }
    }

    pub(crate) fn notify(&self, notice: Notice<F::View>) {
        drop(self.notice_tx.try_send(notice));
    }

    pub(crate) fn transition(&mut self, state: State) {
        if self.state != state {
            self.state = state;
            self.notify(Notice::State(state));
        }
        if state == State::Bound
            && !self.configured
            && let Some(view) = self.view()
        {
            self.notify(Notice::Configured(view));
            self.configured = true;
        }
        if matches!(state, State::Acquiring | State::Stopped) {
            self.configured = false;
        }
    }

    pub(crate) fn fail(&mut self, warning: String) {
        self.notify(Notice::Warning(warning));
        self.transition(State::Degraded);
    }

    pub(crate) async fn next_event(&mut self, now: Instant) -> addrconf::Event {
        match self.method.as_mut() {
            Some(method) => addrconf::Method::next_event(method, now).await,
            None => core::future::pending().await,
        }
    }

    pub(crate) fn next_wake(&self, now: Instant) -> Option<Instant> {
        let method = self.method.as_ref()?;

        addrconf::Method::next_wake(method, now)
    }

    pub(crate) fn view(&self) -> Option<F::View> {
        self.method.as_ref().and_then(addrconf::Method::view)
    }

    pub(crate) const fn state(&self) -> State {
        self.state
    }

    pub(crate) const fn is_active(&self) -> bool {
        self.method.is_some()
    }
}
