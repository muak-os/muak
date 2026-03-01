use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{GetSecurityStateRequest, SecureBootState, SecurityServiceClient};
use crate::ui;

#[derive(Subcommand)]
pub enum SecurityAction {
    State,
}

/// Handles security subcommands
pub async fn handle(
    client: &mut SecurityServiceClient<Channel>,
    action: SecurityAction,
) -> Result<()> {
    match action {
        SecurityAction::State => {
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
        }
    }
    Ok(())
}
