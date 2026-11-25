//! Link selection integration tests.

pub mod common;

#[cfg(test)]
mod tests {
    use netmodel::{AdminState, CarrierState, Link, LinkState, Name, Selector};

    use crate::common;

    #[test]
    fn primary_prefers_carrier() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::NoCarrier),
            common::link("eth1", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn primary_prefers_no_carrier_over_down() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Down),
            common::link("eth1", LinkState::NoCarrier),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn primary_prefers_up_over_dormant() {
        // ARRANGE
        let links = [
            common::link("eno1", LinkState::Dormant),
            common::link("eth0", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn primary_prefers_dormant_over_admin_down() {
        // ARRANGE
        let links = [
            common::link("eno1", LinkState::Down),
            common::link("eth0", LinkState::Dormant),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn unknown_carrier_ranks_as_up() {
        // ARRANGE
        let links = [
            common::link("eno1", LinkState::NoCarrier),
            common::link_with_parts("eth0", 0, AdminState::Up, CarrierState::Unknown),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn primary_prefers_better_naming() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Up),
            common::link("eno1", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn naming_priority_order() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Up),
            common::link("enp3s0", LinkState::Up),
            common::link("ens1", LinkState::Up),
            common::link("eno1", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn primary_scores_end_above_eth() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Up),
            common::link("end0", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "end0");
    }

    #[test]
    fn primary_handles_unknown_ethernet_like_name() {
        // ARRANGE
        let links = [
            common::link("enx001122334455", LinkState::Up),
            common::link("eth0", LinkState::NoCarrier),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(
            primary.expect("should have primary").name,
            "enx001122334455"
        );
    }

    #[test]
    fn carrier_overrides_naming() {
        // ARRANGE
        let links = [
            common::link("eno1", LinkState::NoCarrier),
            common::link("eth0", LinkState::Up),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn backups_excludes_primary() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Up),
            common::link("eth1", LinkState::Up),
            common::link("eth2", LinkState::Down),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = Selector::backups(&links, &primary_name);

        // ASSERT
        assert_eq!(backups.len(), 2);
        assert!(backups.iter().all(|link| link.name != "eth0"));
    }

    #[test]
    fn backups_sorted_by_priority() {
        // ARRANGE
        let links = [
            common::link("eth0", LinkState::Up),
            common::link("eth1", LinkState::Down),
            common::link("eno1", LinkState::Up),
            common::link("enp3s0", LinkState::Up),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = Selector::backups(&links, &primary_name);
        let backup_names: Vec<_> = backups.iter().map(|link| link.name.as_str()).collect();

        // ASSERT
        assert_eq!(backup_names.as_slice(), ["eno1", "enp3s0", "eth1"]);
    }

    #[test]
    fn primary_of_empty_is_none() {
        // ARRANGE
        let links: [Link; 0] = [];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert!(primary.is_none());
    }

    #[test]
    fn primary_of_single_link_returns_it() {
        // ARRANGE
        let links = [common::link("eth0", LinkState::Down)];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn tiebreaker_prefers_lower_index() {
        // ARRANGE
        let links = [
            common::link_with_index("eth0", 8, LinkState::Down),
            common::link_with_index("eth1", 9, LinkState::Down),
            common::link_with_index("eth2", 10, LinkState::Down),
            common::link_with_index("eth3", 11, LinkState::Down),
        ];

        // ACT
        let primary = Selector::primary(&links);

        // ASSERT
        let primary_link = primary.expect("should have primary");
        assert_eq!(primary_link.name, "eth0");
        assert_eq!(primary_link.index, 8);
    }
}
