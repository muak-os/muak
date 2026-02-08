pub mod auth;
pub mod config;
pub mod context;
pub mod disks;
pub mod install;
pub mod logs;
pub mod process;
pub mod reset;
pub mod security;
pub mod update;
pub mod vm;

use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{
    ProcessServiceClient, ProvisionServiceClient, SecurityServiceClient, VmServiceClient, connect,
    connect_tls_insecure,
};
use crate::config::ClientConfig;
use crate::{Cli, Commands};

/// Resolves the connection based on CLI flags and config.
///
/// Priority:
/// 1. `--endpoint` flag: maintenance mode (TOFU TLS, no client cert)
/// 2. `--context` flag or MUAK_CONTEXT env: use specified context
/// 3. Config default context: use current context from config
///
/// Returns (Channel, endpoint_address, context).
async fn resolve_connection(
    cli: &Cli,
    timeout_secs: u64,
) -> Result<(Channel, String, Option<String>)> {
    if let Some(endpoint) = &cli.endpoint {
        let channel = connect_tls_insecure(endpoint, timeout_secs).await?;
        return Ok((channel, endpoint.clone(), None));
    }

    let config = ClientConfig::load()?;

    let context = cli
        .context
        .as_ref()
        .or(config.context.as_ref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No context configured. Run 'muakctl install' or 'muakctl context add' first."
            )
        })?
        .clone();

    let ctx = config.get_context(&context).ok_or_else(|| {
        anyhow::anyhow!(
            "Context '{}' not found. Run 'muakctl context list' to see available contexts.",
            context
        )
    })?;

    let channel = connect(ctx, timeout_secs).await?;
    Ok((channel, ctx.endpoint.clone(), Some(context)))
}

/// Routes CLI commands to their handlers.
pub async fn run(cli: Cli) -> Result<()> {
    if handle_offline_cmd(&cli).await? {
        return Ok(());
    }

    let timeout_secs = match &cli.command {
        Commands::Install { .. } | Commands::Update { .. } | Commands::Reset { .. } => 600,
        _ => 30,
    };

    let (channel, endpoint, context_name) = resolve_connection(&cli, timeout_secs).await?;

    handle_cmd(cli, channel, endpoint, context_name).await
}

/// Handles commands that don't require a server connection.
/// Returns true if the command was handled offline.
async fn handle_offline_cmd(cli: &Cli) -> Result<bool> {
    match &cli.command {
        Commands::Config { action } => {
            if matches!(action, config::ConfigAction::Generate) {
                print!("{}", sysconfig::serialize_default());
                return Ok(true);
            }
        }
        Commands::Context { action } => {
            context::handle(action.clone())?;
            return Ok(true);
        }
        Commands::Auth { action: None } => {
            let endpoint = cli.endpoint.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing --endpoint. Use 'muakctl auth --endpoint <ip>:<port>' to authenticate."
                )
            })?;
            auth::enroll(endpoint).await?;
            return Ok(true);
        }
        Commands::Install { force: false, .. } => {
            if let Some(endpoint) = &cli.endpoint {
                let config = ClientConfig::load()?;
                if config.has_credentials_for_endpoint(endpoint) {
                    println!(
                        "{}",
                        "Existing credentials found for this server. Remove the context to reinstall.".yellow()
                    );
                    return Ok(true);
                }
            }
        }
        _ => {}
    }

    Ok(false)
}

/// Handles commands that require a server connection.
async fn handle_cmd(
    cli: Cli,
    channel: Channel,
    endpoint: String,
    context: Option<String>,
) -> Result<()> {
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
        Commands::Security { action } => {
            let mut client = SecurityServiceClient::new(channel);
            security::handle(&mut client, action).await
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
        Commands::Reset { force } => {
            let mut client = ProvisionServiceClient::new(channel);
            reset::handle(&mut client, force, context).await
        }
        _ => unreachable!("Command not handled"),
    }
}
