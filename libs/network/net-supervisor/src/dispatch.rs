//! Input dispatch, commands, effect failures, and timers.

use std::time::Instant;

use crate::actions::Action;
use crate::reconcile::{RECONCILE_INTERVAL, run};
use crate::{
    Command, EffectKind, Input, State, Supervisor, discovery, feedback, link_events, retry,
};

/// Routes one input into the matching policy handler.
pub(crate) fn handle(supervisor: &mut Supervisor, now: Instant, input: Input<'_>) {
    match input {
        Input::Poll => {}
        Input::Command(command) => on_command(supervisor, now, command),
        Input::Links(links) => discovery::on_links(supervisor, now, links),
        Input::Carriers(carriers) => discovery::on_carriers(supervisor, now, carriers),
        Input::LinkEvent(event) => {
            supervisor.processed_events = supervisor.processed_events.saturating_add(1);
            link_events::apply(supervisor, event);
        }
        Input::InterfaceSnapshot { name, snapshot } => {
            feedback::on_snapshot(supervisor, &name, snapshot);
        }
        Input::BridgeReady {
            name,
            index,
            mac,
            state,
        } => feedback::on_bridge_ready(supervisor, &name, index, mac, state),
        Input::Enslaved { port, master } => feedback::on_enslaved(supervisor, &port, &master),
        Input::Detached { port } => feedback::on_detached(supervisor, &port),
        Input::EffectFailed { kind, .. } => on_effect_failed(supervisor, now, kind),
        Input::Timer => on_timer(supervisor, now),
    }
}

fn on_command(supervisor: &mut Supervisor, now: Instant, command: Command) {
    match command {
        Command::Initialize => initialize(supervisor, now),
        Command::Reconcile => run(supervisor),
        Command::Shutdown => shutdown(supervisor),
    }
}

fn initialize(supervisor: &mut Supervisor, now: Instant) {
    shutdown(supervisor);
    supervisor.reconcile_at = now.checked_add(RECONCILE_INTERVAL);
    supervisor.state.transition(State::Initializing);
    supervisor.queue.push_back(Action::Discover);
}

fn shutdown(supervisor: &mut Supervisor) {
    supervisor.queue.clear();
    let names: Vec<netmodel::Name> = supervisor.known.drain().collect();
    for name in names {
        supervisor
            .queue
            .push_back(Action::ShutdownInterface { name });
    }
    supervisor.state = crate::state::Snapshot::default();
    supervisor.links.clear();
    supervisor.physical.clear();
    supervisor.snapshots.clear();
    supervisor.owners.clear();
    supervisor.enslaved.clear();
    supervisor.resolver_v4.clear();
    supervisor.resolver_v6.clear();
    supervisor.reconcile_at = None;
    retry::clear(supervisor);
}

fn on_effect_failed(supervisor: &mut Supervisor, now: Instant, kind: EffectKind) {
    match kind {
        EffectKind::Discover | EffectKind::ProbeCarriers | EffectKind::SetLinkUp => {
            fail_discovery(supervisor, now);
        }
        EffectKind::ConfigureInterface
        | EffectKind::ReconcileInterface
        | EffectKind::EnsureBridge
        | EffectKind::Enslave
        | EffectKind::Detach
        | EffectKind::ShutdownInterface
        | EffectKind::Publish
        | EffectKind::WriteResolver => {}
    }
}

fn on_timer(supervisor: &mut Supervisor, now: Instant) {
    if !supervisor.state.is_operational()
        && let Some(deadline) = supervisor.retry_at
        && now >= deadline
    {
        supervisor.retry_at = None;
        supervisor.queue.push_back(Action::Discover);
    }

    let Some(deadline) = supervisor.reconcile_at else {
        return;
    };
    if now < deadline {
        return;
    }

    supervisor.reconcile_at = now.checked_add(RECONCILE_INTERVAL);
    if supervisor.state.is_operational() {
        run(supervisor);
    }
}

fn fail_discovery(supervisor: &mut Supervisor, now: Instant) {
    if supervisor.state.is_operational() {
        return;
    }
    supervisor.state.ensure_degraded();
    retry::schedule(supervisor, now);
}
