//! Input dispatch, commands, effect failures, and timers.

use std::time::Instant;

use crate::actions::Action;
use crate::{
    Command, EffectKind, Input, State, Supervisor, discovery, feedback, link_events, reconcile,
    retry,
};

/// Routes one input into the matching policy handler.
pub(crate) fn handle(
    supervisor: &mut Supervisor,
    now: Instant,
    input: Input<'_>,
    journal: &mut net_observe::journal::Journal,
) {
    match input {
        Input::Poll => {}
        Input::Command(command) => on_command(supervisor, now, command),
        Input::Links(links) => discovery::on_links(supervisor, now, links, journal),
        Input::Carriers(carriers) => discovery::on_carriers(supervisor, now, carriers),
        Input::LinkEvent(event) => {
            supervisor.processed_events = supervisor.processed_events.saturating_add(1);
            journal.record(
                net_observe::Level::Info,
                net_observe::Event::LinkEvent {
                    event: event.clone(),
                },
            );
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
        } => {
            if link_state_changed(supervisor, &name, state) {
                journal.record(
                    net_observe::Level::Info,
                    net_observe::Event::BridgeReady {
                        name: name.clone(),
                        index,
                        state,
                    },
                );
            }
            feedback::on_bridge_ready(supervisor, &name, index, mac, state);
        }
        Input::Enslaved { port, master } => {
            if supervisor.enslaved.get(&port) != Some(&master) {
                journal.record(
                    net_observe::Level::Info,
                    net_observe::Event::Enslaved {
                        port: port.clone(),
                        master: master.clone(),
                    },
                );
            }
            feedback::on_enslaved(supervisor, &port, &master);
        }
        Input::Detached { port } => {
            if supervisor.enslaved.contains_key(&port) {
                journal.record(
                    net_observe::Level::Info,
                    net_observe::Event::Detached { port: port.clone() },
                );
            }
            feedback::on_detached(supervisor, &port);
        }
        Input::EffectFailed { kind, .. } => on_effect_failed(supervisor, now, kind),
        Input::Timer => on_timer(supervisor, now),
    }
}

fn link_state_changed(
    supervisor: &Supervisor,
    name: &netmodel::Name,
    state: netmodel::LinkState,
) -> bool {
    !supervisor
        .links
        .iter()
        .find(|link| &link.name == name)
        .is_some_and(|link| link.state() == state)
}

fn on_command(supervisor: &mut Supervisor, now: Instant, command: Command) {
    match command {
        Command::Initialize => initialize(supervisor, now),
        Command::Reconcile => reconcile::run(supervisor),
        Command::Shutdown => shutdown(supervisor),
    }
}

fn initialize(supervisor: &mut Supervisor, now: Instant) {
    let previous = supervisor.state.state;
    shutdown(supervisor);
    supervisor.state.transitions.clear();
    supervisor.reconcile_at = now.checked_add(reconcile::RECONCILE_INTERVAL);
    supervisor.state.transition(State::Initializing);
    if let Some(transition) = supervisor.state.transitions.last_mut() {
        *transition = (previous, State::Initializing);
    }
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
    supervisor.state.reset();
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

    supervisor.reconcile_at = now.checked_add(reconcile::RECONCILE_INTERVAL);
    if supervisor.state.is_operational() {
        reconcile::run(supervisor);
    }
}

fn fail_discovery(supervisor: &mut Supervisor, now: Instant) {
    if supervisor.state.is_operational() {
        return;
    }
    supervisor.state.ensure_degraded();
    retry::schedule(supervisor, now);
}
