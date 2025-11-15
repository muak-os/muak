use super::interface::{Interface, LinkState};

pub struct InterfaceSelector;

impl InterfaceSelector {
    pub fn select_primary(interfaces: &[Interface]) -> Option<&Interface> {
        if interfaces.is_empty() {
            return None;
        }

        interfaces
            .iter()
            .max_by_key(|iface| Self::score_interface(iface))
    }

    pub fn select_backups<'a>(
        interfaces: &'a [Interface],
        exclude_primary: &str,
    ) -> Vec<&'a Interface> {
        let mut backups: Vec<_> = interfaces
            .iter()
            .filter(|iface| iface.name != exclude_primary)
            .collect();

        backups.sort_by_key(|iface| std::cmp::Reverse(Self::score_interface(iface)));

        backups
    }

    fn score_interface(interface: &Interface) -> u64 {
        let mut score = 0u64;

        if interface.link_state == LinkState::Up {
            score += 1_000_000_000;
        }

        score += Self::naming_preference_score(&interface.name) * 1000;

        score += (255 * 10)
            - interface
                .name
                .bytes()
                .take(10)
                .map(|b| b as u64)
                .sum::<u64>();

        score
    }

    fn naming_preference_score(name: &str) -> u64 {
        if name.starts_with("eno") {
            4
        } else if name.starts_with("ens") {
            3
        } else if name.starts_with("enp") {
            2
        } else if name.starts_with("eth") {
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_interface(name: &str, link_state: LinkState) -> Interface {
        Interface::new(name.to_string(), 1, [0, 0, 0, 0, 0, 0], link_state)
    }

    #[test]
    fn test_select_primary_prefers_link_up() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Down),
            make_interface("eth1", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth1");
    }

    #[test]
    fn test_select_primary_prefers_predictable_names() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eno1", LinkState::Up),
            make_interface("ens1", LinkState::Up),
            make_interface("enp3s0", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eno1");
    }

    #[test]
    fn test_select_primary_lexicographic_tiebreaker() {
        let interfaces = vec![
            make_interface("eth1", LinkState::Up),
            make_interface("eth0", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth0");
    }

    #[test]
    fn test_select_backups_excludes_primary() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Up),
            make_interface("eth2", LinkState::Down),
        ];

        let backups = InterfaceSelector::select_backups(&interfaces, "eth0");
        assert_eq!(backups.len(), 2);
        assert!(!backups.iter().any(|i| i.name == "eth0"));
    }

    #[test]
    fn test_select_backups_sorted_by_priority() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Down),
            make_interface("eth2", LinkState::Up),
        ];

        let backups = InterfaceSelector::select_backups(&interfaces, "nonexistent");
        assert_eq!(backups[0].name, "eth0");
        assert_eq!(backups[1].name, "eth2");
        assert_eq!(backups[2].name, "eth1");
    }

    #[test]
    fn test_empty_interfaces() {
        let interfaces: Vec<Interface> = vec![];
        assert!(InterfaceSelector::select_primary(&interfaces).is_none());
        assert!(InterfaceSelector::select_backups(&interfaces, "any").is_empty());
    }
}
