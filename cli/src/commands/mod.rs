pub mod auth;
pub mod config;
pub mod context;
pub mod disks;
pub mod dmesg;
pub mod install;
pub mod logs;
pub mod power;
pub mod process;
pub mod reset;
pub mod rollback;
pub mod security;
pub mod update;
pub mod version;
pub mod vm;

use ::config::user::ClientConfig;
use anyhow::{Result, bail};
use tonic::transport::Channel;
use tui::style::Role;

use crate::client::log_service::log_service_client::LogServiceClient;
use crate::client::process_service::process_service_client::ProcessServiceClient;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::security_service::security_service_client::SecurityServiceClient;
use crate::client::version_service::GetVersionRequest;
use crate::client::version_service::version_service_client::VersionServiceClient;
use crate::client::vm_service::vm_service_client::VmServiceClient;
use crate::client::{connect, connect_tls_insecure};
use crate::console::Console;
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
    console: &Console,
) -> Result<(Channel, String, Option<String>)> {
    if cli.insecure {
        let endpoint = cli.endpoint.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--insecure requires --endpoint to specify the server address.")
        })?;
        let channel = connect_tls_insecure(endpoint, timeout_secs).await?.0;
        if !skip_preflight {
            run_version_preflight(channel.clone(), console).await;
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
        run_version_preflight(channel.clone(), console).await;
    }
    Ok((channel, endpoint, Some(context)))
}

/// Checks CLI vs server version and prints a warning to stderr on mismatch.
async fn run_version_preflight(channel: Channel, console: &Console) {
    let client_ver = env!("CARGO_PKG_VERSION");
    let mut vc = VersionServiceClient::new(channel);
    if let Ok(resp) = vc.get_version(GetVersionRequest {}).await {
        let server_ver = resp.into_inner().version;
        version::print_compat_warning(console, client_ver, &server_ver);
    }
}

/// Routes CLI commands to their handlers.
pub async fn run(cli: Cli, console: &Console) -> Result<()> {
    if handle_offline_cmd(&cli, console).await? {
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
        resolve_connection(&cli, timeout_secs, skip_preflight, console).await?;

    handle_cmd(cli, channel, endpoint, context_name, console).await
}

/// Handles commands that don't require a server connection.
async fn handle_offline_cmd(cli: &Cli, console: &Console) -> Result<bool> {
    match cli.command.clone() {
        Commands::Config {
            action: config::Action::Generate,
        } if !cli.insecure => {
            print!("{}", ::config::system::serialize_default());
            Ok(true)
        }
        Commands::Context { action } => {
            context::handle(action, console)?;
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
            auth::enroll(endpoint, console).await?;
            Ok(true)
        }
        Commands::Update { version, config } => {
            if version.is_some() && config.is_some() {
                bail!("--version and --config are mutually exclusive!");
            }
            Ok(false)
        }
        Commands::Install { force: false, .. } if has_existing_credentials(cli)? => {
            console.line(
                Role::Warn,
                "Existing credentials found for this server. Remove the context to reinstall.",
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
        | Commands::Reboot
        | Commands::Shutdown
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
    console: &Console,
) -> Result<()> {
    match cli.command {
        Commands::Auth { action } => {
            let action =
                action.ok_or_else(|| anyhow::anyhow!("Auth action missing in online handler"))?;
            auth::handle(channel, action, console).await
        }
        Commands::Config { action } => config::handle(channel, action, console).await,
        Commands::Rollback { action } => rollback::handle(channel, action, console).await,
        Commands::Process { action } => {
            let mut client = ProcessServiceClient::new(channel);
            process::handle(&mut client, action, console).await
        }
        Commands::Security { action } => {
            let mut client = SecurityServiceClient::new(channel);
            security::handle(&mut client, action, console).await
        }
        Commands::Vm { action } => {
            let mut client = VmServiceClient::new(channel);
            vm::handle(&mut client, action, console).await
        }
        Commands::Install { force, config } => {
            let mut client = ProvisionServiceClient::new(channel);
            install::handle(&mut client, force, config, &endpoint, console).await
        }
        Commands::Update {
            version,
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
            update::handle(ctx, version, config_path, console).await
        }
        Commands::Disks => {
            let mut client = ProvisionServiceClient::new(channel);
            disks::handle(&mut client, console).await
        }
        Commands::Dmesg { follow } => {
            let mut client = LogServiceClient::new(channel);
            dmesg::handle(&mut client, follow, console).await
        }
        Commands::Logs {
            service,
            tail,
            follow,
            level,
        } => {
            let mut client = LogServiceClient::new(channel);
            logs::handle(&mut client, service, tail, follow, level, console).await
        }
        Commands::Reset { force } => {
            let mut client = ProvisionServiceClient::new(channel);
            reset::handle(&mut client, force, context, console).await
        }
        Commands::Reboot => {
            let mut client = ProvisionServiceClient::new(channel);
            power::reboot(&mut client, console).await
        }
        Commands::Shutdown => {
            let mut client = ProvisionServiceClient::new(channel);
            power::shutdown(&mut client, console).await
        }
        Commands::Version => version::handle(channel, console).await,
        Commands::Context { .. } => bail!("Command not handled"),
    }
}
