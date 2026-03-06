use std::time::Duration;

use e2e::artifacts::Artifacts;
use e2e::cli::Cli;
use e2e::vm::{APID_GUEST_PORT, TestFixture};
use e2e::{assert_success, assert_success_insecure};
use tempfile::NamedTempFile;

const INSTALL_DISK_GIB: u64 = 5;
const DEFAULT_REGISTRY: &str = "ghcr.io/sawangg";
const DEFAULT_TAG: &str = "latest";

fn install_image() -> String {
    let registry = std::env::var("REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_owned());
    let tag = std::env::var("TAG").unwrap_or_else(|_| DEFAULT_TAG.to_owned());
    format!("{registry}/installer:{tag}")
}

/// Minimum TOML config for a QEMU test install: no secureboot (no SB key enrollment in the
/// emulated firmware), NVMe target disk, default installer image.
fn install_config(port: u16) -> String {
    let image = install_image();
    format!(
        "[system]\nname = \"muak\"\ndisk = \"/dev/nvme0n1\"\nimage = \"{image}\"\nsecureboot = false\nport = {port}\n"
    )
}

/// Full install flow: boots in maintenance mode, installs to NVMe, waits for the automatic
/// reboot, then verifies the installed system is reachable with mTLS.
#[tokio::test]
async fn install_and_verify_mtls() {
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");

    let mut fixture = TestFixture::boot_install(&artifacts, INSTALL_DISK_GIB)
        .await
        .expect("failed to boot install VM");

    fixture
        .vm
        .wait_ready(Duration::from_secs(60))
        .await
        .expect("maintenance-mode apid did not become ready");

    let cli =
        Cli::new(&artifacts.cli_bin, fixture.vm.host_port).expect("failed to create CLI driver");

    // Write the install config to a temp file so muakctl can read it.
    let config_file = NamedTempFile::new().expect("failed to create config tempfile");
    std::fs::write(config_file.path(), install_config(APID_GUEST_PORT))
        .expect("failed to write install config");

    // muakctl install drives the full install + post-reboot poll internally.
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

    // After install the VM rebooted and granola logs this when running from the installed disk.
    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")
        .expect("installed-boot marker not found in serial log");

    // muakctl install saved an mTLS context — verify it works with an authenticated call.
    let disks = assert_success!(cli, ["disks"])
        .await
        .expect("authenticated muakctl disks failed");

    assert!(
        disks.contains("nvme0n1"),
        "expected nvme0n1 in disk listing, got: {disks}"
    );

    fixture.vm.kill().await.expect("failed to kill VM");
}
