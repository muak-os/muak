use anyhow::Result;
use clap::Subcommand;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{ListProcessesRequest, ProcessServiceClient};
use crate::format::{format_timestamp, time::TimeSeparator};

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
                println!("{}", "No processes running".yellow());
            } else {
                println!(
                    "{}",
                    format!("{:<8} {:<20} {:<15} STARTED", "PID", "COMMAND", "STATUS")
                        .green()
                        .bold()
                );
                for p in resp.processes {
                    let started = format_timestamp(p.started_at, TimeSeparator::Display);

                    println!(
                        "{:<8} {:<20} {:<15} {}",
                        p.pid, p.command, p.status, started
                    );
                }
            }
        }
    }
    Ok(())
}
