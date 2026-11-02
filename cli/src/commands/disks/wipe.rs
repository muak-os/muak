//! Guarded wipe of a whole disk.

use anyhow::Result;
use tonic::transport::Channel;
use tui::console::Console;
use tui::prompt::Prompter;
use tui::style::Role;

use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{WipeDiskRequest, WipeMethod as ProtoWipeMethod};
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

    if !args.yes {
        confirm(&args.id)?;
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
        return Err(Failure::incomplete(resp.error).into());
    }

    console.success(&resp.message);

    Ok(())
}

fn confirm(id: &str) -> Result<()> {
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

    if prompter.confirm_phrase("", &format!("WIPE {id}"))? {
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
}
