//! Declarative interface and bridge provisioning.

use netmodel::{InterfaceConfig, InterfaceKind, Name};

use crate::actions::Action;
use crate::{Supervisor, view};

/// Queues bridge creation, port enslavement, and address configuration.
pub(crate) fn provision_all(supervisor: &mut Supervisor) {
    for index in 0..supervisor.config.interfaces.len() {
        provision_bridge(supervisor, index);
    }
    for index in 0..supervisor.config.interfaces.len() {
        provision_addresses(supervisor, index);
    }
}

/// Reapplies one configured interface during reconciliation.
pub(crate) fn reconcile_interface(supervisor: &mut Supervisor, index: usize) {
    let (is_bridge, has, name) = {
        let Some(config) = supervisor.config.interfaces.get(index) else {
            return;
        };
        (
            config.kind == InterfaceKind::Bridge,
            has_config(config),
            resolve_name(supervisor, &config.name),
        )
    };
    if is_bridge {
        queue_bridge(supervisor, index);
    }
    if !has {
        return;
    }
    let Some(name) = name else {
        return;
    };
    let target = resolve_owner(supervisor, &name);
    queue_reapply(supervisor, index, target);
}

/// Queues the address configuration of every interface the `master` bridge owns.
pub(crate) fn configure_master(supervisor: &mut Supervisor, master: &Name) {
    for index in 0..supervisor.config.interfaces.len() {
        let Some(config) = supervisor.config.interfaces.get(index) else {
            return;
        };
        if !has_config(config) {
            continue;
        }
        let Some(name) = resolve_name(supervisor, &config.name) else {
            continue;
        };
        if &resolve_owner(supervisor, &name) == master {
            queue_reapply(supervisor, index, master.clone());
        }
    }
}

/// Ensures the bridge at `index` exists and enslaves its port.
pub(crate) fn queue_bridge(supervisor: &mut Supervisor, index: usize) -> bool {
    let Some(port) = bridge_port(supervisor, index) else {
        return false;
    };
    if !supervisor.known.contains(&port) {
        return false;
    }
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return false;
    };
    let Ok(bridge) = Name::new(config.name.clone()) else {
        return false;
    };

    let bridge_up = supervisor
        .links
        .iter()
        .any(|link| link.name == bridge && link.is_admin_up());
    if !supervisor.known.contains(&bridge) || !bridge_up {
        supervisor.queue.push_back(Action::EnsureBridge {
            index,
            name: bridge.clone(),
        });
    }
    if supervisor.enslaved.get(&port) != Some(&bridge) {
        supervisor.queue.push_back(Action::Enslave {
            port: port.clone(),
            master: bridge.clone(),
        });
    }
    release_stale(supervisor, &bridge, &port);

    true
}

/// Queues address configuration for the interface at `index` through `target`.
pub(crate) fn queue_configure(supervisor: &mut Supervisor, index: usize, target: Name) {
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return;
    };
    if !has_config(config) || !supervisor.known.contains(&target) {
        return;
    }
    supervisor.queue.push_back(Action::Configure {
        #[cfg(any(feature = "ipv4", feature = "ipv6"))]
        index,
        target,
    });
}

/// Reconciles an interface through `target`, configuring it when its methods stopped.
pub(crate) fn queue_reapply(supervisor: &mut Supervisor, index: usize, target: Name) {
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return;
    };
    if !has_config(config) || !supervisor.known.contains(&target) {
        return;
    }
    let active = supervisor
        .snapshots
        .get(&target)
        .is_some_and(view::is_active);
    if active {
        supervisor
            .queue
            .push_back(Action::ReconcileInterface { name: target });
    } else {
        supervisor.queue.push_back(Action::Configure {
            #[cfg(any(feature = "ipv4", feature = "ipv6"))]
            index,
            target,
        });
    }
}

/// Resolves a configured interface name, expanding the `auto` alias.
pub(crate) fn resolve_name(supervisor: &Supervisor, name: &str) -> Option<Name> {
    if name == "auto" {
        return supervisor.state.primary.clone();
    }

    Name::new(name).ok()
}

/// Returns the bridge that owns `name` as a port, or `name` itself.
pub(crate) fn resolve_owner(supervisor: &Supervisor, name: &Name) -> Name {
    let Some(primary) = supervisor.state.primary.as_ref() else {
        return name.clone();
    };

    supervisor
        .config
        .interfaces
        .iter()
        .enumerate()
        .find(|&(index, config)| {
            config.kind == InterfaceKind::Bridge
                && bridge_port_for(supervisor, index, primary).as_ref() == Some(name)
        })
        .and_then(|(_, config)| Name::new(config.name.clone()).ok())
        .unwrap_or_else(|| name.clone())
}

/// Resolves the port a bridge should attach to, expanding the `auto` alias.
pub(crate) fn bridge_port(supervisor: &Supervisor, index: usize) -> Option<Name> {
    let primary = supervisor.state.primary.as_ref()?;

    bridge_port_for(supervisor, index, primary)
}

fn provision_bridge(supervisor: &mut Supervisor, index: usize) {
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return;
    };
    if config.kind == InterfaceKind::Bridge {
        queue_bridge(supervisor, index);
    }
}

fn provision_addresses(supervisor: &mut Supervisor, index: usize) {
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return;
    };
    if !has_config(config) {
        return;
    }
    let Some(name) = resolve_name(supervisor, &config.name) else {
        return;
    };
    let owner = resolve_owner(supervisor, &name);
    queue_configure(supervisor, index, owner);
}

fn bridge_port_for(supervisor: &Supervisor, index: usize, primary: &Name) -> Option<Name> {
    let port = supervisor
        .config
        .interfaces
        .get(index)?
        .bridge
        .as_ref()
        .and_then(|bridge| bridge.port.first());

    match port.map(String::as_str) {
        Some("auto") | None => Some(primary.clone()),
        Some(port) => Name::new(port).ok(),
    }
}

fn release_stale(supervisor: &mut Supervisor, bridge: &Name, desired: &Name) {
    let stale: Vec<Name> = supervisor
        .enslaved
        .iter()
        .filter(|&(port, master)| master == bridge && port != desired)
        .map(|(port, _)| port.clone())
        .collect();
    for port in stale {
        supervisor.enslaved.remove(&port);
        supervisor.owners.insert(port.clone(), port.clone());
        supervisor.queue.push_back(Action::Detach { port });
    }
}

/// Returns true when the interface declares configuration for an enabled family.
pub(crate) const fn has_config(iface: &InterfaceConfig) -> bool {
    (cfg!(feature = "ipv4") && iface.ipv4.is_some())
        || (cfg!(feature = "ipv6") && iface.ipv6.is_some())
}
