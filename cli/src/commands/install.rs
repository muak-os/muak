use core::time::Duration;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result};
use config::system::disk;
use config::user::{ClientConfig, ServerContext};
use pki::csr;
use tokio::time::sleep;
use tokio_stream::StreamExt as _;
use tonic::transport::Channel;
use tui::console::Console;
use tui::live::Steps;

use crate::client::connect;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{GetConfigRequest, InstallRequest};

/// Data received upon successful completion of the install process.
struct CompletedInstall {
    ca_pem: String,
    client_cert_pem: String,
    server_name: String,
}

/// Handles system installation with configuration and certificate generation.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    config_path: PathBuf,
    server_endpoint: &str,
    console: &Console,
) -> Result<()> {
    let mut client_config = ClientConfig::load()?;

    let (key_pem, csr_pem) = csr::generate("muak-admin")?;

    let config_raw = std::fs::read_to_string(&config_path).context(format!(
        "Failed to read config file '{}'",
        config_path.display()
    ))?;

    let config = config::system::parse_from_str(&config_raw)
        .context(format!("Invalid config file '{}'", config_path.display()))?;

    config.validate_for_install().context(format!(
        "Invalid config for install in '{}'",
        config_path.display()
    ))?;

    let target_disk =
        disk::system(&config.disks).map_or_else(|| "no system disk".to_owned(), describe);

    let request = tonic::Request::new(InstallRequest {
        force,
        config_bytes: config_raw.into_bytes(),
        csr: csr_pem,
    });

    let msg = format!("Installing Muak to {target_disk}...");
    console.info(&msg);

    let response = client
        .install(request)
        .await
        .context("Failed to send install request")?;

    let mut stream = response.into_inner();
    let mut result_data: Option<CompletedInstall> = None;

    let steps = console.steps()?;

    while let Some(progress) = stream.next().await {
        let progress = progress.context("Error receiving install progress")?;

        if !progress.error.is_empty() {
            let msg = format!("Installation failed: {}", progress.error);
            steps.fail(&msg);
            let _finish_result = steps.finish();
            return Err(anyhow::anyhow!("{msg}"));
        }

        if progress.ca_pem.is_empty() {
            steps.start(&progress.message);
        } else {
            result_data = Some(CompletedInstall {
                ca_pem: progress.ca_pem,
                client_cert_pem: progress.client_cert_pem,
                server_name: progress.server_name,
            });
        }
    }

    let result = result_data.ok_or_else(|| {
        anyhow::anyhow!("Install stream ended without a completion or failure message")
    })?;

    let msg = format!("Successfully installed Muak to {target_disk}");
    steps.complete(&msg);

    let context_name = if result.server_name.is_empty() {
        "default"
    } else {
        &result.server_name
    };

    let ctx = ServerContext::from_pem(
        server_endpoint,
        &result.ca_pem,
        &result.client_cert_pem,
        key_pem.as_bytes(),
    );

    let actual_name = client_config.add_context(context_name, ctx.clone());
    client_config.set_current(&actual_name)?;
    client_config.save()?;

    let reboot_result = wait_for_reboot(&ctx, &steps).await;
    steps.finish()?;
    reboot_result?;

    let msg = format!("Context '{actual_name}' added and set as current.");
    console.success(&msg);

    Ok(())
}

/// Renders a configured disk entry for operator messages.
fn describe(entry: &disk::Entry) -> String {
    if let Some(device) = entry.device.as_deref() {
        return device.to_owned();
    }

    let mut parts = Vec::new();
    push_matcher(&mut parts, "serial", entry.serial.as_deref());
    push_matcher(&mut parts, "wwid", entry.wwid.as_deref());
    push_matcher(&mut parts, "model", entry.model.as_deref());
    push_matcher(&mut parts, "subsystem", entry.subsystem.as_deref());
    push_matcher(&mut parts, "bus_path", entry.bus_path.as_deref());
    if let Some(rotational) = entry.rotational {
        parts.push(format!("rotational={rotational}"));
    }

    if parts.is_empty() {
        "selector".to_owned()
    } else {
        parts.join(" ")
    }
}

fn push_matcher(parts: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        parts.push(format!("{name}={value}"));
    }
}

/// Polls the server after reboot to verify the install succeeded.
async fn wait_for_reboot(ctx: &ServerContext, steps: &Steps) -> Result<()> {
    steps.start("Rebooting system...");
    steps.start("Waiting for system to come back online...");

    let timeout = Duration::from_mins(5);
    let poll_interval = Duration::from_secs(2);
    let start = Instant::now();

    loop {
        if start.elapsed() > timeout {
            steps.fail("Timed out waiting for system to come back online");
            return Err(anyhow::anyhow!(
                "The installation completed but the system did not respond within 5 minutes.\n\
                 Please check the system manually to verify it booted correctly."
            ));
        }

        let Ok(channel) = connect(ctx, 10).await else {
            sleep(poll_interval).await;
            continue;
        };

        let mut client = ProvisionServiceClient::new(channel);
        match client
            .get_config(tonic::Request::new(GetConfigRequest {}))
            .await
        {
            Ok(_) => {
                steps.complete("System is back online. Installation verified successfully!");
                return Ok(());
            }
            Err(_) => {
                sleep(poll_interval).await;
            }
        }
    }
}
