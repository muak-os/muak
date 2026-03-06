mod common;

use std::collections::HashMap;

use common::boot_and_install;
use e2e::artifacts::Artifacts;
use e2e::assert_success;

#[tokio::test]
async fn install() {
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");

    let (fixture, cli) = boot_and_install(
        &artifacts,
        HashMap::from([("system.secureboot", toml::Value::Boolean(false))]),
    )
    .await;

    let disks = assert_success!(cli, ["disks"])
        .await
        .expect("authenticated muakctl disks failed");

    assert!(
        disks.contains("nvme0n1"),
        "expected nvme0n1 in disk listing, got: {disks}"
    );

    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")
        .expect("installed-boot marker not found in serial log");
}

#[tokio::test]
async fn install_secureboot() {
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");

    let (fixture, cli) = boot_and_install(&artifacts, HashMap::new()).await;

    let security = assert_success!(cli, ["security", "state"])
        .await
        .expect("authenticated muakctl security state failed");

    assert!(
        security.contains("Secure Boot: Enabled"),
        "expected Secure Boot to be enabled, got: {security}"
    );

    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")
        .expect("installed-boot marker not found in serial log");
}
