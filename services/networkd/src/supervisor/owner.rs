//! L3 ownership: the interface that carries addresses for the elected primary.

use std::collections::HashMap;

use netlink::link::ifname::Name;
use netstack::interface::Handle;

use super::NetworkSnapshot;

/// Returns the interface that owns L3: the recorded bridge when present,
/// otherwise the primary's L3 owner.
pub(super) fn l3_owner<'a>(
    state: &'a NetworkSnapshot,
    interfaces: &'a HashMap<Name, Handle>,
    owners: &'a HashMap<Name, Name>,
) -> Option<&'a Name> {
    if let Some(owner) = state.l3_owner.as_ref()
        && interfaces.contains_key(owner)
    {
        return Some(owner);
    }

    let primary = state.primary.as_ref()?;
    if !interfaces.contains_key(primary) {
        return None;
    }

    let owner = owners.get(primary).unwrap_or(primary);
    if interfaces.contains_key(owner) {
        Some(owner)
    } else {
        Some(primary)
    }
}

/// Returns true when `name` is attached to a bridge that owns its L3.
pub(super) fn is_enslaved(owners: &HashMap<Name, Name>, name: &Name) -> bool {
    owners.get(name).is_some_and(|owner| owner != name)
}

#[cfg(test)]
mod tests {
    use netlink::link::state::State;
    use netstack::interface::{Handle, Snapshot};
    use netstack::provision;
    use tokio::sync::{mpsc, watch};

    use super::*;

    fn name(value: &str) -> Name {
        Name::new(value).expect("valid name")
    }

    fn handle(link: State) -> Handle {
        let snapshot = Snapshot {
            link,
            v4: None,
            v4_state: provision::State::Stopped,
            v6: None,
            v6_state: provision::State::Stopped,
        };
        let (command_tx, _command_rx) = mpsc::channel(1);
        let (_snapshot_tx, snapshot_rx) = watch::channel(snapshot);

        Handle {
            command_tx,
            snapshot_rx,
        }
    }

    fn interfaces(names: &[&str]) -> HashMap<Name, Handle> {
        names
            .iter()
            .map(|value| (name(value), handle(State::Up)))
            .collect()
    }

    #[test]
    fn owner_follows_the_primary_bridge() {
        // ARRANGE
        let mut state = NetworkSnapshot::empty();
        state.primary = Some(name("eth0"));
        let interfaces = interfaces(&["eth0", "br0"]);
        let owners = HashMap::from([(name("eth0"), name("br0")), (name("br0"), name("br0"))]);

        // ACT
        let owner = l3_owner(&state, &interfaces, &owners);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), Some("br0"));
    }

    #[test]
    fn owner_prefers_the_recorded_bridge() {
        // ARRANGE
        let mut state = NetworkSnapshot::empty();
        state.l3_owner = Some(name("br0"));
        let interfaces = interfaces(&["br0"]);
        let owners = HashMap::new();

        // ACT
        let owner = l3_owner(&state, &interfaces, &owners);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), Some("br0"));
    }

    #[test]
    fn owner_falls_back_to_primary_without_the_bridge() {
        // ARRANGE
        let mut state = NetworkSnapshot::empty();
        state.primary = Some(name("eth0"));
        state.l3_owner = Some(name("br0"));
        let interfaces = interfaces(&["eth0"]);
        let owners = HashMap::from([(name("eth0"), name("br0"))]);

        // ACT
        let owner = l3_owner(&state, &interfaces, &owners);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), Some("eth0"));
    }

    #[test]
    fn owner_is_none_without_a_primary() {
        // ARRANGE
        let state = NetworkSnapshot::empty();
        let interfaces = HashMap::new();
        let owners = HashMap::new();

        // ACT
        let owner = l3_owner(&state, &interfaces, &owners);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), None);
    }

    #[test]
    fn enslaved_reports_a_bridge_port() {
        // ARRANGE
        let owners = HashMap::from([(name("eth0"), name("br0")), (name("br0"), name("br0"))]);

        // ACT / ASSERT
        assert!(is_enslaved(&owners, &name("eth0")));
        assert!(!is_enslaved(&owners, &name("br0")));
        assert!(!is_enslaved(&owners, &name("eth9")));
    }
}
