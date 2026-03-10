use std::time::Duration;

use e2e::artifacts::Artifacts;
use e2e::assert_success_insecure;
use e2e::cli::Cli;
use e2e::vm::TestFixture;

pub const DEFAULT_REGISTRY: &str = "ghcr.io/muak-os";
pub const DEFAULT_TAG: &str = "latest";

/// The image used for the initial install.
pub fn install_image() -> String {
    let registry = std::env::var("REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_owned());
    let tag = std::env::var("TAG").unwrap_or_else(|_| DEFAULT_TAG.to_owned());
    format!("{registry}/installer:{tag}")
}

/// Boots a VM, runs install with the given extra config patch, waits for the
/// installed system marker, and returns the fixture + CLI driver.
pub async fn boot_and_install(
    artifacts: &Artifacts,
    extra_config: impl FnOnce(&mut config::SystemConfig),
) -> (TestFixture, Cli) {
    let fixture = TestFixture::boot_install(artifacts)
        .await
        .expect("failed to boot install VM");

    fixture
        .vm
        .wait_ready(Duration::from_secs(60))
        .await
        .expect("maintenance-mode apid did not become ready");

    let cli =
        Cli::new(&artifacts.cli_bin, fixture.vm.host_port).expect("failed to create CLI driver");

    let image = install_image();
    let config_file = cli
        .generate_config(|cfg| {
            cfg.disk.system = "/dev/nvme0n1".to_owned();
            cfg.host.image = image;
            extra_config(cfg);
        })
        .await
        .expect("failed to generate install config");

    tokio::time::timeout(
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
    .expect("install timed out")
    .expect("muakctl install failed");

    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")
        .expect("installed-boot marker not found in serial log");

    (fixture, cli)
}
