//! Link-state event handlers for a per-interface actor.

use netlib::link::State;
use netlib::netlink::Ops;

use super::Actor;
use crate::dhcp::client::DhcpConnector;
use crate::interface::state::Lifecycle;

impl<N: Ops> Actor<N> {
    /// Handles a link-up event on this interface.
    pub(super) async fn on_link_up<C: DhcpConnector>(&mut self, connector: &C) {
        self.snapshot.link = State::Up;
        if self.snapshot.state != Lifecycle::Degraded {
            return;
        }
        if let Some(lease) = self.snapshot.lease.clone() {
            self.recover_with_lease(lease).await;
            self.publish_snapshot();
            return;
        }
        if let Err(e) = self.do_full_dora(connector).await {
            kmsg::warn!(
                "DHCP re-acquire failed on link-up for {}: {e}",
                self.snapshot.name
            );
        }
        self.publish_snapshot();
    }

    /// Handles a link-down event on this interface.
    pub(super) fn on_link_down(&mut self) {
        self.snapshot.link = State::Down;
        self.dhcp = None;
        self.timers.disarm();
        if self.snapshot.state == Lifecycle::Configured
            && let Err(e) = self.snapshot.transition(Lifecycle::Degraded)
        {
            kmsg::warn!(
                "Interface {} state transition failed on link-down: {}",
                self.snapshot.name,
                e
            );
        }
        self.publish_snapshot();
    }
}
