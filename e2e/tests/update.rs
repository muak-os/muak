mod common;

use std::time::Duration;

use common::{boot_and_install, install_image};
use e2e::artifacts::Artifacts;
use e2e::assert_success;

#[tokio::test]
async fn update() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await;

    // ACT
    let stdout = tokio::time::timeout(
        Duration::from_secs(60),
        assert_success!(cli, ["update", "--image", &install_image()]),
    )
    .await
    .expect("update timed out after 10 minutes")
    .expect("muakctl update failed");

    // ASSERT
    assert!(
        stdout.contains("committed successfully"),
        "expected 'committed successfully' in update output, got: {stdout}"
    );

    fixture
        .vm
        .assert_serial_contains("muak.update_id=")
        .expect("kexec update marker not found in serial log");
}

#[tokio::test]
async fn update_config() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await;

    let image = install_image();
    let update_cfg = cli
        .generate_config(|cfg| {
            cfg.disk.system = "/dev/nvme0n1".to_owned();
            cfg.host.image = image;
        })
        .await
        .expect("failed to generate update config");

    // ACT
    let stdout = tokio::time::timeout(
        Duration::from_secs(60),
        assert_success!(
            cli,
            [
                "update",
                "--config",
                &update_cfg.path().display().to_string(),
            ]
        ),
    )
    .await
    .expect("update --config timed out after 10 minutes")
    .expect("muakctl update --config failed");

    // ASSERT
    assert!(
        stdout.contains("committed successfully"),
        "expected 'committed successfully' in update --config output, got: {stdout}"
    );

    let config_out = assert_success!(cli, ["config", "get"])
        .await
        .expect("muakctl config get failed after update");

    assert!(
        config_out.contains(&install_image()),
        "expected updated image '{}' in config get output, got: {config_out}",
        install_image()
    );

    fixture
        .vm
        .assert_serial_contains("muak.update_id=")
        .expect("kexec update marker not found in serial log");
}

#[tokio::test]
async fn update_config_secureboot() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (fixture, cli) = boot_and_install(&artifacts, |cfg| {
        cfg.host.secureboot = false;
    })
    .await;

    let image = install_image();
    let update_cfg = cli
        .generate_config(|cfg| {
            cfg.disk.system = "/dev/nvme0n1".to_owned();
            cfg.host.image = image;
            cfg.host.secureboot = true;
        })
        .await
        .expect("failed to generate update config");

    // ACT
    let stdout = tokio::time::timeout(
        Duration::from_secs(60),
        assert_success!(
            cli,
            [
                "update",
                "--config",
                &update_cfg.path().display().to_string(),
            ]
        ),
    )
    .await
    .expect("update --config timed out after 10 minutes")
    .expect("muakctl update --config failed");

    // ASSERT
    assert!(
        stdout.contains("committed successfully"),
        "expected 'committed successfully' in update --config output, got: {stdout}"
    );

    let config_out = assert_success!(cli, ["config", "get"])
        .await
        .expect("muakctl config get failed after update");

    assert!(
        config_out.contains(&install_image()),
        "expected updated image '{}' in config get output, got: {config_out}",
        install_image()
    );

    fixture
        .vm
        .assert_serial_contains("muak.update_id=")
        .expect("kexec update marker not found in serial log");

    let security = assert_success!(cli, ["security", "state"])
        .await
        .expect("authenticated muakctl security state failed");

    assert!(
        security.contains("Secure Boot: Pending (firmware reboot required)"),
        "expected Secure Boot to be enabled, got: {security}"
    );
}
