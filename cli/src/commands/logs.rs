use anyhow::Result;
use tonic::transport::Channel;

use crate::client::LogServiceClient;
use crate::client::log_service::{
    self, FollowLogsRequest, GetLogsRequest, GetLogsResponse, LogEntry,
};
use crate::ui;

/// Fetches and displays service logs.
pub async fn handle(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
    tail: Option<u32>,
    follow: bool,
) -> Result<()> {
    if follow {
        handle_follow(client, service).await
    } else {
        handle_get(client, service, tail.unwrap_or(0)).await
    }
}

/// Fetches a batch of recent logs.
async fn handle_get(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
    tail: u32,
) -> Result<()> {
    let request = tonic::Request::new(GetLogsRequest {
        service: service.unwrap_or_default(),
        tail,
    });

    let GetLogsResponse { entries } = client.get_logs(request).await?.into_inner();

    for entry in &entries {
        print_entry(entry);
    }

    Ok(())
}

/// Follows live log output (like `tail -f` / `dmesg -w`).
async fn handle_follow(
    client: &mut LogServiceClient<Channel>,
    service: Option<String>,
) -> Result<()> {
    let request = tonic::Request::new(FollowLogsRequest {
        service: service.unwrap_or_default(),
    });

    let mut stream = client.follow_logs(request).await?.into_inner();

    while let Some(entry) = stream.message().await? {
        print_entry(&entry);
    }

    Ok(())
}

/// Prints a log entry. Stderr lines are printed in red.
fn print_entry(entry: &LogEntry) {
    let line = format!("[{}] {}", entry.service, entry.message);

    if entry.stream() == log_service::Stream::Stderr {
        println!("{}", ui::style::error_text(&line));
    } else {
        println!("{line}");
    }
}
