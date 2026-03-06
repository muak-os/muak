use std::time::Duration;

use e2e::artifacts::Artifacts;
use e2e::cli::Cli;
use e2e::vm::QemuVm;

#[tokio::test]
async fn vm_boots_and_apid_reachable() {
    let artifacts = Artifacts::from_env().expect("failed to resolve test artifacts");

    let mut vm = QemuVm::boot_live(&artifacts)
        .await
        .expect("failed to boot QEMU VM");

    vm.wait_ready(Duration::from_secs(60))
        .await
        .expect("apid did not become reachable");

    let cli = Cli::new(&artifacts.cli_bin, vm.host_port).expect("failed to create CLI driver");

    cli.assert_success_insecure(["disks"])
        .expect("muak disks failed");

    vm.kill().await.expect("failed to kill VM");
}

#[tokio::test]
async fn serial_log_contains_boot_messages() {
    let artifacts = Artifacts::from_env().expect("failed to resolve test artifacts");

    let mut vm = QemuVm::boot_live(&artifacts)
        .await
        .expect("failed to boot QEMU VM");

    vm.wait_ready(Duration::from_secs(60))
        .await
        .expect("apid did not become reachable");

    vm.assert_serial_contains("Linux version")
        .expect("kernel boot message not found in serial log");

    vm.kill().await.expect("failed to kill VM");
}
