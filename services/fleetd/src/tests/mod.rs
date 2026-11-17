//! Persistence and authorization contract tests without VM boot requirements.

mod network;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use fleet_api::fleet::{NodeSnapshot, RegisterNodeRequest, ReportInventoryRequest, VmSummary};
use tempfile::tempdir;
use uuid::Uuid;

use crate::manager::Manager;
use crate::storage::Store;

fn request(manager: &Manager, node: &str) -> RegisterNodeRequest {
    RegisterNodeRequest {
        domain_id: manager.info().unwrap().domain_id,
        node_id: node.to_owned(),
        hostname: "worker".to_owned(),
    }
}

fn report(session: &str, sequence: u64) -> ReportInventoryRequest {
    ReportInventoryRequest {
        session_id: session.to_owned(),
        sequence,
        snapshot: Some(NodeSnapshot {
            hostname: "worker".to_owned(),
            vms: vec![VmSummary {
                vm_id: Uuid::from_u128(42).to_string(),
                name: "example".to_owned(),
                state: 3,
            }],
        }),
    }
}

#[test]
fn approval_reconnect_restart_rotation_and_revocation() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let path = dir.path().join("manager.json");
    let first_credential = "01".repeat(32);
    let next_credential = "02".repeat(32);
    let admin = "ff".repeat(32);
    let node = Uuid::new_v4().to_string();
    let mut manager = Manager::open(&path, 4, 60, String::new()).unwrap();
    let domain = manager.info().unwrap().domain_id;
    // ACT: registration is inert until an exact approval.
    assert!(
        manager
            .register(&first_credential, request(&manager, &node))
            .unwrap()
            .pending
    );
    let _error = manager
        .approve(&admin, &node, &next_credential)
        .expect_err("operation must fail");
    manager.approve(&admin, &node, &first_credential).unwrap();
    let old = manager
        .register(&first_credential, request(&manager, &node))
        .unwrap()
        .session_id;
    manager.report(&first_credential, report(&old, 0)).unwrap();
    assert!(!manager.list().unwrap().first().unwrap().stale);
    let new = manager
        .register(&first_credential, request(&manager, &node))
        .unwrap()
        .session_id;
    assert!(manager.list().unwrap().first().unwrap().stale);
    let _error = manager
        .report(&first_credential, report(&old, 1))
        .expect_err("operation must fail");
    let _error = manager
        .report(&next_credential, report(&new, 0))
        .expect_err("operation must fail");
    manager.report(&first_credential, report(&new, 0)).unwrap();
    let _error = manager
        .report(&first_credential, report(&new, 0))
        .expect_err("operation must fail");
    drop(manager);
    // ASSERT: restart preserves authority, never first_credential live session or freshness.
    let mut manager = Manager::open(&path, 4, 60, String::new()).unwrap();
    assert_eq!(manager.info().unwrap().domain_id, domain);
    let _error = manager
        .report(&first_credential, report(&new, 1))
        .expect_err("operation must fail");
    assert!(manager.list().unwrap().first().unwrap().stale);
    assert!(manager.list().unwrap().first().unwrap().snapshot.is_none());
    manager.rotate(&admin, &node, &next_credential).unwrap();
    let _error = manager
        .register(&first_credential, request(&manager, &node))
        .expect_err("operation must fail");
    let rotated = manager
        .register(&next_credential, request(&manager, &node))
        .unwrap()
        .session_id;
    manager
        .report(&next_credential, report(&rotated, 0))
        .unwrap();
    manager.revoke(&admin, &node).unwrap();
    let _error = manager
        .report(&next_credential, report(&rotated, 1))
        .expect_err("operation must fail");
    drop(manager);
    let mut manager = Manager::open(&path, 4, 60, String::new()).unwrap();
    let _error = manager
        .register(&next_credential, request(&manager, &node))
        .expect_err("operation must fail");
    assert_eq!(manager.list().unwrap().len(), 1);
    assert_eq!(manager.list().unwrap().first().unwrap().status, "revoked");
}

#[test]
fn bounded_pending_records_and_identity_binding() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let mut manager = Manager::open(&dir.path().join("state"), 1, 60, String::new()).unwrap();
    let first_credential = "01".repeat(32);
    let node = Uuid::new_v4().to_string();
    // ACT
    let mut wrong_domain = request(&manager, &node);
    wrong_domain.domain_id = Uuid::new_v4().to_string();
    let _error = manager
        .register(&first_credential, wrong_domain)
        .expect_err("operation must fail");
    manager
        .register(&first_credential, request(&manager, &node))
        .unwrap();
    // ASSERT
    let _error = manager
        .register(&"02".repeat(32), request(&manager, &node))
        .expect_err("operation must fail");
    let _error = manager
        .register(
            &"02".repeat(32),
            request(&manager, &Uuid::new_v4().to_string()),
        )
        .expect_err("operation must fail");
    assert_eq!(manager.list().unwrap().len(), 1);
    let _error = crate::model::identity(&node.to_uppercase()).expect_err("operation must fail");
}

#[test]
fn exclusive_state_and_corruption_fail_closed() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let path = dir.path().join("state");
    let mut store = Store::open(&path, || Ok(1_u64)).unwrap();
    // ACT
    let _error = Store::open(&path, || Ok(2_u64))
        .map(|_store| ())
        .expect_err("operation must fail");
    store.commit(3).unwrap();
    drop(store);
    // ASSERT
    assert_eq!(
        *Store::open(&path, || Ok(2_u64)).unwrap().data().unwrap(),
        3
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&path, b"truncated").unwrap();
    let _error = Store::open(&path, || Ok(4_u64))
        .map(|_store| ())
        .expect_err("operation must fail");
}

#[test]
fn failed_commit_disables_further_operations() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let path = dir.path().join("state");
    let mut store = Store::open(&path, || Ok(1_u64)).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir_all(&path).unwrap();
    // ACT
    let result = store.commit(2);
    // ASSERT: first_credential rename failure cannot leave an apparently usable old policy.
    let _error = result.expect_err("operation must fail");
    let _error = store.data().expect_err("operation must fail");
    let _error = store.commit(3).expect_err("operation must fail");
}

#[test]
fn unsupported_state_is_preserved_for_explicit_recovery() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let path = dir.path().join("state");
    let mut policy = crate::model::Policy::fresh();
    policy.version = 99;
    let bytes = serde_json::to_vec(&policy).unwrap();
    fs::write(&path, &bytes).unwrap();
    // ACT
    let _error = Manager::open(&path, 4, 60, String::new())
        .map(|_manager| ())
        .unwrap_err();
    // ASSERT
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn current_observation_supplies_the_hostname_after_a_local_rename() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let mut manager = Manager::open(&dir.path().join("state"), 1, 60, String::new()).unwrap();
    let credential = "01".repeat(32);
    let node = Uuid::new_v4().to_string();
    manager
        .register(&credential, request(&manager, &node))
        .unwrap();
    manager.approve(&credential, &node, &credential).unwrap();
    let session = manager
        .register(&credential, request(&manager, &node))
        .unwrap()
        .session_id;
    let mut report = report(&session, 0);
    report.snapshot.as_mut().unwrap().hostname = "renamed-worker".to_owned();
    // ACT
    manager.report(&credential, report).unwrap();
    // ASSERT
    assert_eq!(
        manager.list().unwrap().first().unwrap().hostname,
        "renamed-worker"
    );
}

#[test]
fn aggregate_inventory_limit_preserves_previously_accepted_reports() {
    // ARRANGE
    let dir = tempdir().unwrap();
    let mut manager = Manager::open(&dir.path().join("state"), 10, 60, String::new()).unwrap();
    let vms: Vec<VmSummary> = (1_u128..=1024)
        .map(|number| VmSummary {
            vm_id: Uuid::from_u128(number).to_string(),
            name: "x".repeat(255),
            state: 3,
        })
        .collect();
    let mut rejected = false;
    // ACT
    for number in 0_u8..10 {
        let credential = format!("{number:064x}");
        let node = Uuid::new_v4().to_string();
        manager
            .register(&credential, request(&manager, &node))
            .unwrap();
        manager.approve(&credential, &node, &credential).unwrap();
        let session = manager
            .register(&credential, request(&manager, &node))
            .unwrap()
            .session_id;
        let report = ReportInventoryRequest {
            session_id: session,
            sequence: 0,
            snapshot: Some(NodeSnapshot {
                hostname: "worker".to_owned(),
                vms: vms.clone(),
            }),
        };
        if manager.report(&credential, report).is_err() {
            rejected = true;
            break;
        }
    }
    // ASSERT: only the over-budget report fails, without erasing prior inventory.
    let nodes = manager.list().unwrap();
    assert!(rejected);
    assert_eq!(
        nodes.iter().filter(|node| node.snapshot.is_none()).count(),
        1
    );
    assert!(nodes.iter().any(|node| !node.stale));
}
