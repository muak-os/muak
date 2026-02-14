use std::path::PathBuf;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tokio_stream::StreamExt;
use tonic::transport::Channel;

use crate::client::{
    GetConfigRequest, InstallRequest, InstallStep, ProvisionServiceClient, connect,
};
use crate::config::{ClientConfig, ServerContext};

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
) -> Result<()> {
    let mut client_config = ClientConfig::load()?;

    let (key_pem, csr_pem) = pki::generate_csr("muak-admin")?;

    let config_toml = std::fs::read_to_string(&config_path).context(format!(
        "Failed to read config file '{}'",
        config_path.display()
    ))?;

    let config = sysconfig::parse_from_str(&config_toml).context(format!(
        "Invalid TOML in config file '{}'",
        config_path.display()
    ))?;

    config.validate_for_install().context(format!(
        "Invalid config for install in '{}'",
        config_path.display()
    ))?;

    let target_disk = config.system.disk.clone();

    let request = tonic::Request::new(InstallRequest {
        force,
        config_toml: config_toml.into_bytes(),
        csr: csr_pem,
    });

    println!("{}", format!("Installing Muak to {target_disk}...").blue());

    let response = client
        .install(request)
        .await
        .context("Failed to send install request")?;

    let mut stream = response.into_inner();
    let mut result_data: Option<CompletedInstall> = None;

    while let Some(progress) = stream.next().await {
        let progress = progress.context("Error receiving install progress")?;
        let step = InstallStep::try_from(progress.step).unwrap_or(InstallStep::Unknown);

        match step {
            InstallStep::Failed => {
                eprintln!(
                    "{}",
                    format!("Installation failed: {}", progress.error).red()
                );
                std::process::exit(1);
            }
            InstallStep::Completed => {
                result_data = Some(CompletedInstall {
                    ca_pem: progress.ca_pem,
                    client_cert_pem: progress.client_cert_pem,
                    server_name: progress.server_name,
                });
            }
            _ => {
                print_step(&progress.message);
            }
        }
    }

    let result = result_data.ok_or_else(|| {
        anyhow::anyhow!("Install stream ended without a completion or failure message")
    })?;

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

    println!(
        "{}",
        format!("Successfully installed Muak to {target_disk}").green()
    );
    println!(
        "{}",
        format!("Context '{}' added and set as current.", actual_name).green()
    );

    wait_for_reboot(&ctx).await
}

/// Prints a step message
fn print_step(message: &str) {
    println!("{}", format!("{message}").yellow());
}

/// Polls the server after reboot to verify the install succeeded.
async fn wait_for_reboot(ctx: &ServerContext) -> Result<()> {
    println!(
        "{}",
        "\nSystem will reboot automatically in 3 seconds...".yellow()
    );
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    println!("{}", "Waiting for system to come back online...".yellow());

    let timeout = std::time::Duration::from_secs(60 * 5);
    let poll_interval = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            eprintln!(
                "{}",
                "\nWARNING: Timed out waiting for system to come back online after install."
                    .red()
                    .bold()
            );
            eprintln!(
                "{}",
                "The installation completed but the system did not respond within 5 minutes.".red()
            );
            eprintln!(
                "{}",
                "Please check the system manually to verify it booted correctly.".red()
            );
            std::process::exit(1);
        }

        let channel = match connect(ctx, 10).await {
            Ok(c) => c,
            Err(_) => {
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        let mut client = ProvisionServiceClient::new(channel);
        match client
            .get_config(tonic::Request::new(GetConfigRequest {}))
            .await
        {
            Ok(_) => {
                println!(
                    "{}",
                    "System is back online. Installation verified successfully!".green()
                );
                return Ok(());
            }
            Err(_) => {
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
