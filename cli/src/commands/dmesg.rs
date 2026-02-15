use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{GetLogsRequest, ProvisionServiceClient};

/// Streams kernel logs (dmesg) from the server.
pub async fn handle(client: &mut ProvisionServiceClient<Channel>) -> Result<()> {
    let request = tonic::Request::new(GetLogsRequest {});

    let mut stream = client.get_logs(request).await?.into_inner();

    while let Some(response) = stream.message().await? {
        if !response.error.is_empty() {
            eprintln!("{}", format!("Error: {}", response.error).red());
            std::process::exit(1);
        }
        if !response.line.is_empty() {
            println!("{}", response.line);
        }
    }

    Ok(())
}
