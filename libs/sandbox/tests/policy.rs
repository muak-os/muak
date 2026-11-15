//! Integration tests for policy resolution.

#![cfg(feature = "spawn")]

#[cfg(test)]
mod tests {
    use sandbox::credentials::Caps;
    use sandbox::error::{Mechanism, SandboxError};
    use sandbox::limits::{Limit, Limits};
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
    fn inherited_limits_need_no_mechanism() {
        // ARRANGE
        let policy = Policy::new().limits(Limits::new());

        // ACT
        let result = policy.resolve(&Capabilities::empty());

        // ASSERT
        assert!(result.is_ok(), "inherited limits failed: {result:?}");
    }
}
