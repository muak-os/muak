use std::time::Duration;

use e2e::artifacts::Artifacts;
use e2e::assert_success_insecure;
use e2e::cli::Cli;
use e2e::vm::TestFixture;

#[tokio::test]
async fn vm_boots_and_apid_reachable() {
    // ARRANGE
    let artifacts = Artifacts::from_env().expect("failed to resolve test artifacts");

    let fixture = TestFixture::boot_live(&artifacts)
        .await
        .expect("failed to boot QEMU VM");

    fixture
        .vm
        .wait_ready(Duration::from_secs(30))
        .await
        .expect("apid did not become reachable");

    let cli =
        Cli::new(&artifacts.cli_bin, fixture.vm.host_port).expect("failed to create CLI driver");

    // ACT & ASSERT
    assert_success_insecure!(cli, ["disks"])
        .await
        .expect("muak disks failed");
}
