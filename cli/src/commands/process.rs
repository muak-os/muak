use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;
use tui::style::Role;
use tui::table::Table;

use crate::client::process_service::ListProcessesRequest;
use crate::client::process_service::process_service_client::ProcessServiceClient;
use crate::console::Console;
use crate::format::time::format_timestamp;

#[derive(Subcommand, Clone)]
pub enum Action {
    List,
}

/// Handles process subcommands.
pub async fn handle(
    client: &mut ProcessServiceClient<Channel>,
    action: Action,
    console: &Console,
) -> Result<()> {
    match action {
        Action::List => list(client, console).await,
    }
}

/// Lists running processes.
async fn list(client: &mut ProcessServiceClient<Channel>, console: &Console) -> Result<()> {
    let request = tonic::Request::new(ListProcessesRequest {});

    let response = client.list_processes(request).await?;
    let resp = response.into_inner();

    if resp.processes.is_empty() {
        console.line(Role::Warn, "No processes running");
        return Ok(());
    }

    let mut table = Table::new().header(&["PID", "COMMAND", "STATUS", "STARTED"]);

    for process in resp.processes {
        let started = format_timestamp(process.started_at);
        let pid_str = process.pid.to_string();
        table = table.row(&[&pid_str, &process.command, &process.status, &started]);
    }

    console.table(&table)?;

    Ok(())
}
