//! Integration tests for the namespace set.

#[cfg(test)]
mod tests {
    use sandbox::namespace::Set;

    const ALL: [Set; 5] = [Set::MOUNT, Set::NETWORK, Set::IPC, Set::UTS, Set::CGROUP];

    #[test]
    fn every_kind_is_a_distinct_single_bit() {
        // ARRANGE
        let mut bits = [0_u32; 5];

        // ACT
        for (position, kind) in ALL.into_iter().enumerate() {
            bits[position] = kind.bits();
        }

        // ASSERT
        for (position, value) in bits.into_iter().enumerate() {
            assert_eq!(value.count_ones(), 1, "kind {position} is not a single bit");
            assert!(
                !bits[..position].contains(&value),
                "kind {position} repeats bit {value:#x}"
            );
        }
    }

    #[test]
    fn empty_set_ignores_every_kind() {
        // ARRANGE
        let set = Set::empty();

        // ACT
        let contained = set.contains(Set::MOUNT);
        let intersecting = set.intersects(Set::MOUNT);
        let empty = set.is_empty();

        // ASSERT
        assert!(!contained);
        assert!(!intersecting);
        assert!(empty);
        assert!(set.contains(Set::empty()));
    }

    #[test]
    fn union_tracks_what_was_inserted() {
        // ARRANGE
        let set = Set::MOUNT.insert(Set::UTS);

        // ACT
        let both = set.contains(Set::MOUNT.insert(Set::UTS));
        let has_mount = set.intersects(Set::MOUNT);
        let has_network = set.intersects(Set::NETWORK);
        let subset = set.contains(Set::MOUNT);
        let missing = set.contains(Set::MOUNT.insert(Set::CGROUP));
        let empty = set.is_empty();

        // ASSERT
        assert!(both);
        assert!(has_mount);
        assert!(!has_network);
        assert!(subset);
        assert!(!missing);
        assert!(!empty);
    }

    #[test]
    fn flags_yield_one_kind_per_bit_in_order() {
        // ARRANGE
        let set = Set::UTS.insert(Set::MOUNT);

        // ACT
        let flags: Vec<_> = set.flags().collect();

        // ASSERT
        assert_eq!(flags, [Set::MOUNT, Set::UTS]);
    }

    #[test]
    fn flags_of_a_single_kind_yield_exactly_that_kind() {
        // ARRANGE
        let mut counts = [0_usize; 5];

        // ACT
        for (position, kind) in ALL.into_iter().enumerate() {
            counts[position] = kind.flags().count();
        }

        // ASSERT
        assert_eq!(counts, [1; 5]);
    }
}
