use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;
use tui::style::Role;

use crate::client::security_service::security_service_client::SecurityServiceClient;
use crate::client::security_service::{
    GetSecurityStateRequest, SecureBootDeviation, SecureBootState,
};
use crate::console::Console;

#[derive(Subcommand, Clone)]
pub enum Action {
    State,
}

/// Handles security subcommands.
pub async fn handle(
    client: &mut SecurityServiceClient<Channel>,
    action: Action,
    console: &Console,
) -> Result<()> {
    match action {
        Action::State => {
            let response = client
                .get_security_state(tonic::Request::new(GetSecurityStateRequest {}))
                .await?;
            let resp = response.into_inner();

            console.line(Role::Header, "Security State");

            let status = firmware_status(console, resp.secure_boot);
            println!("  Secure Boot: {status}");

            let setup_mode = toggle(console, resp.setup_mode, false);
            println!("  Setup Mode:  {setup_mode}");

            let config = toggle(console, resp.config_enabled, true);
            println!("  Config:      {config}");

            let deviation = SecureBootDeviation::try_from(resp.deviation)
                .unwrap_or(SecureBootDeviation::Unspecified);
            if let Some(warning) = deviation_warning(deviation) {
                console.line(Role::Warn, &format!("  Warning: {warning}"));
            }
        }
    }

    Ok(())
}

fn firmware_status(console: &Console, value: i32) -> String {
    match SecureBootState::try_from(value).unwrap_or(SecureBootState::Unspecified) {
        SecureBootState::Enabled => console.paint("Enabled", Role::Positive).to_string(),
        SecureBootState::Disabled => console.paint("Disabled", Role::Negative).to_string(),
        SecureBootState::Pending => console
            .paint("Pending (firmware reboot required)", Role::Highlight)
            .to_string(),
        SecureBootState::Unspecified => console.paint("Unknown", Role::Negative).to_string(),
    }
}

fn toggle(console: &Console, value: bool, as_positive: bool) -> String {
    let text = if value { "Enabled" } else { "Disabled" };
    let role = if value == as_positive {
        Role::Positive
    } else {
        Role::Negative
    };

    console.paint(text, role).to_string()
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
