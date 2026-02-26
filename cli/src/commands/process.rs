use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{ListProcessesRequest, ProcessServiceClient};
use crate::format::{format_timestamp, time::TimeSeparator};
use crate::ui;

#[derive(Subcommand)]
pub enum ProcessAction {
    List,
}

/// Handles process subcommands.
pub async fn handle(
    client: &mut ProcessServiceClient<Channel>,
    action: ProcessAction,
) -> Result<()> {
    match action {
        ProcessAction::List => {
            let request = tonic::Request::new(ListProcessesRequest {});

            let response = client.list_processes(request).await?;
            let resp = response.into_inner();

            if resp.processes.is_empty() {
                println!("{}", ui::style::warn("No processes running"));
            } else {
                let mut table = ui::Table::new().header(&["PID", "COMMAND", "STATUS", "STARTED"]);

                for p in resp.processes {
                    let started = format_timestamp(p.started_at, TimeSeparator::Display);
                    let pid_str = p.pid.to_string();
                    table = table.row(&[&pid_str, &p.command, &p.status, &started]);
                }

                table.print();
            }
        }
    }
    Ok(())
}
