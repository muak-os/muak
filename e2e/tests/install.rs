use std::collections::HashMap;
use std::time::Duration;

use e2e::artifacts::Artifacts;
use e2e::cli::Cli;
use e2e::vm::TestFixture;
use e2e::{assert_success, assert_success_insecure};

const DEFAULT_REGISTRY: &str = "ghcr.io/sawangg";
const DEFAULT_TAG: &str = "latest";

fn install_image() -> String {
    let registry = std::env::var("REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_owned());
    let tag = std::env::var("TAG").unwrap_or_else(|_| DEFAULT_TAG.to_owned());
    format!("{registry}/installer:{tag}")
}

#[tokio::test]
async fn install_and_verify_mtls() {
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");

    let fixture = TestFixture::boot_install(&artifacts)
        .await
        .expect("failed to boot install VM");

    fixture
        .vm
        .wait_ready(Duration::from_secs(60))
        .await
        .expect("maintenance-mode apid did not become ready");

    let cli =
        Cli::new(&artifacts.cli_bin, fixture.vm.host_port).expect("failed to create CLI driver");

    let config_file = cli
        .generate_config(&HashMap::from([
            ("system.secureboot", toml::Value::Boolean(false)),
            (
                "system.disk",
                toml::Value::String("/dev/nvme0n1".to_owned()),
            ),
            ("system.image", toml::Value::String(install_image())),
        ]))
        .await
        .expect("failed to generate install config");

    let stdout = tokio::time::timeout(
        Duration::from_secs(60),
        assert_success_insecure!(
            cli,
            [
                "install",
                "--config",
                &config_file.path().display().to_string(),
            ]
        ),
    )
    .await
    .expect("install timed out after 5 minutes")
    .expect("muakctl install failed");

    assert!(
        stdout.contains("Installation verified successfully") || stdout.contains("installed"),
        "unexpected install output: {stdout}"
    );

    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")
        .expect("installed-boot marker not found in serial log");

    let disks = assert_success!(cli, ["disks"])
        .await
        .expect("authenticated muakctl disks failed");

    assert!(
        disks.contains("nvme0n1"),
        "expected nvme0n1 in disk listing, got: {disks}"
    );
}
