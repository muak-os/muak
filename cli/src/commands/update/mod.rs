mod progress;
mod trigger;

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use config::system::Config;
use config::user::ServerContext;
use tokio::time::sleep;
use tokio_stream::StreamExt as _;
use tonic::transport::Channel;
use tui::console::Console;
use tui::live::steps::Steps;

use self::progress::Progress;
use self::trigger::{Outcome, classify, failure_message, reboot_notice};
use crate::client::connect;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{
    GetConfigRequest, GetUpdateStatusRequest, PrepareUpdateRequest, UpdateRequest, UpdateStatus,
};

const UNKNOWN_WARN_THRESHOLD: u32 = 30;

enum PollOutcome {
    Done,
    Failed(String),
    Pending,
    Unknown,
    Unreachable(String),
}

struct PollFeedback {
    unreachable: u32,
    unknown: u32,
    unknown_warned: bool,
}

#[derive(Debug)]
struct Payload {
    version: String,
    config: Vec<u8>,
    warning: Option<String>,
}

impl PollFeedback {
    fn on_pending(&mut self, steps: &Steps) {
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

    fn on_unknown(&mut self, steps: &Steps, update_id: &str) {
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

    fn on_unreachable(&mut self, steps: &Steps, err: &str) {
        self.unreachable = self.unreachable.saturating_add(1);
        if self.unreachable == 1 || self.unreachable.is_multiple_of(5) {
            steps.start(format!(
                "Waiting for system ({} failed polls): {err}",
                self.unreachable
            ));
        }
    }
}

/// Handles the update command.
pub(crate) async fn handle(
    ctx: &ServerContext,
    version: Option<String>,
    config_path: Option<PathBuf>,
    force: bool,
    offline: bool,
    console: &Console,
) -> Result<()> {
    ensure_target(version.as_deref(), config_path.as_deref())?;

    let channel = connect(ctx, 600).await?;
    let mut client = ProvisionServiceClient::new(channel);

    let installed = fetch_installed_config(&mut client).await?;
    let payload = request_payload(version, config_path.as_deref(), &installed)?;
    if let Some(warning) = payload.warning.as_deref() {
        console.warn(warning);
    }

    let steps = console.steps()?;
    let reporter = Progress::new(&steps, console.stderr().verbose);

    let response = client
        .prepare_update(tonic::Request::new(PrepareUpdateRequest {
            version: payload.version,
            config: payload.config,
            force,
            offline,
        }))
        .await
        .context("Failed to send prepare_update request")?;

    let mut stream = response.into_inner();
    let mut update_id = String::new();
    let mut noop = None;

    while let Some(progress) = stream.next().await {
        let progress = progress.context("Error receiving prepare_update progress")?;

        if !progress.error.is_empty() {
            let msg = format!("Update preparation failed: {}", progress.error);
            steps.fail(&msg);
            let _finish_result = steps.finish();
            return Err(anyhow::anyhow!("{msg}"));
        }

        if progress.already_current {
            noop = Some(progress.message);
            continue;
        }

        if progress.update_id.is_empty() {
            match progress.event {
                Some(event) => reporter.report(event),
                None => steps.start(&progress.message),
            }
        } else {
            update_id = progress.update_id;
        }
    }

    if let Some(message) = noop {
        steps.finish()?;
        console.success(&message);

        return Ok(());
    }

    if update_id.is_empty() {
        steps.fail("Prepare update stream ended without completion");
        let _finish_result = steps.finish();
        return Err(anyhow::anyhow!(
            "Prepare update stream ended without completion"
        ));
    }

    let prepared_msg = format!("Update prepared. ID: {update_id}");
    steps.complete(&prepared_msg);

    steps.start("Triggering update...");

    let update_channel = connect(ctx, 10).await?;
    let mut update_client = ProvisionServiceClient::new(update_channel);
    match update_client
        .update(tonic::Request::new(UpdateRequest {
            update_id: update_id.clone(),
        }))
        .await
    {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.success {
                let msg = format!("Update failed: {}", resp.error);
                steps.fail(&msg);
                let _finish_result = steps.finish();
                return Err(anyhow::anyhow!("{msg}"));
            }
        }
        Err(status) => match classify(status.code()) {
            Outcome::AssumeReboot => steps.start(reboot_notice(&status)),
            Outcome::Failed => {
                let msg = failure_message(&status);
                steps.fail(&msg);
                let _finish_result = steps.finish();
                return Err(anyhow::anyhow!("{msg}"));
            }
        },
    }

    steps.start("Waiting for system to come back online...");

    wait_for_update_completion(ctx, &update_id, steps).await?;

    console.success(&format!("Update {update_id} committed successfully!"));

    Ok(())
}

fn request_payload(
    version: Option<String>,
    config_path: Option<&Path>,
    installed: &Config,
) -> Result<Payload> {
    let Some(path) = config_path else {
        let target = version
            .ok_or_else(|| anyhow::anyhow!("--version is required when --config is absent"))?;
        config::version::check_no_downgrade(&target, &installed.host.version)
            .with_context(|| format!("version check failed for '{target}'"))?;

        return Ok(Payload {
            version: target,
            config: Vec::new(),
            warning: None,
        });
    };

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;
    let doc_version = desired_version(&raw)
        .with_context(|| format!("no update target in '{}'", path.display()))?;

    let target = match doc_version {
        Some(doc_version) => {
            if let Some(explicit) = version
                && explicit != doc_version
            {
                bail!(
                    "--version '{explicit}' does not match host.version '{doc_version}' in '{}'",
                    path.display()
                );
            }

            doc_version
        }
        None => version.ok_or_else(|| {
            anyhow::anyhow!(
                "no update target in '{}': set host.version in the config or pass --version",
                path.display()
            )
        })?,
    };

    config::version::check_no_downgrade(&target, &installed.host.version)
        .with_context(|| format!("version check failed for '{}'", path.display()))?;

    let warning = match config::system::parse_from_str(&raw) {
        Ok(cfg) => {
            cfg.validate_for_update(installed)
                .with_context(|| format!("config rejected: '{}'", path.display()))?;
            None
        }
        Err(error) => Some(format!(
            "config '{}' will be validated on the node after boot: {error}",
            path.display()
        )),
    };

    Ok(Payload {
        version: target,
        config: raw.into_bytes(),
        warning,
    })
}

fn desired_version(raw: &str) -> Result<Option<String>> {
    config::document::version(raw).map_err(Into::into)
}

async fn wait_for_update_completion(
    ctx: &ServerContext,
    update_id: &str,
    steps: Steps,
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
            let _finish_result = steps.finish();
            return Err(anyhow::anyhow!(
                "Timed out waiting for update {update_id} to complete. The machine may \
                 still be booting, or may have rolled back - check `muakctl rollback \
                 history` and `muakctl config history`."
            ));
        }

        match poll_update_status(ctx, update_id, &steps).await {
            PollOutcome::Done => {
                steps.finish()?;
                return Ok(());
            }
            PollOutcome::Failed(msg) => {
                steps.fail(&msg);
                let _finish_result = steps.finish();
                return Err(anyhow::anyhow!("{msg}"));
            }
            PollOutcome::Pending => feedback.on_pending(&steps),
            PollOutcome::Unknown => feedback.on_unknown(&steps, update_id),
            PollOutcome::Unreachable(err) => feedback.on_unreachable(&steps, &err),
        }

        sleep(poll_interval).await;
    }
}

async fn poll_update_status(ctx: &ServerContext, update_id: &str, steps: &Steps) -> PollOutcome {
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

/// Requires the update target to be given as `--version`, `--config`, or both.
pub(crate) fn ensure_target(version: Option<&str>, config: Option<&Path>) -> Result<()> {
    match (version, config) {
        (None, None) => bail!("one of --version or --config must be specified!"),
        _ => Ok(()),
    }
}

async fn fetch_installed_config(client: &mut ProvisionServiceClient<Channel>) -> Result<Config> {
    let resp = client
        .get_config(tonic::Request::new(GetConfigRequest {}))
        .await
        .context("Failed to fetch installed config from server")?
        .into_inner();

    if !resp.error.is_empty() {
        bail!("Server returned error fetching config: {}", resp.error);
    }

    let raw = String::from_utf8(resp.config).context("Server returned non-UTF-8 config")?;

    config::system::parse_from_str(&raw).context("Failed to parse installed config from server")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use config::system::Config;

    use super::{desired_version, ensure_target, request_payload};

    fn document(version: &str) -> String {
        format!(
            "api_version = \"{}\"\n\n[host]\nname = \"muak\"\nversion = \"{version}\"\n\
             port = 50051\nntp = \"pool.ntp.org\"\n",
            config::system::API_VERSION
        )
    }

    fn installed(version: &str) -> Config {
        let mut config = Config::default();
        config.host.version = version.to_owned();

        config
    }

    #[test]
    fn target_requires_version_or_config() {
        // ARRANGE
        let config = Path::new("config.toml");

        // ACT
        let neither = ensure_target(None, None);
        let version_only = ensure_target(Some("v1.2.0"), None);
        let config_only = ensure_target(None, Some(config));

        // ASSERT
        neither.expect_err("a missing target must be rejected");
        version_only.expect("a version target is valid");
        config_only.expect("a config target is valid");
    }

    #[test]
    fn target_accepts_version_and_config_together() {
        // ARRANGE
        let config = Path::new("config.toml");

        // ACT
        let both = ensure_target(Some("v1.2.0"), Some(config));

        // ASSERT
        both.expect("a version may accompany a config");
    }

    #[test]
    fn desired_version_tolerates_unknown_keys() {
        // ARRANGE
        let raw = "[host]\nversion = \"v1.2.0\"\n\nfuture = { nested = true }\n";

        // ACT
        let version = desired_version(raw).expect("unknown keys must not matter");

        // ASSERT
        assert_eq!(version.as_deref(), Some("v1.2.0"));
    }

    #[test]
    fn desired_version_tolerates_future_api_version() {
        // ARRANGE
        let raw = "api_version = \"muak.dev/config/v9-experimental\"\n\
                   [host]\nversion = \"v2.0.0\"\n";

        // ACT
        let version = desired_version(raw).expect("the api_version must not matter");

        // ASSERT
        assert_eq!(version.as_deref(), Some("v2.0.0"));
    }

    #[test]
    fn desired_version_ignores_a_missing_or_empty_version() {
        // ARRANGE
        let missing = "[host]\nname = \"muak\"\n";
        let empty = "[host]\nversion = \"\"\n";

        // ACT
        let versions = [
            desired_version(missing).expect("a missing version is tolerated"),
            desired_version(empty).expect("an empty version is tolerated"),
        ];

        // ASSERT
        assert_eq!(versions, [None, None]);
    }

    #[test]
    fn desired_version_rejects_a_mistyped_version() {
        // ARRANGE
        let raw = "[host]\nversion = 3\n";

        // ACT
        let error = desired_version(raw).expect_err("a mistyped version must fail");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("host.version"), "{message}");
        assert!(message.contains("integer"), "{message}");
    }

    #[test]
    fn desired_version_rejects_an_invalid_document() {
        // ARRANGE
        let raw = "this is not a toml document";

        // ACT
        let error = desired_version(raw).expect_err("invalid TOML must fail");

        // ASSERT
        assert!(error.to_string().contains("parse"), "{error}");
    }

    #[test]
    fn request_payload_falls_back_to_the_explicit_version() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let raw = "api_version = \"muak.dev/config/v9-experimental\"\n\n\
                   [host]\nname = \"muak\"\nregistry = \"ghcr.io/muak-os\"\n";
        std::fs::write(&path, raw).unwrap();

        // ACT
        let payload = request_payload(Some("v1.2.0".to_owned()), Some(&path), &installed("v1.0.0"))
            .expect("the explicit version must be used");

        // ASSERT
        assert_eq!(payload.version, "v1.2.0");
        assert_eq!(payload.config, raw.as_bytes());
    }

    #[test]
    fn request_payload_without_any_version_is_rejected() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, document("")).unwrap();

        // ACT
        let error = request_payload(None, Some(&path), &installed("v1.0.0"))
            .expect_err("neither source provides a version");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("host.version"), "{message}");
        assert!(message.contains("--version"), "{message}");
    }

    #[test]
    fn request_payload_rejects_version_outside_the_document() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, document("v1.2.0")).unwrap();

        // ACT
        let error = request_payload(Some("v1.3.0".to_owned()), Some(&path), &installed("v1.0.0"))
            .expect_err("a disagreeing version must be rejected");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("v1.3.0"), "{message}");
        assert!(message.contains("v1.2.0"), "{message}");
    }

    #[test]
    fn request_payload_uses_the_document_version() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let raw = document("v1.2.0");
        std::fs::write(&path, &raw).unwrap();

        // ACT
        let payload = request_payload(None, Some(&path), &installed("v1.0.0")).unwrap();

        // ASSERT
        assert_eq!(payload.version, "v1.2.0");
        assert_eq!(payload.config, raw.into_bytes());
        assert!(
            payload.warning.is_none(),
            "a known document needs no warning"
        );
    }

    #[test]
    fn request_payload_warns_on_an_unknown_document() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let raw = document("v2.0.0").replace(
            config::system::API_VERSION,
            "muak.dev/config/v9-experimental",
        );
        std::fs::write(&path, &raw).unwrap();

        // ACT
        let payload = request_payload(None, Some(&path), &installed("v1.0.0")).unwrap();

        // ASSERT
        assert_eq!(payload.version, "v2.0.0");
        let warning = payload.warning.expect("an unknown document must warn");
        assert!(warning.contains("validated on the node"), "{warning}");
    }
}
