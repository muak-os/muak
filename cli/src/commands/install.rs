use std::path::PathBuf;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{InstallRequest, ProvisionServiceClient};

/// Handles the install command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    config_path: PathBuf,
) -> Result<()> {
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
    });

    println!("{}", format!("Installing Muak to {target_disk}...").blue());

    let response = client.install(request).await?;
    let resp = response.into_inner();

    if resp.success {
        println!(
            "{}",
            format!("Successfully installed Muak to {target_disk}").green()
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
