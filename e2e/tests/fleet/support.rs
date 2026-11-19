//! Helpers that drive the production CLI; no fleet API or enrollment fixtures.

use core::time::Duration;
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use config::system::fleet::Config;
use e2e::artifacts::Artifacts;
use e2e::assert_success;
use e2e::cli::Cli;
use e2e::vm::TestFixture;
use tokio::time::{sleep, timeout};

use crate::common::{boot_and_install, install_version};

const WAIT: Duration = Duration::from_secs(120);
const BOOT: &str = "[millefeuille] Persistent partitions mounted";

pub(crate) struct Identity {
    pub(crate) node: String,
    pub(crate) certificate: String,
}

impl Identity {
    pub(crate) async fn read(cli: &Cli) -> Result<Self> {
        let output = wait(cli, &["fleet", "agent-status"], |out| {
            out.contains("Phase: reporting")
        })
        .await?;
        Ok(Self {
            node: field(&output, "Node: ")?.to_owned(),
            certificate: field(&output, "Certificate: ")?.to_owned(),
        })
    }

    pub(crate) fn assert_matches(&self, expected: &Self) -> Result<()> {
        ensure!(self.node == expected.node, "node identity changed");
        ensure!(
            self.certificate == expected.certificate,
            "credential changed unexpectedly"
        );
        Ok(())
    }
}

pub(crate) struct Workload {
    kernel: String,
    initrd: String,
}

impl Workload {
    pub(crate) fn from_env() -> Result<Self> {
        Ok(Self {
            kernel: workload_artifact("MUAK_FLEET_VM_KERNEL")?,
            initrd: workload_artifact("MUAK_FLEET_VM_INITRD")?,
        })
    }

    pub(crate) async fn create(&self, cli: &Cli) -> Result<String> {
        let output = timeout(
            WAIT,
            assert_success!(
                cli,
                [
                    "vm",
                    "create",
                    "qemu",
                    "--name",
                    "fleet-e2e-workload",
                    "--kernel",
                    &self.kernel,
                    "--initrd",
                    &self.initrd,
                    "--memory",
                    "256",
                    "--disk-size",
                    "64",
                    "--cmdline",
                    "console=ttyS0",
                ]
            ),
        )
        .await
        .context("workload creation timed out")??;
        let (_, rest) = output
            .split_once("(ID: ")
            .context("VM creation returned no identity")?;
        let (id, _) = rest
            .split_once(')')
            .context("invalid VM creation identity")?;
        ensure!(id.len() == 36, "invalid VM identity: {id}");
        wait(cli, &["vm", "logs", id], |out| {
            out.contains("Linux version")
        })
        .await?;
        Ok(id.to_owned())
    }
}

fn workload_artifact(name: &str) -> Result<String> {
    let path = PathBuf::from(std::env::var(name).with_context(|| format!("set {name}"))?);
    ensure!(path.is_file(), "{name} is not a file: {}", path.display());
    Ok(path.canonicalize()?.display().to_string())
}

pub(crate) async fn install(
    artifacts: &Artifacts,
    name: &str,
    role: Config,
) -> Result<(TestFixture, Cli)> {
    boot_and_install(artifacts, |config| {
        name.clone_into(&mut config.host.name);
        config.host.secureboot = false;
        config.host.extensions = vec!["qemu".to_owned(), "fleetd".to_owned()];
        config.fleet = Some(role);
    })
    .await
}

pub(crate) fn field<'a>(output: &'a str, prefix: &str) -> Result<&'a str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .filter(|value| !value.is_empty())
        .with_context(|| format!("missing {prefix} in CLI output: {output}"))
}

pub(crate) async fn wait<F>(cli: &Cli, args: &[&str], ready: F) -> Result<String>
where
    F: Fn(&str) -> bool,
{
    let mut last = String::new();
    timeout(WAIT, async {
        loop {
            match timeout(
                Duration::from_secs(15),
                cli.assert_success_impl(args, false),
            )
            .await
            {
                Ok(Ok(output)) if ready(&output) => return output,
                Ok(Ok(output)) => last = output,
                Ok(Err(error)) => last = error.to_string(),
                Err(error) => last = error.to_string(),
            }
            sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .with_context(|| format!("CLI {args:?} did not converge; last result: {last}"))
}

pub(crate) async fn enroll(manager: &Cli, worker: &Cli) -> Result<Identity> {
    let output = wait(worker, &["fleet", "agent-status"], |out| {
        out.contains("Phase: pending_certificate")
    })
    .await?;
    let csr = field(&output, "CSR: ")?;
    approve_certificate(manager, csr).await?;
    let output = wait(worker, &["fleet", "agent-status"], |out| {
        out.contains("Phase: pending_node")
    })
    .await?;
    let node = field(&output, "Node: ")?;
    let certificate = field(&output, "Certificate: ")?;
    wait(manager, &["fleet", "nodes"], |out| {
        node_row(out, node)
            .is_some_and(|row| row.contains("\tpending\t") && row.ends_with(certificate))
    })
    .await?;
    assert_success!(manager, ["fleet", "approve", node, certificate]).await?;
    Identity::read(worker).await
}

async fn approve_certificate(manager: &Cli, csr: &str) -> Result<()> {
    let short = csr.get(..16).context("invalid CSR fingerprint")?;
    wait(manager, &["auth", "requests"], |out| out.contains(short)).await?;
    let approved = assert_success!(
        manager,
        ["auth", "approve", csr, "--permissions", "fleet:report"]
    )
    .await?;
    ensure!(
        approved.contains("Certificate issued!"),
        "CSR was not approved: {approved}"
    );
    Ok(())
}

fn node_row<'a>(output: &'a str, node: &str) -> Option<&'a str> {
    output
        .lines()
        .find(|line| line.split('\t').next() == Some(node))
}

pub(crate) async fn fresh_nodes(manager: &Cli, nodes: &[&str]) -> Result<()> {
    wait(manager, &["fleet", "nodes"], |out| {
        out.lines().count() == nodes.len().saturating_add(1)
            && nodes.iter().all(|node| {
                node_row(out, node).is_some_and(|row| row.contains("\tapproved\tfresh\t"))
            })
    })
    .await?;
    Ok(())
}

pub(crate) async fn vm_state(manager: &Cli, node: &str, vm: &str, state: &str) -> Result<()> {
    let expected = format!("{node}\t{vm}\tfleet-e2e-workload\t{state}\tfresh");
    wait(manager, &["fleet", "vms"], |out| {
        out.lines().any(|line| line == expected)
    })
    .await?;
    Ok(())
}

pub(crate) async fn reboot(fixture: &TestFixture, cli: &Cli) -> Result<()> {
    let count = fixture.vm.read_serial()?.matches(BOOT).count();
    // A reboot can close its own RPC; a new boot marker is the completion proof.
    let _request = timeout(Duration::from_secs(15), cli.run(["reboot"], false)).await;
    timeout(WAIT, async {
        while fixture.vm.read_serial()?.matches(BOOT).count() <= count {
            sleep(Duration::from_millis(500)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .with_context(|| {
        format!(
            "reboot timed out: {}",
            fixture.vm.read_serial().unwrap_or_default()
        )
    })??;
    wait(cli, &["config", "get"], |out| out.contains("api_version")).await?;
    Ok(())
}

pub(crate) async fn update(cli: &Cli) -> Result<()> {
    let output = timeout(
        Duration::from_secs(180),
        assert_success!(cli, ["update", "--version", &install_version(), "--force"]),
    )
    .await
    .context("worker update timed out")??;
    ensure!(
        output.contains("committed successfully"),
        "update was not committed: {output}"
    );
    Ok(())
}

pub(crate) async fn renew_and_revoke(
    manager: &Cli,
    worker: &Cli,
    old: &Identity,
    other: &str,
) -> Result<()> {
    assert_success!(worker, ["fleet", "renew-agent-credential"]).await?;
    let output = wait(worker, &["fleet", "agent-status"], |out| {
        out.contains("Phase: pending_certificate")
    })
    .await?;
    approve_certificate(manager, field(&output, "CSR: ")?).await?;
    let output = wait(worker, &["fleet", "agent-status"], |out| {
        field(out, "Certificate: ").is_ok_and(|value| value != old.certificate)
    })
    .await?;
    let certificate = field(&output, "Certificate: ")?;
    ensure!(
        field(&output, "Node: ")? == old.node,
        "renewal changed node identity"
    );
    assert_success!(
        manager,
        ["fleet", "rotate-credential", &old.node, certificate]
    )
    .await?;
    assert_success!(manager, ["auth", "revoke", &old.certificate]).await?;
    let new = Identity::read(worker).await?;
    ensure!(
        new.node == old.node && new.certificate == certificate,
        "renewal did not converge"
    );
    fresh_nodes(manager, &[&old.node, other]).await?;
    assert_success!(manager, ["fleet", "revoke", &old.node]).await?;
    assert_success!(manager, ["auth", "revoke", certificate]).await?;
    wait(manager, &["fleet", "nodes"], |out| {
        node_row(out, &old.node).is_some_and(|row| row.contains("\trevoked\tstale\tnever\t"))
            && node_row(out, other).is_some_and(|row| row.contains("\tapproved\tfresh\t"))
    })
    .await?;
    let inventory = assert_success!(manager, ["fleet", "vms"]).await?;
    ensure!(
        !inventory.contains(&old.node),
        "revoked observations retained in fleet inventory"
    );
    Ok(())
}
