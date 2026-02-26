use anyhow::Result;

use crate::client::{
    GetUpdateStatusRequest, PrepareUpdateRequest, ProvisionServiceClient, UpdateRequest,
    UpdateStatus, connect,
};
use crate::config::ServerContext;
use crate::ui;

/// Handles the update command with polling for completion
pub async fn handle(ctx: &ServerContext, image: Option<String>) -> Result<()> {
    let image = image.unwrap_or_else(|| "ghcr.io/sawangg/installer:latest".to_string());

    let steps = ui::Steps::new();
    let prepare_msg = format!("Preparing update to {image}...");
    steps.start(&prepare_msg);

    let channel = connect(ctx, 600).await?;
    let mut client = ProvisionServiceClient::new(channel);

    let response = client
        .prepare_update(tonic::Request::new(PrepareUpdateRequest {
            image: image.clone(),
        }))
        .await?;
    let resp = response.into_inner();

    if !resp.success {
        let msg = format!("Update failed: {}", resp.error);
        steps.fail(&msg);
        steps.finish().await;
        std::process::exit(1);
    }

    let update_id = resp.update_id.clone();
    let prepared_msg = format!("Update prepared. ID: {update_id}");
    steps.complete(&prepared_msg);

    steps.start("Triggering update...");

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
            let msg = format!("Update failed: {}", resp.error);
            steps.fail(&msg);
            steps.finish().await;
            std::process::exit(1);
        }
    }

    steps.start("Waiting for system to come back online...");

    let timeout = std::time::Duration::from_secs(60 * 5);
    let poll_interval = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            steps.fail("Timeout waiting for system to come back online after update");
            steps.finish().await;
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
                        let msg = format!("Update {update_id} committed successfully!");
                        steps.complete(&msg);
                        steps.finish().await;
                        return Ok(());
                    }
                    UpdateStatus::RolledBack => {
                        let msg = format!("Update {update_id} rolled back: {}", resp.error);
                        steps.fail(&msg);
                        steps.finish().await;
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
