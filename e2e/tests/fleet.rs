//! Booted fleet qualification, explicitly enabled by `just e2e-fleet`.

mod common;
#[path = "fleet/support.rs"]
mod support;

use anyhow::{Context as _, Result, ensure};
use config::system::fleet::Config;
use e2e::artifacts::Artifacts;
use e2e::assert_success;
use support::{Identity, Workload, enroll, field, install, reboot, wait};

async fn qualify_installed_fleet() -> Result<()> {
    // ARRANGE
    let artifacts = Artifacts::from_env()?;
    let workload = Workload::from_env()?;
    let (manager, manager_cli) = install(
        &artifacts,
        "fleet-manager",
        Config::Manager {
            max_nodes: 16,
            stale_after_secs: 10,
        },
    )
    .await?;
    let info = wait(&manager_cli, &["fleet", "info"], |out| {
        out.contains("CA:\n")
    })
    .await?;
    let domain = field(&info, "Domain: ")?;
    let (_, ca) = info
        .split_once("CA:\n")
        .ok_or_else(|| anyhow::anyhow!("missing manager CA"))?;
    let role = Config::Agent {
        manager: format!("https://10.0.2.2:{}", manager.vm.host_port),
        domain: domain.to_owned(),
        ca_pem: ca.to_owned(),
        server_name: field(&info, "Server name: ")?.to_owned(),
        report_interval_secs: 1,
    };
    let (worker, worker_cli) = install(&artifacts, "fleet-worker-a", role.clone()).await?;
    let (other, other_cli) = install(&artifacts, "fleet-worker-b", role).await?;

    // ACT: use production CSR approval and exact node approval through the CLI.
    let first = enroll(&manager_cli, &worker_cli).await?;
    let second = enroll(&manager_cli, &other_cli).await?;
    ensure!(
        first.node != second.node,
        "reusable image produced duplicate node identities"
    );
    ensure!(
        first.certificate != second.certificate,
        "workers share a credential"
    );
    support::fresh_nodes(&manager_cli, &[&first.node, &second.node]).await?;
    let vm = workload.create(&worker_cli).await?;
    support::vm_state(&manager_cli, &first.node, &vm, "running").await?;
    assert_success!(worker_cli, ["vm", "stop", &vm, "--force"]).await?;
    support::vm_state(&manager_cli, &first.node, &vm, "stopped").await?;

    // ASSERT: installed STATE survives worker reboot/update and manager reboot.
    reboot(&worker, &worker_cli).await?;
    Identity::read(&worker_cli).await?.assert_matches(&first)?;
    support::vm_state(&manager_cli, &first.node, &vm, "stopped").await?;
    support::update(&worker_cli).await?;
    Identity::read(&worker_cli).await?.assert_matches(&first)?;
    support::vm_state(&manager_cli, &first.node, &vm, "stopped").await?;
    reboot(&manager, &manager_cli).await?;
    let restored = assert_success!(manager_cli, ["fleet", "info"]).await?;
    ensure!(
        field(&restored, "Domain: ")? == domain,
        "manager domain changed after reboot"
    );
    support::fresh_nodes(&manager_cli, &[&first.node, &second.node]).await?;
    assert_success!(worker_cli, ["vm", "start", &vm]).await?;
    support::vm_state(&manager_cli, &first.node, &vm, "running").await?;
    assert_success!(worker_cli, ["vm", "stop", &vm, "--force"]).await?;
    support::vm_state(&manager_cli, &first.node, &vm, "stopped").await?;
    support::renew_and_revoke(&manager_cli, &worker_cli, &first, &second.node).await?;
    let local = assert_success!(worker_cli, ["vm", "list"]).await?;
    ensure!(
        local.contains(&vm),
        "revocation affected local workload ownership"
    );
    assert_success!(worker_cli, ["vm", "delete", &vm]).await?;
    for host in [&manager, &worker, &other] {
        let serial = host.vm.read_serial()?;
        let mode = serial
            .lines()
            .rfind(|line| line.contains("SELinux policy loaded ("))
            .context("no SELinux mode recorded for the host")?;
        ensure!(
            mode.contains("SELinux policy loaded (enforcing)"),
            "latest boot is not enforcing: {mode}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a fleet development catalog, enforcing SELinux and nested KVM; run just e2e-fleet"]
    async fn installed_fleet_enrollment_inventory_and_lifecycle() -> Result<()> {
        qualify_installed_fleet().await
    }
}
