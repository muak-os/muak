use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::process_service::{
    ListProcessesRequest, process_service_client::ProcessServiceClient,
};
use crate::format::time::{Separator, format_timestamp};
use crate::ui;

#[derive(Subcommand, Clone)]
pub enum Action {
    List,
}

/// Handles process subcommands.
pub async fn handle(client: &mut ProcessServiceClient<Channel>, action: Action) -> Result<()> {
    match action {
        Action::List => list(client).await,
    }
}

/// Lists running processes.
async fn list(client: &mut ProcessServiceClient<Channel>) -> Result<()> {
    let request = tonic::Request::new(ListProcessesRequest {});

    let response = client.list_processes(request).await?;
    let resp = response.into_inner();

    if resp.processes.is_empty() {
        println!("{}", ui::style::warn("No processes running"));
        return Ok(());
    }

    let mut table = ui::table::Table::new().header(&["PID", "COMMAND", "STATUS", "STARTED"]);

    for process in resp.processes {
        let started = format_timestamp(process.started_at, Separator::Display);
        let pid_str = process.pid.to_string();
        table = table.row(&[&pid_str, &process.command, &process.status, &started]);
    }

    table.print();
    Ok(())
}
