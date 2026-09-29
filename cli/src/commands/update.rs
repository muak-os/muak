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
    version: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    if version.is_some() && config_path.is_some() {
        bail!("--version and --config are mutually exclusive!");
    }

    let channel = connect(ctx, 600).await?;
    let mut client = ProvisionServiceClient::new(channel);

    let installed = fetch_installed_config(&mut client).await?;

    let (version_str, config_bytes) = if let Some(ref path) = config_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;

        let cfg = config::parse_from_str(&raw)
            .with_context(|| format!("invalid format in '{}'", path.display()))?;

        cfg.validate_for_update(&installed)
            .with_context(|| format!("config rejected: '{}'", path.display()))?;

        config::check_no_downgrade(&cfg.host.version, &installed.host.version)
            .with_context(|| format!("version check failed for '{}'", path.display()))?;

        (String::new(), raw.into_bytes())
    } else {
        let target = version.clone().unwrap_or_default();
        if !target.is_empty() {
            config::check_no_downgrade(&target, &installed.host.version)
                .with_context(|| format!("version check failed for '{target}'"))?;
        }

        // An empty version makes the server follow the machine's channel.
        (target, Vec::new())
    };

    let steps = ui::steps::Steps::new();

    let response = client
        .prepare_update(tonic::Request::new(PrepareUpdateRequest {
            version: version_str,
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

enum PollOutcome {
    Done,
    Failed(String),
    Pending,
    Unknown,
    Unreachable(String),
}

const UNKNOWN_WARN_THRESHOLD: u32 = 30;

struct PollFeedback {
    unreachable: u32,
    unknown: u32,
    unknown_warned: bool,
}

impl PollFeedback {
    fn on_pending(&mut self, steps: &ui::steps::Steps) {
        if self.unreachable > 0 {
            steps.start(format!(
                "Update pending (reachable again after {} failed poll(s))",
                self.unreachable
            ));
        }
        self.unreachable = 0;
        self.unknown = 0;
        self.unknown_warned = false;
    }

    fn on_unknown(&mut self, steps: &ui::steps::Steps, update_id: &str) {
        self.unreachable = 0;
        self.unknown = self.unknown.saturating_add(1);
        if self.unknown >= UNKNOWN_WARN_THRESHOLD && !self.unknown_warned {
            self.unknown_warned = true;
            steps.start(format!(
                "Server does not recognize update {update_id}; it may have rolled \
                 back without a journal entry. Check 'muakctl rollback history'."
            ));
        }
    }

    fn on_unreachable(&mut self, steps: &ui::steps::Steps, err: &str) {
        self.unreachable = self.unreachable.saturating_add(1);
        if self.unreachable == 1 || self.unreachable.is_multiple_of(5) {
            steps.start(format!(
                "Waiting for system ({} failed polls): {err}",
                self.unreachable
            ));
        }
    }
}

async fn wait_for_update_completion(
    ctx: &ServerContext,
    update_id: &str,
    steps: ui::steps::Steps,
) -> Result<()> {
    let timeout = Duration::from_mins(5);
    let poll_interval = Duration::from_secs(2);
    let start = Instant::now();
    let mut feedback = PollFeedback {
        unreachable: 0,
        unknown: 0,
        unknown_warned: false,
    };

    loop {
        if start.elapsed() > timeout {
            steps.fail("Timeout waiting for system to come back online after update");
            steps.finish().await;
            return Err(anyhow::anyhow!(
                "Timed out waiting for update {update_id} to complete. The machine may \
                 still be booting, or may have rolled back - check `muakctl rollback \
                 history` and `muakctl config history`."
            ));
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
            PollOutcome::Pending => feedback.on_pending(&steps),
            PollOutcome::Unknown => feedback.on_unknown(&steps, update_id),
            PollOutcome::Unreachable(err) => feedback.on_unreachable(&steps, &err),
        }

        sleep(poll_interval).await;
    }
}

async fn poll_update_status(
    ctx: &ServerContext,
    update_id: &str,
    steps: &ui::steps::Steps,
) -> PollOutcome {
    let channel = match connect(ctx, 10).await {
        Ok(channel) => channel,
        Err(err) => return PollOutcome::Unreachable(format!("connect failed: {err}")),
    };

    let mut client = ProvisionServiceClient::new(channel);
    let request = tonic::Request::new(GetUpdateStatusRequest {
        update_id: update_id.to_owned(),
    });

    let resp = match client.get_update_status(request).await {
        Ok(response) => response.into_inner(),
        Err(err) => return PollOutcome::Unreachable(format!("status rpc failed: {err}")),
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
        UpdateStatus::Pending => PollOutcome::Pending,
        UpdateStatus::Unknown => PollOutcome::Unknown,
    }
}

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
