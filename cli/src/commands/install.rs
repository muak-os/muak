use std::path::PathBuf;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{GetConfigRequest, InstallRequest, ProvisionServiceClient, connect};
use crate::config::{ClientConfig, ServerContext};

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
    let resp = response.into_inner();

    if !resp.success {
        eprintln!("{}", format!("Installation failed: {}", resp.error).red());
        std::process::exit(1);
    }

    let context_name = if resp.server_name.is_empty() {
        "default"
    } else {
        &resp.server_name
    };

    let ctx = ServerContext::from_pem(
        server_endpoint,
        &resp.ca_pem,
        &resp.client_cert_pem,
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

/// Polls the server after reboot to verify the install succeeded
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
