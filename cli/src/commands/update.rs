use core::time::Duration;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use config::ServerContext;
use tokio::time::sleep;
use tokio_stream::StreamExt as _;
use tonic::transport::Channel;

use crate::client::{
    connect,
    provision_service::{
        GetConfigRequest, GetUpdateStatusRequest, PrepareUpdateRequest, UpdateRequest,
        UpdateStatus, provision_service_client::ProvisionServiceClient,
    },
};
use crate::ui;

/// Handles the update command.
pub async fn handle(
    ctx: &ServerContext,
    image: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if image.is_some() && config_path.is_some() {
        bail!("--image and --config are mutually exclusive!");
    }

    let channel = connect(ctx, 600).await?;
    let mut client = ProvisionServiceClient::new(channel);

    let installed = fetch_installed_config(&mut client).await?;

    let (image_str, config_bytes) = if let Some(ref path) = config_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;

        let cfg = config::parse_from_str(&raw)
            .with_context(|| format!("invalid format in '{}'", path.display()))?;

        cfg.validate_for_update(&installed)
            .with_context(|| format!("config rejected: '{}'", path.display()))?;

        config::check_no_downgrade(&cfg.host.image, &installed.host.image)
            .with_context(|| format!("version check failed for '{}'", path.display()))?;

        (String::new(), raw.into_bytes())
    } else if let Some(ref img) = image {
        config::check_no_downgrade(img, &installed.host.image)
            .with_context(|| format!("version check failed for image '{img}'"))?;

        (img.clone(), Vec::new())
    } else {
        bail!("Either --image or --config must be provided for update!");
    };

    let steps = ui::steps::Steps::new();

    let response = client
        .prepare_update(tonic::Request::new(PrepareUpdateRequest {
            image: image_str,
            config: config_bytes,
        }))
        .await
        .context("Failed to send prepare_update request")?;

    let mut stream = response.into_inner();
    let mut update_id = String::new();

    while let Some(progress) = stream.next().await {
        let progress = progress.context("Error receiving prepare_update progress")?;

        if !progress.error.is_empty() {
            let msg = format!("Update preparation failed: {}", progress.error);
            steps.fail(&msg);
            steps.finish().await;
            return Err(anyhow::anyhow!("{msg}"));
        }

        if progress.update_id.is_empty() {
            steps.start(&progress.message);
        } else {
            update_id = progress.update_id;
        }
    }

    if update_id.is_empty() {
        steps.fail("Prepare update stream ended without completion");
        steps.finish().await;
        return Err(anyhow::anyhow!(
            "Prepare update stream ended without completion"
        ));
    }

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
            return Err(anyhow::anyhow!("{msg}"));
        }
    }

    steps.start("Waiting for system to come back online...");

    wait_for_update_completion(ctx, &update_id, steps).await
}

/// Outcome of a single update-status poll.
enum PollOutcome {
    Done,
    Failed(String),
    Retry,
}

/// Polls the server until the update reaches a terminal state.
async fn wait_for_update_completion(
    ctx: &ServerContext,
    update_id: &str,
    steps: ui::steps::Steps,
) -> Result<()> {
    let timeout = Duration::from_mins(5);
    let poll_interval = Duration::from_secs(2);
    let start = Instant::now();

    loop {
        if start.elapsed() > timeout {
            steps.fail("Timeout waiting for system to come back online after update");
            steps.finish().await;
            return Err(anyhow::anyhow!("Timed out waiting for update to complete"));
        }

        match poll_update_status(ctx, update_id, &steps).await {
            PollOutcome::Done => {
                steps.finish().await;
                return Ok(());
            }
            PollOutcome::Failed(msg) => {
                steps.fail(&msg);
                steps.finish().await;
                return Err(anyhow::anyhow!("{msg}"));
            }
            PollOutcome::Retry => {
                sleep(poll_interval).await;
            }
        }
    }
}

/// Polls the update status once and decides whether to keep waiting.
async fn poll_update_status(
    ctx: &ServerContext,
    update_id: &str,
    steps: &ui::steps::Steps,
) -> PollOutcome {
    let Ok(channel) = connect(ctx, 10).await else {
        return PollOutcome::Retry;
    };

    let mut client = ProvisionServiceClient::new(channel);
    let request = tonic::Request::new(GetUpdateStatusRequest {
        update_id: update_id.to_owned(),
    });

    let resp = match client.get_update_status(request).await {
        Ok(response) => response.into_inner(),
        Err(_) => return PollOutcome::Retry,
    };

    match UpdateStatus::try_from(resp.status).unwrap_or(UpdateStatus::Unknown) {
        UpdateStatus::Committed => {
            let msg = format!("Update {update_id} committed successfully!");
            steps.complete(&msg);
            PollOutcome::Done
        }
        UpdateStatus::RolledBack => {
            let msg = format!("Update {update_id} rolled back: {}", resp.error);
            steps.fail(&msg);
            PollOutcome::Failed(msg)
        }
        UpdateStatus::Pending | UpdateStatus::Unknown => PollOutcome::Retry,
    }
}

/// Fetches and parses the installed config from the server.
async fn fetch_installed_config(
    client: &mut ProvisionServiceClient<Channel>,
) -> Result<config::SystemConfig> {
    let resp = client
        .get_config(tonic::Request::new(GetConfigRequest {}))
        .await
        .context("Failed to fetch installed config from server")?
        .into_inner();

    if !resp.error.is_empty() {
        bail!("Server returned error fetching config: {}", resp.error);
    }

    let raw = String::from_utf8(resp.config).context("Server returned non-UTF-8 config")?;

    config::parse_from_str(&raw).context("Failed to parse installed config from server")
}
