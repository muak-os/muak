use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{GetSecurityStateRequest, SecurityServiceClient};
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
            let request = tonic::Request::new(GetSecurityStateRequest {});

            let response = client.get_security_state(request).await?;
            let resp = response.into_inner();

            println!("{}", ui::style::header("Security State"));

            let status = if resp.secure_boot_enabled {
                ui::style::positive("Enabled").to_string()
            } else {
                ui::style::negative("Disabled").to_string()
            };

            println!("  Secure Boot: {status}");
        }
    }
    Ok(())
}
