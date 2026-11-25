//! Driver stepping, event waiting, and supervisor dispatch.

use std::io;
use std::time::{Instant, SystemTime};

use net_runtime::Events as _;

use crate::pending::Pending;
use crate::{Loop, Progress, feedback, interface_effects, supervisor_effects};

pub(crate) async fn start<R: net_runtime::Runtime>(driver: &mut Loop<R>, now: Instant) -> Progress {
    if driver.started {
        return Progress::Idle;
    }
    let input = net_supervisor::Input::Command(net_supervisor::Command::Initialize);
    let progress = drive(driver, now, input).await;
    driver.started = true;

    progress
}

pub(crate) async fn run<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
) -> io::Result<Progress> {
    if !driver.started {
        return Ok(Progress::Idle);
    }

    if let Some(pending) = driver.pending.pop_front() {
        feedback::feed(driver, pending, now, wall).await?;
        return Ok(Progress::Worked);
    }

    if drive(driver, now, net_supervisor::Input::Timer).await == Progress::Worked {
        return Ok(Progress::Worked);
    }

    if poll_interfaces(driver, now, wall).await? {
        return Ok(Progress::Worked);
    }

    Ok(Progress::Idle)
}

pub(crate) async fn wait_event<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
) -> io::Result<Progress> {
    let event = driver.runtime.events().next().await?;
    driver.pending.push_back(Pending::LinkEvent(event));

    Ok(Progress::Worked)
}

pub(crate) async fn drive<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    input: net_supervisor::Input<'_>,
) -> Progress {
    if poll_supervisor(driver, now, input).await {
        return Progress::Worked;
    }

    Progress::Idle
}

pub(crate) async fn advance<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    index: usize,
    input: iface::Input<'_>,
    now: Instant,
    wall: SystemTime,
) -> io::Result<bool> {
    let Some(name) = driver.names.get(index) else {
        return Ok(false);
    };
    let Some(controller) = driver.interfaces.get_mut(index) else {
        return Ok(false);
    };
    let effect = match controller.step(&mut driver.journal, now, wall, input) {
        iface::Step::Effect(effect) => effect,
        iface::Step::Wait => return Ok(false),
    };

    match effect {
        iface::Effect::Publish => {
            let snapshot = controller.snapshot().clone();
            driver.pending.push_back(Pending::Snapshot {
                name: name.clone(),
                snapshot,
            });
        }
        effect @ (iface::Effect::SetLinkUp { .. }
        | iface::Effect::SetLinkDown { .. }
        | iface::Effect::AddAddress { .. }
        | iface::Effect::RemoveAddress { .. }
        | iface::Effect::EnsureRoute { .. }
        | iface::Effect::RemoveRoute { .. }
        | iface::Effect::OpenSocket { .. }
        | iface::Effect::Send { .. }
        | iface::Effect::CloseSocket { .. }
        | iface::Effect::RequestEntropy { .. }) => {
            interface_effects::apply(
                &driver.runtime,
                name,
                &mut driver.sockets,
                &mut driver.pending,
                &mut driver.journal,
                effect,
            )
            .await?;
        }
    }

    Ok(true)
}

async fn poll_supervisor<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    input: net_supervisor::Input<'_>,
) -> bool {
    let net_supervisor::Step::Effect(effect) =
        driver.supervisor.step(now, input, &mut driver.journal)
    else {
        return false;
    };
    let kind = effect.kind();
    let observed = net_observe::EffectKind::from(kind);
    let target = effect.target().cloned();
    let level = observed.level();
    driver.journal.record(
        level,
        net_observe::Event::EffectStarted {
            kind: observed,
            target: target.clone(),
        },
    );
    let applied = supervisor_effects::apply(
        &driver.runtime,
        &mut driver.pending,
        &mut driver.resolver,
        effect,
    )
    .await;
    match applied {
        Ok(()) => driver.journal.record(
            level,
            net_observe::Event::EffectFinished {
                kind: observed,
                target,
            },
        ),
        Err(error) => {
            driver.journal.record(
                net_observe::Level::Warn,
                net_observe::Event::EffectFailed {
                    kind: observed,
                    target,
                    error: error.to_string(),
                },
            );
            driver
                .pending
                .push_back(Pending::EffectFailed { kind, error });
        }
    }

    true
}

async fn poll_interfaces<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
) -> io::Result<bool> {
    let count = driver.interfaces.len();
    for index in 0..count {
        if advance(driver, index, iface::Input::Poll, now, wall).await? {
            return Ok(true);
        }
    }

    Ok(false)
}
