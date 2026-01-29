pub mod auth;
pub mod config;
pub mod disks;
pub mod install;
pub mod logs;
pub mod process;
pub mod update;
pub mod vm;

use anyhow::Result;

use crate::client::{ProcessServiceClient, ProvisionServiceClient, VmServiceClient, connect};
use crate::{Cli, Commands, ConfigAction};

/// Routes CLI commands to their handlers.
pub async fn run(cli: Cli) -> Result<()> {
    // Handle offline commands first
    if let Commands::Config {
        action: ConfigAction::Generate,
    } = &cli.command
    {
        let default = sysconfig::serialize_default();
        print!("{default}");
        return Ok(());
    }

    // Handle auth commands separately (they manage their own connection)
    if let Commands::Auth { action } = cli.command {
        return auth::handle(&cli.server, action).await;
    }

    let timeout_secs = match &cli.command {
        Commands::Install { .. } | Commands::Update { .. } => 600,
        _ => 30,
    };

    let channel = connect(&cli.server, timeout_secs).await?;

    match cli.command {
        Commands::Process { action } => {
            let mut client = ProcessServiceClient::new(channel);
            process::handle(&mut client, action).await?;
        }
        Commands::Vm { action } => {
            let mut client = VmServiceClient::new(channel);
            vm::handle(&mut client, action).await?;
        }
        Commands::Config { action } => {
            config::handle(channel, action).await?;
        }
        Commands::Install { force, config } => {
            let mut client = ProvisionServiceClient::new(channel);
            install::handle(&mut client, force, config).await?;
        }
        Commands::Update { image } => {
            update::handle(&cli.server, image).await?;
        }
        Commands::Disks => {
            let mut client = ProvisionServiceClient::new(channel);
            disks::handle(&mut client).await?;
        }
        Commands::Logs => {
            let mut client = ProvisionServiceClient::new(channel);
            logs::handle(&mut client).await?;
        }
        Commands::Auth { .. } => unreachable!(),
    }

    Ok(())
}
