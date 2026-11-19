//! Protocol-independent regression coverage for the draft registry contract.

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::time::Instant;

    use coordination::error::Failure;
    use coordination::inventory::{Limits, Snapshot, Vm, VmState};
    use coordination::registry::{AuthenticatedPeer, Registry};
    use uuid::Uuid;

    fn setup() -> (Registry, AuthenticatedPeer) {
        let peer = AuthenticatedPeer {
            domain: Uuid::from_u128(1),
            node: Uuid::from_u128(2),
            fingerprint: [1; 32],
        };
        let mut registry = Registry::new(peer.domain, Limits::new(3, 2, 8).unwrap()).unwrap();
        registry.approve(peer.node, peer.fingerprint).unwrap();
        (registry, peer)
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            hostname: "host".to_owned(),
            vms: vec![Vm {
                id: Uuid::from_u128(100),
                name: "vm".to_owned(),
                state: VmState::Running,
            }],
        }
    }

    #[test]
    fn three_hosts_keep_independent_inventory_when_one_disconnects() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let mut sessions = Vec::new();
        registry.approve(Uuid::from_u128(3), [3; 32]).unwrap();
        registry.approve(Uuid::from_u128(4), [4; 32]).unwrap();
        for (node, fingerprint) in [
            (peer.node, peer.fingerprint),
            (Uuid::from_u128(3), [3; 32]),
            (Uuid::from_u128(4), [4; 32]),
        ] {
            let session = registry
                .connect(AuthenticatedPeer {
                    node,
                    fingerprint,
                    ..peer
                })
                .unwrap();
            registry.report(session, 0, snapshot()).unwrap();
            sessions.push(session);
        }

        // ACT
        registry.disconnect(sessions[0]).unwrap();

        // ASSERT
        assert_eq!(registry.nodes().count(), 3);
        for (id, node) in registry.nodes() {
            assert_eq!(
                node.is_stale(Instant::now(), Duration::from_secs(60)),
                *id == peer.node
            );
            assert_eq!(node.observation().unwrap().snapshot(), &snapshot());
        }
    }

    #[test]
    fn rejected_connections_do_not_supersede_an_approved_session() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        let cases = [
            (
                AuthenticatedPeer {
                    domain: Uuid::from_u128(9),
                    ..peer
                },
                Failure::DomainMismatch,
            ),
            (
                AuthenticatedPeer {
                    node: Uuid::from_u128(9),
                    ..peer
                },
                Failure::UnknownNode,
            ),
            (
                AuthenticatedPeer {
                    fingerprint: [9; 32],
                    ..peer
                },
                Failure::CredentialMismatch,
            ),
        ];

        // ACT
        for (unapproved, error) in cases {
            assert_eq!(registry.connect(unapproved), Err(error));
        }

        // ASSERT
        registry.report(session, 0, snapshot()).unwrap();
        assert_eq!(registry.nodes().count(), 1);
    }

    #[test]
    fn reconnect_invalidates_old_reports_and_close_callbacks() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let old = registry.connect(peer).unwrap();
        registry.report(old, 99, snapshot()).unwrap();

        // ACT
        let current = registry.connect(peer).unwrap();

        // ASSERT
        let node = registry.node(peer.node).unwrap();
        assert!(node.is_stale(Instant::now(), Duration::from_secs(60)));
        assert_eq!(node.observation().unwrap().sequence(), 99);
        assert_eq!(
            registry.report(old, 100, snapshot()),
            Err(Failure::StaleSession)
        );
        assert_eq!(registry.disconnect(old), Err(Failure::StaleSession));
        registry.report(current, 0, snapshot()).unwrap();
        assert!(
            !registry
                .node(peer.node)
                .unwrap()
                .is_stale(Instant::now(), Duration::from_secs(60))
        );
    }

    #[test]
    fn repeated_or_reordered_reports_do_not_refresh_inventory() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, 5, snapshot()).unwrap();
        let received = registry
            .node(peer.node)
            .unwrap()
            .observation()
            .unwrap()
            .received_at();

        // ACT
        for sequence in [5, 4, 0] {
            assert_eq!(
                registry.report(
                    session,
                    sequence,
                    Snapshot {
                        hostname: "old".to_owned(),
                        vms: vec![]
                    }
                ),
                Err(Failure::StaleReport)
            );
        }

        // ASSERT
        let observation = registry.node(peer.node).unwrap().observation().unwrap();
        assert_eq!(observation.snapshot(), &snapshot());
        assert_eq!(observation.received_at(), received);
        assert_eq!(observation.sequence(), 5);
    }

    #[test]
    fn invalid_snapshots_preserve_the_previous_report_and_sequence() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, 0, snapshot()).unwrap();
        let vm = snapshot().vms.remove(0);
        let cases = [
            (
                Snapshot {
                    vms: vec![vm.clone(); 3],
                    ..snapshot()
                },
                Failure::SnapshotTooLarge,
            ),
            (
                Snapshot {
                    vms: vec![vm.clone(); 2],
                    ..snapshot()
                },
                Failure::DuplicateVm,
            ),
            (
                Snapshot {
                    hostname: String::new(),
                    ..snapshot()
                },
                Failure::InvalidName,
            ),
            (
                Snapshot {
                    hostname: "123456789".to_owned(),
                    ..snapshot()
                },
                Failure::InvalidName,
            ),
            (
                Snapshot {
                    vms: vec![Vm {
                        name: "ééééé".to_owned(),
                        ..vm.clone()
                    }],
                    ..snapshot()
                },
                Failure::InvalidName,
            ),
            (
                Snapshot {
                    vms: vec![Vm {
                        name: String::new(),
                        ..vm.clone()
                    }],
                    ..snapshot()
                },
                Failure::InvalidName,
            ),
            (
                Snapshot {
                    vms: vec![Vm {
                        id: Uuid::nil(),
                        ..vm
                    }],
                    ..snapshot()
                },
                Failure::InvalidIdentity,
            ),
        ];

        // ACT
        for (invalid, error) in cases {
            assert_eq!(registry.report(session, 1, invalid), Err(error));
        }

        // ASSERT
        let observation = registry.node(peer.node).unwrap().observation().unwrap();
        assert_eq!(observation.snapshot(), &snapshot());
        assert_eq!(observation.sequence(), 0);
        registry.report(session, 1, snapshot()).unwrap();
    }

    #[test]
    fn a_full_snapshot_removes_vms_missing_from_the_new_report() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, 0, snapshot()).unwrap();

        // ACT
        registry
            .report(
                session,
                1,
                Snapshot {
                    vms: vec![],
                    ..snapshot()
                },
            )
            .unwrap();

        // ASSERT
        assert_eq!(
            registry
                .node(peer.node)
                .unwrap()
                .observation()
                .unwrap()
                .snapshot()
                .vms,
            Vec::<Vm>::new()
        );
    }

    #[test]
    fn revocation_invalidates_sessions_without_forgetting_inventory() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, 0, snapshot()).unwrap();

        // ACT
        registry.revoke(peer.node).unwrap();

        // ASSERT
        assert_eq!(registry.connect(peer), Err(Failure::RevokedNode));
        assert_eq!(
            registry.report(session, 1, snapshot()),
            Err(Failure::StaleSession)
        );
        assert_eq!(
            registry.approve(peer.node, peer.fingerprint),
            Err(Failure::AlreadyRegistered)
        );
        assert_eq!(
            registry.rotate_credential(peer.node, [2; 32]),
            Err(Failure::RevokedNode)
        );
        let node = registry.node(peer.node).unwrap();
        assert!(node.is_revoked());
        assert!(node.is_stale(Instant::now(), Duration::from_secs(60)));
        assert_eq!(node.observation().unwrap().snapshot(), &snapshot());
    }

    #[test]
    fn reclaiming_revoked_observations_preserves_other_nodes_and_revocation() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let other = AuthenticatedPeer {
            node: Uuid::from_u128(3),
            fingerprint: [3; 32],
            ..peer
        };
        registry.approve(other.node, other.fingerprint).unwrap();
        let revoked = registry.connect(peer).unwrap();
        let disconnected = registry.connect(other).unwrap();
        registry.report(revoked, 0, snapshot()).unwrap();
        registry.report(disconnected, 0, snapshot()).unwrap();
        registry.disconnect(disconnected).unwrap();
        registry.revoke(peer.node).unwrap();

        // ACT
        registry.discard_revoked_observations();
        registry.discard_revoked_observations();

        // ASSERT
        let node = registry.node(peer.node).unwrap();
        assert!(node.is_revoked());
        assert!(node.observation().is_none());
        assert_eq!(registry.connect(peer), Err(Failure::RevokedNode));
        assert_eq!(
            registry.report(revoked, 1, snapshot()),
            Err(Failure::StaleSession)
        );
        assert_eq!(
            registry
                .node(other.node)
                .unwrap()
                .observation()
                .unwrap()
                .snapshot(),
            &snapshot()
        );
    }

    #[test]
    fn credential_rotation_preserves_identity_and_rejects_the_old_session() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let old = registry.connect(peer).unwrap();
        registry.report(old, 0, snapshot()).unwrap();

        // ACT
        registry.rotate_credential(peer.node, [2; 32]).unwrap();

        // ASSERT
        assert_eq!(registry.connect(peer), Err(Failure::CredentialMismatch));
        assert_eq!(
            registry.report(old, 1, snapshot()),
            Err(Failure::StaleSession)
        );
        let session = registry
            .connect(AuthenticatedPeer {
                fingerprint: [2; 32],
                ..peer
            })
            .unwrap();
        registry.report(session, 0, snapshot()).unwrap();
        assert_eq!(registry.nodes().count(), 1);
        assert_eq!(
            registry
                .node(peer.node)
                .unwrap()
                .observation()
                .unwrap()
                .snapshot(),
            &snapshot()
        );
    }

    #[test]
    fn credentials_cannot_be_bound_to_two_nodes() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let other = Uuid::from_u128(3);
        registry.approve(other, [2; 32]).unwrap();
        let session = registry.connect(peer).unwrap();

        // ACT
        assert_eq!(
            registry.approve(Uuid::from_u128(4), peer.fingerprint),
            Err(Failure::CredentialInUse)
        );
        assert_eq!(
            registry.rotate_credential(peer.node, [2; 32]),
            Err(Failure::CredentialInUse)
        );

        // ASSERT
        registry.report(session, 0, snapshot()).unwrap();
        assert_eq!(registry.nodes().count(), 2);
    }

    #[test]
    fn restored_registrations_do_not_restore_old_sessions() {
        // ARRANGE
        let (mut original, peer) = setup();
        let old = original.connect(peer).unwrap();
        original.report(old, 0, snapshot()).unwrap();
        let (mut restored, _) = setup();
        let current = restored.connect(peer).unwrap();

        // ACT
        assert_eq!(
            restored.report(old, 1, snapshot()),
            Err(Failure::StaleSession)
        );
        assert_eq!(restored.disconnect(old), Err(Failure::StaleSession));

        // ASSERT
        assert_ne!(old, current);
        assert!(restored.node(peer.node).unwrap().observation().is_none());
        restored.report(current, 0, snapshot()).unwrap();
    }

    #[test]
    fn freshness_expires_at_the_boundary_without_agent_clock_input() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, 0, snapshot()).unwrap();
        let node = registry.node(peer.node).unwrap();
        let received = node.observation().unwrap().received_at();
        let max_age = Duration::from_secs(30);

        // ACT & ASSERT
        assert!(!node.is_stale(received, max_age));
        assert!(!node.is_stale(received + Duration::from_secs(29), max_age));
        assert!(node.is_stale(received + max_age, max_age));
        assert!(node.is_stale(
            received.checked_sub(Duration::from_secs(1)).unwrap(),
            max_age
        ));
    }

    #[test]
    fn report_sequence_never_wraps_within_a_session() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        registry.report(session, u64::MAX, snapshot()).unwrap();

        // ACT
        assert_eq!(
            registry.report(session, 0, snapshot()),
            Err(Failure::StaleReport)
        );

        // ASSERT
        let next = registry.connect(peer).unwrap();
        registry.report(next, 0, snapshot()).unwrap();
    }

    #[test]
    fn revoked_registrations_remain_bounded_and_cannot_be_reenrolled() {
        // ARRANGE
        let domain = Uuid::from_u128(1);
        let node = Uuid::from_u128(2);
        let mut registry = Registry::new(domain, Limits::new(1, 1, 8).unwrap()).unwrap();
        registry.approve(node, [1; 32]).unwrap();
        registry.revoke(node).unwrap();

        // ACT & ASSERT
        assert_eq!(
            registry.approve(Uuid::from_u128(3), [2; 32]),
            Err(Failure::NodeLimit)
        );
        assert_eq!(
            registry.approve(node, [1; 32]),
            Err(Failure::AlreadyRegistered)
        );
        assert_eq!(registry.nodes().count(), 1);
    }

    #[test]
    fn nil_identities_and_zero_limits_are_rejected() {
        // ARRANGE
        let (mut registry, _) = setup();

        // ACT & ASSERT
        for limits in [(0, 1, 1), (1, 0, 1), (1, 1, 0)] {
            assert!(matches!(
                Limits::new(limits.0, limits.1, limits.2),
                Err(Failure::InvalidLimits)
            ));
        }
        assert!(matches!(
            Registry::new(Uuid::nil(), Limits::new(1, 1, 8).unwrap()),
            Err(Failure::InvalidIdentity)
        ));
        assert_eq!(
            registry.approve(Uuid::nil(), [1; 32]),
            Err(Failure::InvalidIdentity)
        );
    }

    #[test]
    fn inventory_accepts_exact_configured_limits() {
        // ARRANGE
        let (mut registry, peer) = setup();
        let session = registry.connect(peer).unwrap();
        let report = Snapshot {
            hostname: "12345678".to_owned(),
            vms: vec![
                Vm {
                    id: Uuid::from_u128(100),
                    name: "12345678".to_owned(),
                    state: VmState::Running,
                },
                Vm {
                    id: Uuid::from_u128(101),
                    name: "87654321".to_owned(),
                    state: VmState::Stopped,
                },
            ],
        };

        // ACT
        registry.report(session, 0, report.clone()).unwrap();

        // ASSERT
        assert_eq!(
            registry
                .node(peer.node)
                .unwrap()
                .observation()
                .unwrap()
                .snapshot(),
            &report
        );
    }
}
