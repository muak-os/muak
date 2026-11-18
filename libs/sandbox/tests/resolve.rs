//! Integration tests for the resolve gate.

#[cfg(test)]
mod tests {
    use sandbox::error::{Mechanism, SandboxError};
    use sandbox::probe::Capabilities;
    use sandbox::resolve::Requirements;

    const ALL: [Capabilities; 27] = [
        Capabilities::MOUNT_NS,
        Capabilities::PID_NS,
        Capabilities::NET_NS,
        Capabilities::IPC_NS,
        Capabilities::UTS_NS,
        Capabilities::USER_NS,
        Capabilities::CGROUP_NS,
        Capabilities::TIME_NS,
        Capabilities::CGROUP_V2,
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
        Capabilities::UNPRIV_USER_NS,
    ];

    #[test]
    fn resolve_accepts_present_requirements() {
        // ARRANGE
        let caps = Capabilities::PIDFD.insert(Capabilities::CLONE3);
        let requirements = Requirements::new()
            .require(Capabilities::empty())
            .require(Capabilities::CLONE3)
            .require(Capabilities::PIDFD);

        // ACT
        let result = requirements.resolve(&caps);

        // ASSERT
        assert!(result.is_ok(), "resolve failed: {result:?}");
    }

    #[test]
    fn uncompiled_mechanism_reports_not_compiled() {
        // ARRANGE
        let caps = Capabilities::CGROUP_V2;
        let requirements = Requirements::new().require(Capabilities::CGROUP_V2);
        assert_eq!(Mechanism::Cgroup.compiled(), cfg!(feature = "cgroupv2"));

        // ACT
        let result = requirements.resolve(&caps);

        // ASSERT
        if cfg!(feature = "cgroupv2") {
            assert!(result.is_ok(), "resolve failed: {result:?}");
        } else {
            assert!(matches!(
                result,
                Err(SandboxError::NotCompiled {
                    mechanism: Mechanism::Cgroup
                })
            ));
        }
    }

    #[test]
    fn absent_kernel_flag_reports_unsupported() {
        // ARRANGE
        let caps = Capabilities::empty();
        let requirements = Requirements::new().require(Capabilities::CLONE3);
        assert_eq!(Mechanism::Spawn.compiled(), cfg!(feature = "spawn"));

        // ACT
        let result = requirements.resolve(&caps);

        // ASSERT
        if cfg!(feature = "spawn") {
            assert!(matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Spawn,
                    detail: "CLONE3"
                })
            ));
        } else {
            assert!(matches!(
                result,
                Err(SandboxError::NotCompiled {
                    mechanism: Mechanism::Spawn
                })
            ));
        }
    }

    // The ordering is only observable in a build that leaves a mechanism out,
    // so a build carrying every mechanism has nothing to tell apart here.
    #[cfg(not(feature = "cgroupv2"))]
    #[test]
    fn not_compiled_wins_over_unsupported() {
        // ARRANGE
        let caps = Capabilities::empty();
        let requirements = Requirements::new().require(Capabilities::CGROUP_V2);

        // ACT
        let result = requirements.resolve(&caps);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::NotCompiled {
                    mechanism: Mechanism::Cgroup
                })
            ),
            "a build problem must be reported before a kernel problem"
        );
    }

    #[test]
    fn missing_reports_absent_compiled_flags() {
        // ARRANGE
        let caps = Capabilities::PID_NS;
        let requirements = Requirements::new()
            .require(Capabilities::MOUNT_NS)
            .require(Capabilities::PID_NS)
            .require(Capabilities::CLONE3);

        // ACT
        let missing = requirements.missing(&caps);

        // ASSERT
        let mut expected = Capabilities::empty();
        if cfg!(feature = "namespaces") {
            expected = expected.insert(Capabilities::MOUNT_NS);
        }
        if cfg!(feature = "spawn") {
            expected = expected.insert(Capabilities::CLONE3);
        }
        assert_eq!(missing, expected);
    }

    #[test]
    fn flags_yield_one_entry_per_bit_in_order() {
        // ARRANGE
        let set = Capabilities::PID_NS.insert(Capabilities::MOUNT_NS);

        // ACT
        let flags: Vec<_> = set.flags().collect();

        // ASSERT
        assert_eq!(flags, [Capabilities::MOUNT_NS, Capabilities::PID_NS]);
    }

    #[test]
    fn every_flag_round_trips_through_name_and_mechanism() {
        // ARRANGE
        let mut names = [""; 27];
        let mut mechanisms = [None; 27];
        let mut single = [false; 27];

        // ACT
        for (position, flag) in ALL.into_iter().enumerate() {
            names[position] = flag.name().unwrap_or("");
            mechanisms[position] = flag.mechanism();
            let mut flags = flag.flags();
            single[position] = flags.next() == Some(flag) && flags.next().is_none();
        }

        // ASSERT
        for (position, flag) in ALL.into_iter().enumerate() {
            assert!(!names[position].is_empty(), "flag {position} has no name");
            assert!(
                mechanisms[position].is_some(),
                "flag {} has no mechanism",
                names[position]
            );
            assert!(
                single[position],
                "flag {} does not yield exactly itself",
                names[position]
            );
            assert!(
                !names[..position].contains(&names[position]),
                "flag {position} repeats name {}",
                names[position]
            );
            assert_eq!(flag.flags().count(), 1);
        }

        let combined = Capabilities::MOUNT_NS.insert(Capabilities::PID_NS);
        assert_eq!(combined.name(), None);
        assert_eq!(combined.mechanism(), None);
    }
}
