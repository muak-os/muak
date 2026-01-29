use std::path::PathBuf;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{InstallRequest, ProvisionServiceClient};
use crate::config::{ClientConfig, ServerContext};

/// Handles the install command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    config_path: PathBuf,
    server_endpoint: &str,
) -> Result<()> {
    let mut client_config = ClientConfig::load()?;

    if client_config.has_credentials_for_endpoint(server_endpoint) {
        println!(
            "{}",
            "Existing credentials found for this server. Remove the context to reinstall.".yellow()
        );
        return Ok(());
    }

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

    if resp.success {
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

        let actual_name = client_config.add_context(context_name, ctx);
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
        println!(
            "{}",
            "\nSystem will reboot automatically in 3 seconds...".yellow()
        );
    } else {
        eprintln!("{}", format!("Installation failed: {}", resp.error).red());
        std::process::exit(1);
    }

    Ok(())
}
