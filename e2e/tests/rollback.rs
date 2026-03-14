mod common;

use std::time::Duration;

use common::{boot_and_install, install_image};
use e2e::artifacts::Artifacts;
use e2e::assert_success;

/// Triggers an update then abandons the CLI before it can contact provisiond,
#[tokio::test]
async fn update_rollback_on_cli_contact_timeout() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve artifacts");
    let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await;

    let update_cfg = cli
        .generate_config(|cfg| {
            cfg.disk.system = "/dev/nvme0n1".to_owned();
            cfg.host.image = install_image();
        })
        .await
        .expect("failed to generate update config");

    // ACT
    let config_path = update_cfg.path().display().to_string();
    tokio::select! {
        _ = cli.run(
            ["update", "--config", &config_path],
            false,
        ) => {},
        r = fixture.vm.wait_serial_contains(
            "kexec booting into update",
            Duration::from_secs(60),
        ) => {
            r.expect("kexec marker not found in serial log");
        }
    }

    fixture
        .vm
        .wait_serial_contains("Rebooting for rollback of update", Duration::from_secs(120))
        .await
        .expect("rollback reboot marker not found in serial log");

    fixture
        .vm
        .wait_serial_contains(
            "[apid] API daemon ready, listening on",
            Duration::from_secs(60),
        )
        .await
        .expect("apid did not become ready after rollback reboot");

    // ASSERT
    let history = assert_success!(cli, ["rollback", "history"])
        .await
        .expect("muakctl rollback history failed");

    assert!(
        history.contains("CLI contact check failed"),
        "expected rollback reason in history, got: {history}"
    );

    let config_out = assert_success!(cli, ["config", "get"])
        .await
        .expect("muakctl config get failed after rollback");

    assert!(
        config_out.contains(&install_image()),
        "expected original image '{}' in config after rollback, got: {config_out}",
        install_image()
    );

    fixture
        .vm
        .assert_serial_contains("muak.update_id=")
        .expect("kexec update marker not found in serial log");
}
