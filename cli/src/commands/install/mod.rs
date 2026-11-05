mod plan;
mod reboot;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use config::user::{ClientConfig, ServerContext};
use pki::csr;
use tokio_stream::StreamExt as _;
use tonic::transport::Channel;
use tui::console::Console;

use super::Failure;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{InstallPlan, InstallRequest, PlanInstallRequest};

// Data received upon successful completion of the install process.
struct CompletedInstall {
    ca_pem: String,
    client_cert_pem: String,
    server_name: String,
}

/// Options selecting what the install command does.
pub struct Options {
    pub force: bool,
    pub config: PathBuf,
    pub dry_run: bool,
    pub allow_system_disk: bool,
}

/// Plans the install and runs it.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    options: Options,
    server_endpoint: &str,
    console: &Console,
) -> Result<()> {
    let config_bytes = read_config(&options.config)?;
    let plan = request_plan(client, &config_bytes, &options).await?;

    plan::print(&plan, console)?;
    if plan::refused(&plan) {
        return Err(Failure::preflight("Install refused by preflight checks").into());
    }

    if options.dry_run {
        console.info("Dry run: nothing was written");
        return Ok(());
    }

    install(
        client,
        &plan,
        config_bytes,
        &options,
        server_endpoint,
        console,
    )
    .await
}

fn read_config(path: &Path) -> Result<Vec<u8>> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        Failure::usage(format!(
            "Failed to read config file '{}': {e}",
            path.display()
        ))
    })?;

    let parsed = config::system::parse_from_str(&raw)
        .map_err(|e| Failure::usage(format!("Invalid config file '{}': {e}", path.display())))?;

    parsed.validate_for_install().map_err(|e| {
        Failure::usage(format!(
            "Invalid config for install in '{}': {e}",
            path.display()
        ))
    })?;

    Ok(raw.into_bytes())
}

async fn request_plan(
    client: &mut ProvisionServiceClient<Channel>,
    config_bytes: &[u8],
    options: &Options,
) -> Result<InstallPlan> {
    let request = tonic::Request::new(PlanInstallRequest {
        config_bytes: config_bytes.to_vec(),
        force: options.force,
        allow_system_disk: options.allow_system_disk,
    });

    client
        .plan_install(request)
        .await
        .map_err(plan_error)
        .map(tonic::Response::into_inner)
}

fn plan_error(status: tonic::Status) -> anyhow::Error {
    match status.code() {
        tonic::Code::FailedPrecondition => {
            Failure::preflight(format!("Install preflight failed: {}", status.message())).into()
        }
        tonic::Code::InvalidArgument => Failure::usage(status.message().to_owned()).into(),
        tonic::Code::Ok
        | tonic::Code::Cancelled
        | tonic::Code::Unknown
        | tonic::Code::DeadlineExceeded
        | tonic::Code::NotFound
        | tonic::Code::AlreadyExists
        | tonic::Code::PermissionDenied
        | tonic::Code::ResourceExhausted
        | tonic::Code::Unauthenticated
        | tonic::Code::Aborted
        | tonic::Code::OutOfRange
        | tonic::Code::Unimplemented
        | tonic::Code::Internal
        | tonic::Code::Unavailable
        | tonic::Code::DataLoss => anyhow::Error::new(status),
    }
}

async fn install(
    client: &mut ProvisionServiceClient<Channel>,
    plan: &InstallPlan,
    config_bytes: Vec<u8>,
    options: &Options,
    server_endpoint: &str,
    console: &Console,
) -> Result<()> {
    let mut client_config = ClientConfig::load()?;
    let (key_pem, csr_pem) = csr::generate("muak-admin")?;

    let request = tonic::Request::new(InstallRequest {
        force: options.force,
        config_bytes,
        csr: csr_pem,
        allow_system_disk: options.allow_system_disk,
    });

    let target_disk = plan::system_path(plan);
    let msg = format!("Installing Muak to {target_disk}...");
    console.info(&msg);

    let response = client
        .install(request)
        .await
        .context("Failed to send install request")?;

    let mut stream = response.into_inner();
    let mut result_data: Option<CompletedInstall> = None;

    let steps = console.steps()?;

    while let Some(progress) = stream.next().await {
        let progress = progress.context("Error receiving install progress")?;

        if !progress.error.is_empty() {
            let msg = format!("Installation failed: {}", progress.error);
            steps.fail(&msg);
            let _finish_result = steps.finish();
            let failure = format!("{msg}. The installation may be incomplete.");
            return Err(Failure::incomplete(failure).into());
        }

        if progress.ca_pem.is_empty() {
            steps.start(&progress.message);
        } else {
            result_data = Some(CompletedInstall {
                ca_pem: progress.ca_pem,
                client_cert_pem: progress.client_cert_pem,
                server_name: progress.server_name,
            });
        }
    }

    let result = result_data.ok_or_else(|| {
        Failure::incomplete("Install stream ended without a completion or failure message")
    })?;

    let msg = format!("Successfully installed Muak to {target_disk}");
    steps.complete(&msg);

    let context_name = if result.server_name.is_empty() {
        "default"
    } else {
        &result.server_name
    };

    let ctx = ServerContext::from_pem(
        server_endpoint,
        &result.ca_pem,
        &result.client_cert_pem,
        key_pem.as_bytes(),
    );

    let actual_name = client_config.add_context(context_name, ctx.clone());
    client_config.set_current(&actual_name)?;
    client_config.save()?;

    let reboot_result = reboot::wait(&ctx, &steps).await;
    steps.finish()?;
    reboot_result?;

    let msg = format!("Context '{actual_name}' added and set as current.");
    console.success(&msg);

    Ok(())
}
