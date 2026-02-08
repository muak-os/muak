use anyhow::Result;
use clap::Subcommand;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{GetSecurityStateRequest, SecurityServiceClient};

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

            println!("{}", "Security State".green().bold());

            let (label, color_fn): (&str, fn(&str) -> String) = if resp.secure_boot_enabled {
                ("Enabled", |s| s.green().to_string())
            } else {
                ("Disabled", |s| s.red().to_string())
            };

            println!("  Secure Boot: {}", color_fn(label));
        }
    }
    Ok(())
}
