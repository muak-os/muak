use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{
    CLIENT_KEY_FILE, InstallRequest, ProvisionServiceClient, config_dir, has_credentials,
    save_credentials,
};

/// Handles the install command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    config_path: PathBuf,
) -> Result<()> {
    if has_credentials() {
        println!(
            "{}",
            format!("Existing credentials found. Remove them to install the server").yellow()
        );
        return Ok(());
    }

    let (key_pem, csr_pem) = pki::generate_csr("muak-admin")?;

    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;

    let key_path = config_dir()?.join(CLIENT_KEY_FILE);
    fs::write(&key_path, &key_pem)?;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;

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

    let response = client.install(request).await?;
    let resp = response.into_inner();

    if resp.success {
        save_credentials(&resp.ca_pem, &resp.client_cert_pem)
            .context("Failed to save mTLS credentials")?;

        println!(
            "{}",
            format!("Successfully installed Muak to {target_disk}").green()
        );
        println!("{}", "Credentials saved.".green());
        println!(
            "{}",
            "\nSystem will reboot automatically in 3 seconds...".yellow()
        );
    } else {
        fs::remove_file(&key_path).context("Failed to remove private key after failed install")?;
        eprintln!("{}", format!("Installation failed: {}", resp.error).red());
        std::process::exit(1);
    }

    Ok(())
}
