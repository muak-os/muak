//! Guarded wipe of a whole disk.

use anyhow::Result;
use tonic::transport::Channel;
use tui::console::Console;
use tui::prompt::Prompter;
use tui::style::Role;

use super::preview;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{
    DiskInfo, WipeDiskRequest, WipeDiskResponse, WipeMethod as ProtoWipeMethod,
};
use crate::commands::Failure;

/// Wipe method selected on the command line.
#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    /// Zero the head and tail of the disk.
    #[default]
    Fast,
    /// Zero the whole disk.
    Zeroes,
}

/// Arguments of the wipe subcommand.
#[derive(clap::Args, Clone)]
pub struct Args {
    /// Kernel name (sda) or device path (/dev/sda).
    pub id: String,
    /// Validate the target and print the plan without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Wipe method.
    #[arg(long, value_enum, default_value_t = Method::Fast)]
    pub method: Method,
    /// Drop partition entries in addition to wiping the disk.
    #[arg(long)]
    pub drop_partition: bool,
    /// Skip the interactive confirmation phrase.
    #[arg(long)]
    pub yes: bool,
    /// Allow wiping the disk that hosts the running system.
    #[arg(long)]
    pub allow_system_disk: bool,
}

/// Validates the target on the server, confirms it and starts the wipe.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    args: Args,
    console: &Console,
) -> Result<()> {
    let preview_request = WipeDiskRequest {
        disk: args.id.clone(),
        dry_run: true,
        method: method(args.method),
        drop_partition: args.drop_partition,
        allow_system_disk: args.allow_system_disk,
    };

    let preview_resp = client.wipe_disk(preview_request).await?.into_inner();

    preview::print(&preview_resp, console)?;

    if !preview_resp.success {
        return Err(Failure::preflight(preview_resp.error).into());
    }

    if !args.dry_run {
        console.line(
            Role::Error,
            &format!(
                "WARNING: This will {}",
                preview_resp.message.trim_start_matches("Would ")
            ),
        );
    }

    console.info(&preview_resp.message);

    if args.dry_run {
        return Ok(());
    }

    let disk = preview_resp
        .disk
        .as_ref()
        .ok_or_else(|| Failure::incomplete("Server did not return the resolved disk"))?;
    let phrase = confirmation(disk);

    if !args.yes {
        confirm(&phrase)?;
    }

    let request = WipeDiskRequest {
        disk: args.id,
        dry_run: false,
        method: method(args.method),
        drop_partition: args.drop_partition,
        allow_system_disk: args.allow_system_disk,
    };

    let resp = client.wipe_disk(request).await?.into_inner();

    if !resp.success {
        return Err(failure(&resp).into());
    }

    console.success(&resp.message);

    Ok(())
}

fn confirmation(disk: &DiskInfo) -> String {
    format!("WIPE {}", disk.name)
}

fn failure(response: &WipeDiskResponse) -> Failure {
    if response.checks.iter().any(|check| !check.passed) {
        Failure::preflight(response.error.clone())
    } else {
        Failure::incomplete(response.error.clone())
    }
}

fn confirm(phrase: &str) -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(Failure::declined(
            "Refusing to wipe without confirmation: standard input is not a terminal. \
             Add --yes to skip the prompt.",
        )
        .into());
    }

    let Some(mut prompter) = Prompter::stdio() else {
        return Err(Failure::declined(
            "Refusing to wipe without confirmation: standard input is not a terminal. \
             Add --yes to skip the prompt.",
        )
        .into());
    };

    if prompter.confirm_phrase("", phrase)? {
        return Ok(());
    }

    Err(Failure::declined("Disk wipe cancelled").into())
}

fn method(value: Method) -> i32 {
    match value {
        Method::Fast => i32::from(ProtoWipeMethod::Fast),
        Method::Zeroes => i32::from(ProtoWipeMethod::Zeroes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::provision_service::PreflightCheck;

    fn disk() -> DiskInfo {
        DiskInfo {
            name: "nvme0n1".to_owned(),
            path: "/dev/nvme0n1".to_owned(),
            ..DiskInfo::default()
        }
    }

    #[test]
    fn method_maps_to_the_wire_enum() {
        // ARRANGE
        let values = [Method::Fast, Method::Zeroes];

        // ACT
        let mapped = values.map(method);

        // ASSERT
        assert_eq!(
            mapped,
            [
                i32::from(ProtoWipeMethod::Fast),
                i32::from(ProtoWipeMethod::Zeroes)
            ]
        );
    }

    #[test]
    fn refusal_with_a_failed_check_is_a_preflight_failure() {
        // ARRANGE
        let response = WipeDiskResponse {
            success: false,
            error: "mounted".to_owned(),
            checks: vec![PreflightCheck {
                name: "mounted_partitions".to_owned(),
                passed: false,
                detail: "mounted".to_owned(),
            }],
            ..WipeDiskResponse::default()
        };

        // ACT
        let failure = failure(&response);

        // ASSERT
        assert_eq!(failure.code(), 3_i32);
    }

    #[test]
    fn refusal_without_a_failed_check_is_incomplete() {
        // ARRANGE
        let response = WipeDiskResponse {
            success: false,
            error: "write error".to_owned(),
            checks: vec![PreflightCheck {
                name: "read_only".to_owned(),
                passed: true,
                detail: "writable".to_owned(),
            }],
            ..WipeDiskResponse::default()
        };

        // ACT
        let failure = failure(&response);

        // ASSERT
        assert_eq!(failure.code(), 5_i32);
    }

    #[test]
    fn confirmation_uses_the_resolved_kernel_name() {
        // ARRANGE
        let disk = disk();

        // ACT
        let phrase = confirmation(&disk);

        // ASSERT
        assert_eq!(phrase, "WIPE nvme0n1");
    }
}
