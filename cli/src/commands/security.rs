use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;
use tui::console::Console;
use tui::style::Role;

use crate::client::security_service::security_service_client::SecurityServiceClient;
use crate::client::security_service::{
    GetSecurityStateRequest, GetSecurityStateResponse, SecureBootDeviation, SecureBootState,
};

#[derive(Subcommand, Clone)]
pub enum Action {
    State,
}

/// Handles security subcommands.
pub(crate) async fn handle(
    client: &mut SecurityServiceClient<Channel>,
    action: Action,
    console: &Console,
) -> Result<()> {
    match action {
        Action::State => {
            let response = client
                .get_security_state(tonic::Request::new(GetSecurityStateRequest {}))
                .await?;
            let state = response.into_inner();

            console.line(Role::Header, "Security State");

            for line in report(console, &state) {
                println!("{line}");
            }
        }
    }

    Ok(())
}

fn report(console: &Console, state: &GetSecurityStateResponse) -> Vec<String> {
    let status = firmware_status(console, state.secure_boot);
    let setup_mode = toggle(console, state.setup_mode, false);

    let mut lines = vec![
        format!("  Secure Boot: {status}"),
        format!("  Setup Mode:  {setup_mode}"),
    ];

    if let Some(warning) = deviation_warning(deviation(state.deviation)) {
        let text = format!("  Warning: {warning}");
        lines.push(console.paint(&text, Role::Warn).to_string());
    }

    lines
}

fn deviation(value: i32) -> SecureBootDeviation {
    SecureBootDeviation::try_from(value).unwrap_or(SecureBootDeviation::Unspecified)
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

#[cfg(test)]
mod tests {
    use tui::glyphs::Glyphs;
    use tui::output::{Mode, Output};
    use tui::size::Size;
    use tui::style::Theme;

    use super::*;

    fn console() -> Console {
        let output = Output {
            mode: Mode::Plain,
            theme: Theme::plain(),
            glyphs: Glyphs::plain(),
            size: Size::new(80, 25),
            verbose: false,
        };

        Console::new(output, output)
    }

    fn response(
        secure_boot: SecureBootState,
        config_enabled: bool,
        deviation: SecureBootDeviation,
    ) -> GetSecurityStateResponse {
        GetSecurityStateResponse {
            secure_boot: secure_boot.into(),
            setup_mode: true,
            config_enabled,
            deviation: deviation.into(),
        }
    }

    #[test]
    fn report_omits_config_when_firmware_matches_it() {
        // ARRANGE
        let console = console();
        let state = response(SecureBootState::Disabled, false, SecureBootDeviation::None);

        // ACT
        let lines = report(&console, &state);

        // ASSERT
        assert_eq!(lines, ["  Secure Boot: Disabled", "  Setup Mode:  Enabled"]);
    }

    #[test]
    fn report_warns_when_config_wants_secure_boot_without_keys() {
        // ARRANGE
        let console = console();
        let state = response(
            SecureBootState::Disabled,
            true,
            SecureBootDeviation::ConfigEnabledNoKeys,
        );

        // ACT
        let lines = report(&console, &state);

        // ASSERT
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines.last().map(String::as_str),
            Some("  Warning: config enables Secure Boot but no keys are enrolled")
        );
    }

    #[test]
    fn report_marks_pending_firmware_reboot() {
        // ARRANGE
        let console = console();
        let state = response(SecureBootState::Pending, true, SecureBootDeviation::None);

        // ACT
        let lines = report(&console, &state);

        // ASSERT
        assert_eq!(
            lines.first().map(String::as_str),
            Some("  Secure Boot: Pending (firmware reboot required)")
        );
    }
}
