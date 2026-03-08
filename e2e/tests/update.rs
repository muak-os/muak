mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{boot_and_install, install_image};
use e2e::artifacts::Artifacts;
use e2e::assert_success;

#[tokio::test]
async fn update() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (fixture, cli) = boot_and_install(&artifacts, HashMap::new()).await;

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
    let (fixture, cli) = boot_and_install(&artifacts, HashMap::new()).await;

    let update_cfg = cli
        .generate_config(&HashMap::from([
            (
                "system.disk",
                toml::Value::String("/dev/nvme0n1".to_owned()),
            ),
            ("system.image", toml::Value::String(install_image())),
        ]))
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
    let (fixture, cli) = boot_and_install(
        &artifacts,
        HashMap::from([("system.secureboot", toml::Value::Boolean(false))]),
    )
    .await;

    let update_cfg = cli
        .generate_config(&HashMap::from([
            (
                "system.disk",
                toml::Value::String("/dev/nvme0n1".to_owned()),
            ),
            ("system.image", toml::Value::String(install_image())),
            ("system.secureboot", toml::Value::Boolean(true)),
        ]))
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

#[tokio::test]
async fn update_rejects_image_and_config_together() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (_, cli) = boot_and_install(&artifacts, HashMap::new()).await;

    let dummy_cfg = cli
        .generate_config(&HashMap::from([
            (
                "system.disk",
                toml::Value::String("/dev/nvme0n1".to_owned()),
            ),
            ("system.image", toml::Value::String(install_image())),
        ]))
        .await
        .expect("failed to generate config for rejection test");

    // ACT
    let output = cli
        .run(
            [
                "update",
                "--image",
                &install_image(),
                "--config",
                &dummy_cfg.path().display().to_string(),
            ],
            false,
        )
        .await
        .expect("failed to execute muakctl");

    // ASSERT
    assert!(
        !output.status.success(),
        "expected non-zero exit when --image and --config are both given"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "expected 'mutually exclusive' in stderr, got: {stderr}"
    );
}

#[tokio::test]
async fn update_rejects_no_source() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (_, cli) = boot_and_install(&artifacts, HashMap::new()).await;

    // ACT
    let output = cli
        .run(["update"], false)
        .await
        .expect("failed to execute muakctl");

    // ASSERT
    assert!(
        !output.status.success(),
        "expected non-zero exit when neither --image nor --config is given"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--image") || stderr.contains("--config"),
        "expected mention of --image or --config in stderr, got: {stderr}"
    );
}
