//! Integration tests for policy resolution.

#![cfg(feature = "spawn")]

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sandbox::credentials::Caps;
    use sandbox::error::{Mechanism, SandboxError};
    use sandbox::limits::{Limit, Limits};
    use sandbox::mount::{Entry, Flags, Source};
    use sandbox::namespace::Set;
    use sandbox::policy::Policy;
    use sandbox::probe::Capabilities;

    #[test]
    fn plain_policy_needs_nothing() {
        // ARRANGE
        let policy = Policy::new();

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        assert!(result.is_ok(), "plain policy failed: {result:?}");
    }

    #[test]
    fn capabilities_require_the_cap_facilities() {
        // ARRANGE
        let policy = Policy::new().caps(Caps::none());

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        if cfg!(feature = "caps") {
            assert!(matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Caps,
                    ..
                })
            ));
        } else {
            assert!(matches!(
                result,
                Err(SandboxError::NotCompiled {
                    mechanism: Mechanism::Caps
                })
            ));
        }
    }

    #[cfg(feature = "caps")]
    #[test]
    fn non_root_identity_with_capabilities_is_unsupported() {
        use sandbox::credentials::Id;

        // ARRANGE
        let policy = Policy::new().id(Id::new(1000, 1000)).caps(Caps::all());
        let detected = Capabilities::CAPSET.insert(Capabilities::CAP_BOUNDING);

        // ACT
        let result = policy.resolve(&detected);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Caps,
                    ..
                })
            ),
            "ambient capabilities are not implemented yet: {result:?}"
        );
    }

    #[test]
    fn limits_report_not_compiled_when_absent() {
        // ARRANGE
        let policy = Policy::new().limits(Limits::new().nofile(Limit::Value(64)));

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        if cfg!(feature = "limits") {
            assert!(result.is_ok(), "limits failed: {result:?}");
        } else {
            assert!(matches!(
                result,
                Err(SandboxError::NotCompiled {
                    mechanism: Mechanism::Limits
                })
            ));
        }
    }

    #[test]
    fn namespaces_require_the_flag_that_names_them() {
        // ARRANGE
        let policy = Policy::new().namespaces(Set::MOUNT.insert(Set::NETWORK).insert(Set::IPC));

        // ACT
        let needs = policy.requirements().needs();

        // ASSERT
        assert!(needs.contains(Capabilities::MOUNT_NS));
        assert!(needs.contains(Capabilities::NET_NS));
        assert!(needs.contains(Capabilities::IPC_NS));
        assert!(!needs.contains(Capabilities::UTS_NS));
        assert!(!needs.contains(Capabilities::CGROUP_NS));
    }

    #[test]
    fn mounts_require_the_mount_namespace_and_their_filesystems() {
        // ARRANGE
        let entries = [
            Entry {
                source: Source::Tmpfs,
                target: Path::new("/root"),
                data: None,
                flags: Flags::new(),
            },
            Entry {
                source: Source::Proc,
                target: Path::new("/root/proc"),
                data: None,
                flags: Flags::new(),
            },
            Entry {
                source: Source::Bind(Path::new("/usr")),
                target: Path::new("/root/usr"),
                data: None,
                flags: Flags::new(),
            },
        ];
        let policy = Policy::new().mounts(&entries);

        // ACT
        let needs = policy.requirements().needs();

        // ASSERT
        assert!(needs.contains(Capabilities::MOUNT_NS));
        assert!(needs.contains(Capabilities::TMPFS));
        assert!(needs.contains(Capabilities::PROC));
        assert!(!needs.contains(Capabilities::SYSFS));
        assert!(!needs.contains(Capabilities::DEVPTS));
    }

    #[test]
    fn an_invalid_policy_is_unsupported_before_it_is_not_compiled() {
        // ARRANGE
        // Resolving the requirements first would report the missing namespace mechanism instead of the policy shape.
        let entries = [Entry {
            source: Source::Tmpfs,
            target: Path::new("/root"),
            data: None,
            flags: Flags::new(),
        }];
        let policy = Policy::new().mounts(&entries);

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Mounts,
                    ..
                })
            ),
            "the policy check must run before the requirement: {result:?}"
        );
    }

    #[test]
    fn a_root_filesystem_requires_the_mount_namespace_and_pivot_root() {
        // ARRANGE
        let policy = Policy::new().rootfs(Path::new("/srv/root"));

        // ACT
        let needs = policy.requirements().needs();

        // ASSERT
        assert!(needs.contains(Capabilities::MOUNT_NS));
        assert!(needs.contains(Capabilities::PIVOT_ROOT));
    }

    #[test]
    fn inherited_limits_need_no_mechanism() {
        // ARRANGE
        let policy = Policy::new().limits(Limits::new());

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        assert!(result.is_ok(), "inherited limits failed: {result:?}");
    }
}
