pub mod auth;
pub mod config;
pub mod context;
pub mod disks;
pub mod install;
pub mod logs;
pub mod process;
pub mod update;
pub mod vm;

use anyhow::Result;
use tonic::transport::Channel;

use crate::client::{
    ProcessServiceClient, ProvisionServiceClient, VmServiceClient, connect, connect_insecure,
};
use crate::config::ClientConfig;
use crate::{Cli, Commands};

/// Resolves the connection based on CLI flags and config.
///
/// Priority:
/// 1. `--endpoint` flag: maintenance mode (insecure HTTP)
/// 2. `--context` flag or MUAK_CONTEXT env: use specified context
/// 3. Config default context: use current context from config
///
/// Returns (Channel, endpoint_address).
async fn resolve_connection(cli: &Cli, timeout_secs: u64) -> Result<(Channel, String)> {
    if let Some(endpoint) = &cli.endpoint {
        let channel = connect_insecure(endpoint, timeout_secs).await?;
        return Ok((channel, endpoint.clone()));
    }

    let config = ClientConfig::load()?;

    let context_name = cli
        .context
        .as_ref()
        .or(config.context.as_ref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No context configured. Run 'muakctl install' or 'muakctl context add' first."
            )
        })?;

    let ctx = config.get_context(context_name).ok_or_else(|| {
        anyhow::anyhow!(
            "Context '{}' not found. Run 'muakctl context list' to see available contexts.",
            context_name
        )
    })?;

    let channel = connect(ctx, timeout_secs).await?;
    Ok((channel, ctx.endpoint.clone()))
}

/// Routes CLI commands to their handlers.
pub async fn run(cli: Cli) -> Result<()> {
    // Handle offline commands first
    match &cli.command {
        Commands::Config { action } => {
            if matches!(action, config::ConfigAction::Generate) {
                print!("{}", sysconfig::serialize_default());
                return Ok(());
            }
        }
        Commands::Context { action } => {
            return context::handle(action.clone());
        }
        Commands::Auth { action: None } => {
            let endpoint = cli.endpoint.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing --endpoint. Use 'muakctl auth --endpoint <ip>:<port>' to authenticate."
                )
            })?;
            return auth::enroll(endpoint).await;
        }
        _ => {}
    }

    let timeout_secs = match &cli.command {
        Commands::Install { .. } | Commands::Update { .. } => 600,
        _ => 30,
    };

    let (channel, endpoint) = resolve_connection(&cli, timeout_secs).await?;

    match cli.command {
        Commands::Auth { action } => {
            let action = action.expect("None case handled in offline commands");
            auth::handle(channel, action).await
        }
        Commands::Config { action } => config::handle(channel, action).await,
        Commands::Process { action } => {
            let mut client = ProcessServiceClient::new(channel);
            process::handle(&mut client, action).await
        }
        Commands::Vm { action } => {
            let mut client = VmServiceClient::new(channel);
            vm::handle(&mut client, action).await
        }
        Commands::Install { force, config } => {
            let mut client = ProvisionServiceClient::new(channel);
            install::handle(&mut client, force, config, &endpoint).await
        }
        Commands::Update { image } => update::handle(&endpoint, image).await,
        Commands::Disks => {
            let mut client = ProvisionServiceClient::new(channel);
            disks::handle(&mut client).await
        }
        Commands::Logs => {
            let mut client = ProvisionServiceClient::new(channel);
            logs::handle(&mut client).await
        }
        _ => unreachable!("Command not handled"),
    }
}
