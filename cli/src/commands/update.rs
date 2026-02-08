use anyhow::Result;
use owo_colors::OwoColorize;

use crate::client::{
    GetUpdateStatusRequest, PrepareUpdateRequest, ProvisionServiceClient, UpdateRequest,
    UpdateStatus, connect,
};
use crate::config::ServerContext;

/// Handles the update command with polling for completion
pub async fn handle(ctx: &ServerContext, image: Option<String>) -> Result<()> {
    let image = image.unwrap_or_else(|| "ghcr.io/sawangg/installer:latest".to_string());

    println!("{}", format!("Starting update to {image}...").blue());

    let channel = connect(ctx, 600).await?;
    let mut client = ProvisionServiceClient::new(channel);

    let response = client
        .prepare_update(tonic::Request::new(PrepareUpdateRequest {
            image: image.clone(),
        }))
        .await?;
    let resp = response.into_inner();

    if !resp.success {
        eprintln!("{}", format!("Update failed: {}", resp.error).red());
        std::process::exit(1);
    }

    let update_id = resp.update_id.clone();
    println!("{}", format!("Update prepared. ID: {update_id}").green());
    println!("{}", "Triggering update...".yellow());

    let update_channel = connect(ctx, 10).await?;
    let mut update_client = ProvisionServiceClient::new(update_channel);
    if let Ok(response) = update_client
        .update(tonic::Request::new(UpdateRequest {
            update_id: update_id.clone(),
        }))
        .await
    {
        let resp = response.into_inner();
        if !resp.success {
            eprintln!("{}", format!("Update failed: {}", resp.error).red());
            std::process::exit(1);
        }
    }

    println!("{}", "Waiting for system to come back online...".yellow());

    let timeout = std::time::Duration::from_secs(60 * 5);
    let poll_interval = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            eprintln!(
                "{}",
                "Timeout waiting for system to come back online after update".red()
            );
            std::process::exit(1);
        }

        let channel = match connect(ctx, 10).await {
            Ok(c) => c,
            Err(_) => {
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        let mut client = ProvisionServiceClient::new(channel);
        let request = tonic::Request::new(GetUpdateStatusRequest {
            update_id: update_id.clone(),
        });

        match client.get_update_status(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                match UpdateStatus::try_from(resp.status).unwrap_or(UpdateStatus::Unknown) {
                    UpdateStatus::Committed => {
                        println!(
                            "{}",
                            format!("Update {update_id} committed successfully!").green()
                        );
                        return Ok(());
                    }
                    UpdateStatus::RolledBack => {
                        eprintln!(
                            "{}",
                            format!("Update {update_id} rolled back: {}", resp.error).red()
                        );
                        std::process::exit(1);
                    }
                    UpdateStatus::Pending | UpdateStatus::Unknown => {
                        tokio::time::sleep(poll_interval).await;
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
