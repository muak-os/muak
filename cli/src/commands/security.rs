use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::security_service::{
    GetSecurityStateRequest, SecureBootState, security_service_client::SecurityServiceClient,
};
use crate::ui;

#[derive(Subcommand, Clone)]
pub enum Action {
    State,
}

/// Handles security subcommands.
pub async fn handle(client: &mut SecurityServiceClient<Channel>, action: Action) -> Result<()> {
    match action {
        Action::State => {
            let response = client
                .get_security_state(tonic::Request::new(GetSecurityStateRequest {}))
                .await?;
            let resp = response.into_inner();

            println!("{}", ui::style::header("Security State"));

            let status = match SecureBootState::try_from(resp.secure_boot)
                .unwrap_or(SecureBootState::Unspecified)
            {
                SecureBootState::Enabled => ui::style::positive("Enabled").to_string(),
                SecureBootState::Disabled => ui::style::negative("Disabled").to_string(),
                SecureBootState::Pending => {
                    ui::style::highlight("Pending (firmware reboot required)").to_string()
                }
                SecureBootState::Unspecified => ui::style::negative("Unknown").to_string(),
            };

            println!("  Secure Boot: {status}");

            let setup_mode = if resp.setup_mode {
                ui::style::negative("Enabled").to_string()
            } else {
                ui::style::positive("Disabled").to_string()
            };

            println!("  Setup Mode:  {setup_mode}");
        }
    }
    Ok(())
}
