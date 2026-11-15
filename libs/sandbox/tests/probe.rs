//! Integration tests for the kernel capability set.

#[cfg(test)]
mod tests {
    use sandbox::probe::Capabilities;

    const ALL: [Capabilities; 30] = [
        Capabilities::MOUNT_NS,
        Capabilities::PID_NS,
        Capabilities::NET_NS,
        Capabilities::IPC_NS,
        Capabilities::UTS_NS,
        Capabilities::USER_NS,
        Capabilities::CGROUP_NS,
        Capabilities::TIME_NS,
        Capabilities::CGROUP_V2,
        Capabilities::CGROUP_KILL,
        Capabilities::CGROUP_FREEZER,
        Capabilities::OVERLAYFS,
        Capabilities::TMPFS,
        Capabilities::DEVPTS,
        Capabilities::PROC,
        Capabilities::SYSFS,
        Capabilities::EROFS,
        Capabilities::SECCOMP,
        Capabilities::SECCOMP_NOTIF,
        Capabilities::PIVOT_ROOT,
        Capabilities::OPEN_TREE,
        Capabilities::MOUNT_SETATTR,
        Capabilities::IDMAPPED_MOUNTS,
        Capabilities::CAPSET,
        Capabilities::CAP_BOUNDING,
        Capabilities::PIDFD,
        Capabilities::CLONE3,
        Capabilities::CLONE_INTO_CGROUP,
        Capabilities::LANDLOCK,
        Capabilities::UNPRIV_USER_NS,
    ];

    #[test]
    fn every_flag_is_a_distinct_single_bit() {
        // ARRANGE
        let mut bits = [0_u32; 30];

        // ACT
        for (position, flag) in ALL.into_iter().enumerate() {
            bits[position] = flag.bits();
        }

        // ASSERT
        for (position, value) in bits.into_iter().enumerate() {
            assert_eq!(value.count_ones(), 1, "flag {position} is not a single bit");
            assert!(
                !bits[..position].contains(&value),
                "flag {position} repeats bit {value:#x}"
            );
        }
    }

    #[test]
    fn all_flags_fit_in_the_bit_set() {
        // ARRANGE
        let mut union = Capabilities::empty();

        // ACT
        for flag in ALL {
            union = union.insert(flag);
        }

        // ASSERT
        assert_eq!(union.bits(), 0x3fff_ffff);
    }

    #[test]
    fn empty_set_ignores_every_flag() {
        // ARRANGE
        let set = Capabilities::empty();
        let flag = Capabilities::LANDLOCK;

        // ACT
        let contained = set.contains(flag);
        let intersecting = set.intersects(flag);
        let empty = set.is_empty();

        // ASSERT
        assert!(!contained);
        assert!(!intersecting);
        assert!(empty);
        assert!(set.contains(Capabilities::empty()));
    }

    #[test]
    fn union_tracks_what_was_inserted() {
        // ARRANGE
        let set = Capabilities::MOUNT_NS.insert(Capabilities::SECCOMP);

        // ACT
        let both = set.contains(Capabilities::MOUNT_NS.insert(Capabilities::SECCOMP));
        let has_mount = set.intersects(Capabilities::MOUNT_NS);
        let has_cgroup = set.intersects(Capabilities::CGROUP_V2);
        let subset = set.contains(Capabilities::MOUNT_NS);
        let missing = set.contains(Capabilities::MOUNT_NS.insert(Capabilities::CGROUP_V2));
        let empty = set.is_empty();

        // ASSERT
        assert!(both);
        assert!(has_mount);
        assert!(!has_cgroup);
        assert!(subset);
        assert!(!missing);
        assert!(!empty);
    }

    #[test]
    fn display_of_empty_set_is_none() {
        // ARRANGE
        let set = Capabilities::empty();

        // ACT
        let displayed = set.to_string();

        // ASSERT
        assert_eq!(displayed, "none");
    }

    #[test]
    fn display_lists_every_set_bit() {
        // ARRANGE
        let set = Capabilities::MOUNT_NS.insert(Capabilities::PID_NS);

        // ACT
        let displayed = set.to_string();

        // ASSERT
        assert_eq!(displayed, "MOUNT_NS, PID_NS");
    }
}
