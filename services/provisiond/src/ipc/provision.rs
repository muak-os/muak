//! gRPC service implementation for provisioning operations.

use tokio::task;
use tonic::{Request, Response, Status};

use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::{
    ConfigHistoryEntry, DiskInfo, FactoryResetRequest, FactoryResetResponse,
    GetConfigHistoryRequest, GetConfigHistoryResponse, GetConfigRequest, GetConfigResponse,
    GetConfigSnapshotRequest, GetConfigSnapshotResponse, GetRollbackHistoryRequest,
    GetRollbackHistoryResponse, GetUpdateStatusRequest, GetUpdateStatusResponse, InstallProgress,
    InstallRequest, ListDisksRequest, ListDisksResponse, PartitionInfo, PrepareUpdateProgress,
    PrepareUpdateRequest, RollbackHistoryEntry, UpdateRequest, UpdateResponse,
};
use crate::disk;
use crate::history;
use crate::install;
use crate::reboot;
use crate::reset;
use crate::streaming;
use crate::update;
use crate::update::rollback;

/// Creates the `ProvisionService` gRPC server.
pub fn service() -> ProvisionServiceServer<ServiceImpl> {
    ProvisionServiceServer::new(ServiceImpl)
}

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

        let config_raw = String::from_utf8(req.config_bytes)
            .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8 in config: {e}")))?;

        let config: config::SystemConfig = config::parse_from_str(&config_raw)
            .map_err(|e| Status::invalid_argument(format!("Invalid config: {e}")))?;

        config
            .validate_for_install()
            .map_err(|e| Status::invalid_argument(format!("Invalid config for install: {e}")))?;

        let server_name = config.host.name.clone();
        let force = req.force;
        let csr = req.csr;

        let stream = streaming::run(
            move |progress_tx| async move {
                install::run(
                    &config.disk.system.clone(),
                    config.disk.data_disk(),
                    force,
                    &config,
                    &csr,
                    progress_tx,
                )
                .await
            },
            move |result, out_tx| {
                let msg = match result {
                    Ok(result) => {
                        reboot::schedule(1);
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

    async fn prepare_update(
        &self,
        request: Request<PrepareUpdateRequest>,
    ) -> Result<Response<Self::PrepareUpdateStream>, Status> {
        let author = extract_author(&request);
        let req = request.into_inner();
        let installed = config::config();

        let (image, extensions, new_config) = if req.config.is_empty() {
            let image = if req.image.is_empty() {
                installed.host.image.clone()
            } else {
                config::check_no_downgrade(&req.image, &installed.host.image)
                    .map_err(|e| Status::invalid_argument(format!("{e}")))?;
                req.image.clone()
            };
            let extensions = installed.host.extensions.clone();
            (image, extensions, None)
        } else {
            let raw = String::from_utf8(req.config)
                .map_err(|e| Status::invalid_argument(format!("Config is not valid UTF-8: {e}")))?;

            let cfg: config::SystemConfig = config::parse_from_str(&raw)
                .map_err(|e| Status::invalid_argument(format!("Invalid config: {e}")))?;

            cfg.validate_for_update(installed)
                .map_err(|e| Status::invalid_argument(format!("Config rejected: {e}")))?;

            config::check_no_downgrade(&cfg.host.image, &installed.host.image)
                .map_err(|e| Status::invalid_argument(format!("{e}")))?;

            let image = cfg.host.image.clone();
            let extensions = cfg.host.extensions.clone();
            (image, extensions, Some(cfg))
        };

        let stream = streaming::run(
            move |progress_tx| async move {
                update::prepare(&image, &extensions, new_config, &author, progress_tx).await
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
                error: format!("{e}"),
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
            update::UpdateStatus::Unknown => (0, String::new()),
            update::UpdateStatus::Pending => {
                update::signal_cli_contact();
                (1, String::new())
            }
            update::UpdateStatus::Committed => (2, String::new()),
            update::UpdateStatus::RolledBack(reason) => (3, reason),
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

        let proto_disks = disks
            .into_iter()
            .map(|disk| DiskInfo {
                name: disk.name,
                path: disk.path,
                size_bytes: disk.size_bytes,
                model: disk.model,
                removable: disk.removable,
                read_only: disk.read_only,
                partitions: disk
                    .partitions
                    .into_iter()
                    .map(|partition| PartitionInfo {
                        number: partition.number,
                        start_sector: partition.start_sector,
                        size_bytes: partition.size_bytes,
                        name: partition.name,
                        path: partition.path,
                        fstype: partition.fstype,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(ListDisksResponse {
            disks: proto_disks,
            error: String::new(),
        }))
    }

    async fn get_config(
        &self,
        _request: Request<GetConfigRequest>,
    ) -> Result<Response<GetConfigResponse>, Status> {
        if let Some(config) = config::try_config() {
            match config::serialize(config) {
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

    async fn get_config_history(
        &self,
        request: Request<GetConfigHistoryRequest>,
    ) -> Result<Response<GetConfigHistoryResponse>, Status> {
        let limit = usize::try_from(request.into_inner().limit).unwrap_or(0);
        let effective_limit = if limit == 0 { 100 } else { limit };

        match task::spawn_blocking(move || history::list(effective_limit)).await {
            Ok(Ok(entries)) => {
                let proto_entries = entries
                    .into_iter()
                    .map(|e| ConfigHistoryEntry {
                        timestamp: e.timestamp,
                        update_id: e.update_id,
                        author: e.author,
                        change_kind: e.change_kind.to_string(),
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

        match task::spawn_blocking(move || history::config(&update_id)).await {
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

        match task::spawn_blocking(move || rollback::list(effective_limit)).await {
            Ok(entries) => {
                let proto_entries: Vec<RollbackHistoryEntry> = entries
                    .into_iter()
                    .map(|e| RollbackHistoryEntry {
                        update_id: e.update_id,
                        failed_image: e.failed_image,
                        reason: e.reason,
                        rolled_back_at: e.rolled_back_at,
                    })
                    .collect();
                Ok(Response::new(GetRollbackHistoryResponse {
                    entries: proto_entries,
                    error: String::new(),
                }))
            }
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
                reboot::schedule(1);
                Ok(Response::new(FactoryResetResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("{e}"),
            })),
            Err(e) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("Task panicked: {e}"),
            })),
        }
    }
}

/// Extracts the mTLS client certificate fingerprint from the request metadata.
fn extract_author<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-client-fingerprint")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned()
}
