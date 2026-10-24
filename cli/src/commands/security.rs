use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;
use tui::style;

use crate::client::security_service::{
    GetSecurityStateRequest, SecureBootDeviation, SecureBootState,
    security_service_client::SecurityServiceClient,
};

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

            println!("{}", style::header("Security State"));

            let status = firmware_status(resp.secure_boot);
            println!("  Secure Boot: {status}");

            let setup_mode = toggle(resp.setup_mode, false);
            println!("  Setup Mode:  {setup_mode}");

            let config = toggle(resp.config_enabled, true);
            println!("  Config:      {config}");

            let deviation = SecureBootDeviation::try_from(resp.deviation)
                .unwrap_or(SecureBootDeviation::Unspecified);
            if let Some(warning) = deviation_warning(deviation) {
                println!("  {}", style::warn(&format!("Warning: {warning}")));
            }
        }
    }

    Ok(())
}

fn firmware_status(value: i32) -> String {
    match SecureBootState::try_from(value).unwrap_or(SecureBootState::Unspecified) {
        SecureBootState::Enabled => style::positive("Enabled").to_string(),
        SecureBootState::Disabled => style::negative("Disabled").to_string(),
        SecureBootState::Pending => {
            style::highlight("Pending (firmware reboot required)").to_string()
        }
        SecureBootState::Unspecified => style::negative("Unknown").to_string(),
    }
}

fn toggle(value: bool, as_positive: bool) -> String {
    if value == as_positive {
        style::positive(if value { "Enabled" } else { "Disabled" }).to_string()
    } else {
        style::negative(if value { "Enabled" } else { "Disabled" }).to_string()
    }
}

fn deviation_warning(deviation: SecureBootDeviation) -> Option<&'static str> {
    match deviation {
        SecureBootDeviation::ConfigEnabledNoKeys => {
            Some("config enables Secure Boot but no keys are enrolled")
        }
        SecureBootDeviation::KeysRemain => {
            Some("config disables Secure Boot but keys are still enrolled")
        }
        SecureBootDeviation::EnforcingWhileDisabled => {
            Some("config disables Secure Boot but firmware still enforces it")
        }
        SecureBootDeviation::None | SecureBootDeviation::Unspecified => None,
    }
}
