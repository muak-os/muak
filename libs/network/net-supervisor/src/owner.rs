//! Layer 3 ownership of the interface that carries the elected primary.

use netmodel::Name;

use crate::Supervisor;

/// Returns the interface that owns layer 3.
pub(crate) fn owner(supervisor: &Supervisor) -> Option<&Name> {
    if let Some(owner) = supervisor.state.l3_owner.as_ref()
        && supervisor.known.contains(owner)
    {
        return Some(owner);
    }

    let primary = supervisor.state.primary.as_ref()?;
    if !supervisor.known.contains(primary) {
        return None;
    }

    let owner = supervisor.owners.get(primary).unwrap_or(primary);
    if supervisor.known.contains(owner) {
        Some(owner)
    } else {
        Some(primary)
    }
}

/// Returns true when `name` is attached to a bridge that owns its layer 3.
pub(crate) fn is_enslaved(supervisor: &Supervisor, name: &Name) -> bool {
    supervisor
        .owners
        .get(name)
        .is_some_and(|owner| owner != name)
        || supervisor.enslaved.contains_key(name)
}
