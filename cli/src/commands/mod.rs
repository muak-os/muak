pub mod auth;
pub mod config;
pub mod context;
pub mod disks;
pub mod dmesg;
pub mod install;
pub mod logs;
pub mod process;
pub mod reset;
pub mod rollback;
pub mod security;
pub mod update;
pub mod version;
pub mod vm;

use ::config::ClientConfig;
use anyhow::{Result, bail};
use tonic::transport::Channel;

use crate::client::{
    connect, connect_tls_insecure,
    log_service::log_service_client::LogServiceClient,
    process_service::process_service_client::ProcessServiceClient,
    provision_service::provision_service_client::ProvisionServiceClient,
    security_service::security_service_client::SecurityServiceClient,
    version_service::{GetVersionRequest, version_service_client::VersionServiceClient},
    vm_service::vm_service_client::VmServiceClient,
};
use crate::ui;
use crate::{Cli, Commands};

/// Resolves the connection based on CLI flags and config.
///
/// Priority:
/// 1. `--insecure` flag: TOFU TLS mode (no cert verification, no client cert).
///    Requires `--endpoint` to specify the server address.
/// 2. `--endpoint` flag (without `--insecure`): use context credentials with
///    the provided endpoint address override.
/// 3. `--context` flag or `MUAK_CONTEXT` env: use specified context.
/// 4. Config default context: use current context from config.
///
/// Returns (`Channel`, `endpoint_address`, `context`).
async fn resolve_connection(
    cli: &Cli,
    timeout_secs: u64,
    skip_preflight: bool,
) -> Result<(Channel, String, Option<String>)> {
    if cli.insecure {
        let endpoint = cli.endpoint.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--insecure requires --endpoint to specify the server address.")
        })?;
        let channel = connect_tls_insecure(endpoint, timeout_secs).await?.0;
        if !skip_preflight {
            run_version_preflight(channel.clone()).await;
        }
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
            "Context '{context}' not found. Run 'muakctl context list' to see available contexts."
        )
    })?;

    let endpoint = cli.endpoint.as_deref().unwrap_or(&ctx.endpoint).to_owned();

    let channel = if cli.endpoint.is_some() {
        let mut ctx_override = ctx.clone();
        ctx_override.endpoint = endpoint.clone();
        connect(&ctx_override, timeout_secs).await?
    } else {
        connect(ctx, timeout_secs).await?
    };

    if !skip_preflight {
        run_version_preflight(channel.clone()).await;
    }
    Ok((channel, endpoint, Some(context)))
}

/// Checks CLI vs server version and prints a warning to stderr on mismatch.
async fn run_version_preflight(channel: Channel) {
    let client_ver = env!("CARGO_PKG_VERSION");
    let mut vc = VersionServiceClient::new(channel);
    if let Ok(resp) = vc.get_version(GetVersionRequest {}).await {
        let server_ver = resp.into_inner().version;
        version::print_compat_warning(client_ver, &server_ver);
    }
}

/// Routes CLI commands to their handlers.
pub async fn run(cli: Cli) -> Result<()> {
    if handle_offline_cmd(&cli).await? {
        return Ok(());
    }

    let timeout_secs = if matches!(
        &cli.command,
        Commands::Install { .. } | Commands::Update { .. } | Commands::Reset { .. }
    ) {
        600
    } else if matches!(
        &cli.command,
        Commands::Logs { follow: true, .. } | Commands::Dmesg { follow: true }
    ) {
        86400
    } else {
        30
    };
    let skip_preflight = cli.skip_version_check || matches!(cli.command, Commands::Version);
    let (channel, endpoint, context_name) =
        resolve_connection(&cli, timeout_secs, skip_preflight).await?;

    handle_cmd(cli, channel, endpoint, context_name).await
}

/// Handles commands that don't require a server connection.
async fn handle_offline_cmd(cli: &Cli) -> Result<bool> {
    match cli.command.clone() {
        Commands::Config {
            action: config::Action::Generate,
        } => {
            print!("{}", ::config::serialize_default());
            Ok(true)
        }
        Commands::Context { action } => {
            context::handle(action)?;
            Ok(true)
        }
        Commands::Auth { action: None } => {
            let endpoint = cli.endpoint.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing --endpoint. Use 'muakctl auth --endpoint <ip>:<port> --insecure' to authenticate."
                )
            })?;
            if !cli.insecure {
                bail!(
                    "Authentication enrollment requires --insecure. Use 'muakctl auth --endpoint <ip>:<port> --insecure'."
                );
            }
            auth::enroll(endpoint).await?;
            Ok(true)
        }
        Commands::Update { image, config } => {
            if image.is_some() && config.is_some() {
                bail!("--image and --config are mutually exclusive!");
            }
            if image.is_none() && config.is_none() {
                bail!("Either --image or --config must be provided for update!");
            }
            Ok(false)
        }
        Commands::Install { force: false, .. } if has_existing_credentials(cli)? => {
            println!(
                "{}",
                ui::style::warn(
                    "Existing credentials found for this server. Remove the context to reinstall."
                )
            );
            Ok(true)
        }
        Commands::Config { .. }
        | Commands::Auth { action: Some(_) }
        | Commands::Install { .. }
        | Commands::Process { .. }
        | Commands::Security { .. }
        | Commands::Vm { .. }
        | Commands::Rollback { .. }
        | Commands::Disks
        | Commands::Dmesg { .. }
        | Commands::Logs { .. }
        | Commands::Reset { .. }
        | Commands::Version => Ok(false),
    }
}

/// Returns true when the user already has credentials for the target endpoint.
fn has_existing_credentials(cli: &Cli) -> Result<bool> {
    if !cli.insecure {
        return Ok(false);
    }
    let Some(endpoint) = cli.endpoint.as_ref() else {
        return Ok(false);
    };
    let config = ClientConfig::load()?;

    Ok(config.has_credentials_for_endpoint(endpoint))
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
            let action =
                action.ok_or_else(|| anyhow::anyhow!("Auth action missing in online handler"))?;
            auth::handle(channel, action).await
        }
        Commands::Config { action } => config::handle(channel, action).await,
        Commands::Rollback { action } => rollback::handle(channel, action).await,
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
        Commands::Update {
            image,
            config: config_path,
        } => {
            let ctx_name = context.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "'update' requires an installed system and cannot be used in maintenance mode!"
                )
            })?;
            let config = ClientConfig::load()?;
            let ctx = config
                .get_context(ctx_name)
                .ok_or_else(|| anyhow::anyhow!("Context '{ctx_name}' not found."))?;
            update::handle(ctx, image, config_path).await
        }
        Commands::Disks => {
            let mut client = ProvisionServiceClient::new(channel);
            disks::handle(&mut client).await
        }
        Commands::Dmesg { follow } => {
            let mut client = LogServiceClient::new(channel);
            dmesg::handle(&mut client, follow).await
        }
        Commands::Logs {
            service,
            tail,
            follow,
            level,
        } => {
            let mut client = LogServiceClient::new(channel);
            logs::handle(&mut client, service, tail, follow, level).await
        }
        Commands::Reset { force } => {
            let mut client = ProvisionServiceClient::new(channel);
            reset::handle(&mut client, force, context).await
        }
        Commands::Version => version::handle(channel).await,
        Commands::Context { .. } => bail!("Command not handled"),
    }
}
