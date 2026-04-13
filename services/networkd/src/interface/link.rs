//! Link-state event handlers for a per-interface actor.

use netlib::ops::NetlinkOps;

use super::InterfaceActor;
use crate::interface::state::InterfaceState;

impl<N: NetlinkOps> InterfaceActor<N> {
    /// Handles a link-up event on this interface.
    pub(super) async fn on_link_up(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Up;
        if self.snapshot.state != InterfaceState::Degraded {
            return;
        }
        if let Some(lease) = self.snapshot.lease.clone() {
            self.recover_with_lease(lease).await;
        } else if let Err(e) = self.do_full_dora().await {
            kmsg::warn!(
                "DHCP re-acquire failed on link-up for {}: {}",
                self.snapshot.name,
                e
            );
        }
        self.publish_snapshot();
    }

    /// Handles a link-down event on this interface.
    pub(super) fn on_link_down(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Down;
        self.dhcp = None;
        self.timers.disarm();
        if self.snapshot.state == InterfaceState::Configured
            && let Err(e) = self.snapshot.transition(InterfaceState::Degraded)
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
