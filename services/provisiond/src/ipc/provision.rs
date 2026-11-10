//! gRPC service implementation for provisioning operations.

use blockdev::types::Disk;
use tokio::task;
use tonic::{Request, Response, Status};

use super::checks;
use super::inventory::to_disk_info;
use super::plan::to_install_plan;
use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::{
    ConfigHistoryEntry, FactoryResetRequest, FactoryResetResponse, GetConfigHistoryRequest,
    GetConfigHistoryResponse, GetConfigRequest, GetConfigResponse, GetConfigSnapshotRequest,
    GetConfigSnapshotResponse, GetDefaultConfigRequest, GetDefaultConfigResponse,
    GetRollbackHistoryRequest, GetRollbackHistoryResponse, GetUpdateStatusRequest,
    GetUpdateStatusResponse, InstallPlan, InstallProgress, InstallRequest, ListDisksRequest,
    ListDisksResponse, PlanInstallRequest, PowerResponse, PrepareUpdateProgress,
    PrepareUpdateRequest, RebootRequest, RollbackHistoryEntry, ShutdownRequest, UpdateRequest,
    UpdateResponse, WipeDiskRequest, WipeDiskResponse, WipeMethod as ProtoWipeMethod,
};
use crate::check::{self, Check};
use crate::{disk, install, journal, power, reset, streaming, template, update};

/// Implementation of the `ProvisionService` gRPC interface.
pub struct ServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ServiceImpl {
    type InstallStream = streaming::ProgressStream<InstallProgress>;
    type PrepareUpdateStream = streaming::ProgressStream<PrepareUpdateProgress>;

    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<Self::InstallStream>, Status> {
        let req = request.into_inner();

        if req.csr.is_empty() {
            return Err(Status::invalid_argument("CSR is required"));
        }

        let config = parse_install_config(req.config_bytes)?;

        let server_name = config.host.name.clone();
        let force = req.force;
        let allow_system_disk = req.allow_system_disk;
        let csr = req.csr;

        let stream = streaming::run(
            move |progress_tx| async move {
                install::run(force, allow_system_disk, &config, &csr, progress_tx).await
            },
            move |result, out_tx| {
                let msg = match result {
                    Ok(result) => {
                        power::reboot(1);
                        Ok(InstallProgress {
                            ca_pem: result.ca_pem,
                            client_cert_pem: result.admin_cert_pem,
                            server_name,
                            ..Default::default()
                        })
                    }
                    Err(e) => Ok(InstallProgress {
                        error: format!("{e:#}"),
                        ..Default::default()
                    }),
                };
                let _sent = out_tx.try_send(msg);
            },
        );

        Ok(Response::new(stream))
    }

    async fn plan_install(
        &self,
        request: Request<PlanInstallRequest>,
    ) -> Result<Response<InstallPlan>, Status> {
        let req = request.into_inner();
        let config = parse_install_config(req.config_bytes)?;

        let plan = install::plan::resolve(&config, req.force, req.allow_system_disk)
            .await
            .map_err(|e| Status::failed_precondition(format!("{e:#}")))?;

        Ok(Response::new(to_install_plan(plan)))
    }

    async fn prepare_update(
        &self,
        request: Request<PrepareUpdateRequest>,
    ) -> Result<Response<Self::PrepareUpdateStream>, Status> {
        let author = extract_author(&request);
        let req = request.into_inner();
        let force = req.force;
        let strategy = update::strategy(req.offline);
        let installed = config::system::config().map_err(|e| Status::internal(e.to_string()))?;

        let (inputs, new_config) = if req.config.is_empty() {
            let version = update::resolve_target(&installed.host.version, &req.version)
                .map_err(|e| Status::invalid_argument(format!("{e:#}")))?;
            let inputs = config::document::HostSpec {
                registry: installed.host.registry.clone(),
                version,
                extensions: installed.host.extensions.clone(),
                secureboot: installed.host.secureboot,
            };

            (inputs, None)
        } else {
            let raw = String::from_utf8(req.config)
                .map_err(|e| Status::invalid_argument(format!("Config is not valid UTF-8: {e}")))?;
            let requested = (!req.version.is_empty()).then_some(req.version.as_str());

            let inputs = config::document::host_spec(&raw, requested)
                .map_err(|e| Status::invalid_argument(format!("Invalid config: {e:#}")))?;

            check_against_installed(&raw, installed)?;

            config::version::check_no_downgrade(&inputs.version, &installed.host.version)
                .map_err(|e| Status::invalid_argument(format!("{e:#}")))?;

            (inputs, Some(raw.into_bytes()))
        };

        let current_config = std::fs::read(config::system::CONFIG_PATH)
            .map_err(|e| Status::internal(format!("Failed to read the current config: {e}")))?;

        if update::already_current(
            &installed.host.version,
            &inputs.version,
            &current_config,
            new_config.as_deref(),
            force,
        ) {
            return Ok(Response::new(streaming::single(PrepareUpdateProgress {
                message: format!(
                    "System already at version {}, nothing to do",
                    inputs.version
                ),
                already_current: true,
                ..Default::default()
            })));
        }

        let stream = streaming::run(
            move |progress_tx| async move {
                update::build::prepare(&inputs, new_config, &author, strategy, progress_tx).await
            },
            |result, out_tx| {
                let msg = match result {
                    Ok(update_id) => Ok(PrepareUpdateProgress {
                        update_id,
                        ..Default::default()
                    }),
                    Err(e) => Ok(PrepareUpdateProgress {
                        error: format!("{e:#}"),
                        ..Default::default()
                    }),
                };
                let _sent = out_tx.try_send(msg);
            },
        );

        Ok(Response::new(stream))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let update_id = request.into_inner().update_id;

        match task::spawn_blocking(move || update::kexec::run(&update_id)).await {
            Ok(Ok(())) => Ok(Response::new(UpdateResponse {
                success: false,
                error: "kexec returned unexpectedly without rebooting".to_owned(),
            })),
            Ok(Err(e)) => Ok(Response::new(UpdateResponse {
                success: false,
                error: format!("{e:#}"),
            })),
            Err(e) => Ok(Response::new(UpdateResponse {
                success: false,
                error: format!("Task panicked: {e}"),
            })),
        }
    }

    async fn get_update_status(
        &self,
        request: Request<GetUpdateStatusRequest>,
    ) -> Result<Response<GetUpdateStatusResponse>, Status> {
        let update_id = request.into_inner().update_id;

        let status = task::spawn_blocking(move || update::status(&update_id))
            .await
            .map_err(|e| Status::internal(format!("Task failed: {e}")))?;

        let (proto_status, error) = match status {
            update::UpdateStatus::Unknown => (0_i32, String::new()),
            update::UpdateStatus::Pending => {
                update::signal_cli_contact();
                (1_i32, String::new())
            }
            update::UpdateStatus::Committed => (2_i32, String::new()),
            update::UpdateStatus::RolledBack(reason) => (3_i32, reason),
        };

        Ok(Response::new(GetUpdateStatusResponse {
            status: proto_status,
            error,
        }))
    }

    async fn list_disks(
        &self,
        _request: Request<ListDisksRequest>,
    ) -> Result<Response<ListDisksResponse>, Status> {
        let disks = disk::list_disks()
            .await
            .map_err(|e| Status::internal(format!("Failed to list disks: {e}")))?;

        let proto_disks = disks.into_iter().map(to_disk_info).collect();

        Ok(Response::new(ListDisksResponse {
            disks: proto_disks,
            error: String::new(),
        }))
    }

    async fn wipe_disk(
        &self,
        request: Request<WipeDiskRequest>,
    ) -> Result<Response<WipeDiskResponse>, Status> {
        let req = request.into_inner();
        let method = wipe_method(req.method)?;

        let disks = disk::list_disks()
            .await
            .map_err(|e| Status::internal(format!("Failed to list disks: {e}")))?;
        let verdict = disk::guard::validate(&disks, &req.disk, req.allow_system_disk)
            .map_err(|e| Status::failed_precondition(format!("{e:#}")))?;
        let refusal = check::enforce(&verdict.checks)
            .err()
            .map(|e| format!("{e:#}"));
        let mut response = wipe_response(refusal, verdict.disk, verdict.checks);

        if !response.success {
            return Ok(Response::new(response));
        }

        let options = disk::wipe::Options {
            method,
            drop_partition: req.drop_partition,
            dry_run: req.dry_run,
        };

        match task::spawn_blocking(move || disk::wipe::run(&verdict.target, options)).await {
            Ok(Ok(message)) => response.message = message,
            Ok(Err(e)) => {
                response.success = false;
                response.error = format!("{e:#}");
            }
            Err(e) => {
                response.success = false;
                response.error = format!("Task panicked: {e}");
            }
        }

        Ok(Response::new(response))
    }

    async fn get_config(
        &self,
        _request: Request<GetConfigRequest>,
    ) -> Result<Response<GetConfigResponse>, Status> {
        if let Some(config) = config::system::try_config() {
            match config::system::serialize(config) {
                Ok(config_bytes) => Ok(Response::new(GetConfigResponse {
                    config: config_bytes.into_bytes(),
                    error: String::new(),
                })),
                Err(e) => Ok(Response::new(GetConfigResponse {
                    config: Vec::new(),
                    error: format!("Failed to serialize config: {e}"),
                })),
            }
        } else {
            Ok(Response::new(GetConfigResponse {
                config: Vec::new(),
                error: "Config not initialized: system has not been installed yet".to_owned(),
            }))
        }
    }

    async fn get_default_config(
        &self,
        _request: Request<GetDefaultConfigRequest>,
    ) -> Result<Response<GetDefaultConfigResponse>, Status> {
        match template::generate().await {
            Ok(config) => Ok(Response::new(GetDefaultConfigResponse {
                config: config.into_bytes(),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(GetDefaultConfigResponse {
                config: Vec::new(),
                error: format!("{e:#}"),
            })),
        }
    }

    async fn get_config_history(
        &self,
        request: Request<GetConfigHistoryRequest>,
    ) -> Result<Response<GetConfigHistoryResponse>, Status> {
        let limit = usize::try_from(request.into_inner().limit).unwrap_or(0);
        let effective_limit = if limit == 0 { 100 } else { limit };

        match task::spawn_blocking(move || journal::list(effective_limit)).await {
            Ok(Ok(entries)) => {
                let proto_entries = entries
                    .into_iter()
                    .map(|e| ConfigHistoryEntry {
                        timestamp: e.timestamp(),
                        update_id: e.update_id().to_owned(),
                        author: e.author().to_owned(),
                        change_kind: e.kind().to_string(),
                        reason: e.reason().unwrap_or_default().to_owned(),
                    })
                    .collect();
                Ok(Response::new(GetConfigHistoryResponse {
                    entries: proto_entries,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => Ok(Response::new(GetConfigHistoryResponse {
                entries: Vec::new(),
                error: format!("{e:#}"),
            })),
            Err(e) => Ok(Response::new(GetConfigHistoryResponse {
                entries: Vec::new(),
                error: format!("Task panicked: {e}"),
            })),
        }
    }

    async fn get_config_snapshot(
        &self,
        request: Request<GetConfigSnapshotRequest>,
    ) -> Result<Response<GetConfigSnapshotResponse>, Status> {
        let update_id = request.into_inner().update_id;

        match task::spawn_blocking(move || journal::snapshot(&update_id)).await {
            Ok(Ok(config)) => Ok(Response::new(GetConfigSnapshotResponse {
                config: config.into_bytes(),
                error: String::new(),
            })),
            Ok(Err(e)) => Ok(Response::new(GetConfigSnapshotResponse {
                config: Vec::new(),
                error: format!("{e:#}"),
            })),
            Err(e) => Ok(Response::new(GetConfigSnapshotResponse {
                config: Vec::new(),
                error: format!("Task panicked: {e}"),
            })),
        }
    }

    async fn get_rollback_history(
        &self,
        request: Request<GetRollbackHistoryRequest>,
    ) -> Result<Response<GetRollbackHistoryResponse>, Status> {
        let limit = usize::try_from(request.into_inner().limit).unwrap_or(0);
        let effective_limit = if limit == 0 { 100 } else { limit };

        match task::spawn_blocking(move || journal::list(effective_limit)).await {
            Ok(Ok(entries)) => {
                let proto_entries: Vec<RollbackHistoryEntry> = entries
                    .into_iter()
                    .filter(|entry| entry.kind() == &journal::ChangeKind::Rollback)
                    .map(|e| RollbackHistoryEntry {
                        update_id: e.update_id().to_owned(),
                        failed_version: e.failed_version().unwrap_or_default().to_owned(),
                        reason: e.reason().unwrap_or_default().to_owned(),
                        rolled_back_at: e.timestamp(),
                    })
                    .collect();
                Ok(Response::new(GetRollbackHistoryResponse {
                    entries: proto_entries,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => Ok(Response::new(GetRollbackHistoryResponse {
                entries: Vec::new(),
                error: format!("{e:#}"),
            })),
            Err(e) => Ok(Response::new(GetRollbackHistoryResponse {
                entries: Vec::new(),
                error: format!("Task panicked: {e}"),
            })),
        }
    }

    async fn factory_reset(
        &self,
        _request: Request<FactoryResetRequest>,
    ) -> Result<Response<FactoryResetResponse>, Status> {
        match task::spawn_blocking(reset::factory_reset).await {
            Ok(Ok(())) => {
                power::reboot(1);
                Ok(Response::new(FactoryResetResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("{e:#}"),
            })),
            Err(e) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("Task panicked: {e}"),
            })),
        }
    }

    async fn reboot(
        &self,
        _request: Request<RebootRequest>,
    ) -> Result<Response<PowerResponse>, Status> {
        power::reboot(1);

        Ok(Response::new(PowerResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn shutdown(
        &self,
        _request: Request<ShutdownRequest>,
    ) -> Result<Response<PowerResponse>, Status> {
        power::poweroff(1);

        Ok(Response::new(PowerResponse {
            success: true,
            error: String::new(),
        }))
    }
}

/// Extracts the mTLS client certificate fingerprint from the request metadata.
/// Creates the `ProvisionService` gRPC server.
pub fn service() -> ProvisionServiceServer<ServiceImpl> {
    ProvisionServiceServer::new(ServiceImpl)
}

fn wipe_response(refusal: Option<String>, disk: Disk, checks: Vec<Check>) -> WipeDiskResponse {
    WipeDiskResponse {
        success: refusal.is_none(),
        error: refusal.unwrap_or_default(),
        message: String::new(),
        disk: Some(to_disk_info(disk)),
        checks: checks.into_iter().map(checks::to_proto).collect(),
    }
}

fn check_against_installed(raw: &str, installed: &config::system::Config) -> Result<(), Status> {
    let Ok(candidate) = config::system::parse_from_str(raw) else {
        return Ok(());
    };

    candidate
        .validate_for_update(installed)
        .map_err(|e| Status::invalid_argument(format!("Invalid config for update: {e}")))
}

fn parse_install_config(config_bytes: Vec<u8>) -> Result<config::system::Config, Status> {
    let raw = String::from_utf8(config_bytes)
        .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8 in config: {e}")))?;

    let config: config::system::Config = config::system::parse_from_str(&raw)
        .map_err(|e| Status::invalid_argument(format!("Invalid config: {e}")))?;

    config
        .validate_for_install()
        .map_err(|e| Status::invalid_argument(format!("Invalid config for install: {e}")))?;

    Ok(config)
}

fn wipe_method(value: i32) -> Result<disk::wipe::Method, Status> {
    match ProtoWipeMethod::try_from(value) {
        Ok(ProtoWipeMethod::Fast) => Ok(disk::wipe::Method::Fast),
        Ok(ProtoWipeMethod::Zeroes) => Ok(disk::wipe::Method::Zeroes),
        Err(_) => Err(Status::invalid_argument("unknown wipe method")),
    }
}

fn extract_author<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-client-fingerprint")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk() -> Disk {
        Disk {
            name: "nvme0n1".to_owned(),
            path: "/dev/nvme0n1".to_owned(),
            size_bytes: 1024,
            ..Disk::default()
        }
    }

    #[test]
    fn immutable_disk_changes_are_rejected_before_staging() {
        // ARRANGE
        let installed = config::system::Config::default();
        let raw = format!(
            "api_version = \"{}\"\n\n[host]\nname = \"muak\"\nversion = \"v2.0.0\"\nport = 50051\n\
             ntp = \"pool.ntp.org\"\n\n[[disks]]\nrole = \"system\"\nwwid = \"eui.0025385b71a1c7d4\"\n",
            config::system::API_VERSION
        );

        // ACT
        let status = check_against_installed(&raw, &installed)
            .expect_err("a new system disk must be rejected");

        // ASSERT
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn undecodable_documents_skip_the_installed_check() {
        // ARRANGE
        let installed = config::system::Config::default();
        let raw = "api_version = \"muak.dev/config/v9-future\"\n\n[host]\nname = \"muak\"\n";

        // ACT
        let result = check_against_installed(raw, &installed);

        // ASSERT
        result.expect("a future schema must be stored as is");
    }

    #[test]
    fn response_carries_the_resolved_disk_and_checks() {
        // ARRANGE
        let checks = vec![
            Check::pass("read_only", "writable".to_owned()),
            Check::fail("muak_partitions", "STATE present".to_owned()),
        ];

        // ACT
        let response = wipe_response(None, disk(), checks);

        // ASSERT
        let disk = response.disk.expect("resolved disk");
        assert_eq!(disk.path, "/dev/nvme0n1");
        assert_eq!(response.checks.len(), 2);
        assert_eq!(response.checks[0].name, "read_only");
        assert!(response.checks[0].passed);
        assert!(!response.checks[1].passed);
    }

    #[test]
    fn refusal_fails_the_response_but_keeps_the_preview() {
        // ARRANGE
        let checks = vec![Check::fail("read_only", "read-only".to_owned())];

        // ACT
        let response = wipe_response(Some("read-only".to_owned()), disk(), checks);

        // ASSERT
        assert!(!response.success);
        assert_eq!(response.error, "read-only");
        assert!(response.disk.is_some());
        assert_eq!(response.checks.len(), 1);
    }
}
