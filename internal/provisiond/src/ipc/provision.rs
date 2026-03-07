//! gRPC service implementation for provisioning operations.

use tonic::{Request, Response, Status};

use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::*;
use crate::disk;
use crate::history;
use crate::install;
use crate::reboot;
use crate::reset;
use crate::streaming;
use crate::update;

/// Creates the ProvisionService gRPC server.
pub fn service() -> ProvisionServiceServer<ProvisionServiceImpl> {
    ProvisionServiceServer::new(ProvisionServiceImpl)
}

/// Implementation of the ProvisionService gRPC interface.
pub struct ProvisionServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ProvisionServiceImpl {
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
            .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8 in config: {}", e)))?;

        let config: sysconfig::HostConfig = sysconfig::parse_from_str(&config_raw)
            .map_err(|e| Status::invalid_argument(format!("Invalid config: {}", e)))?;

        config
            .validate_for_install()
            .map_err(|e| Status::invalid_argument(format!("Invalid config for install: {}", e)))?;

        let server_name = config.system.name.clone();
        let force = req.force;
        let csr = req.csr;

        let stream = streaming::run(
            move |progress_tx| async move {
                let disk = config.system.disk.clone();
                install::run(&disk, force, &config, &csr, progress_tx).await
            },
            move |result, out_tx| {
                let msg = match result {
                    Ok(r) => {
                        reboot::schedule(1);
                        Ok(InstallProgress {
                            ca_pem: r.ca_pem,
                            client_cert_pem: r.admin_cert_pem,
                            server_name,
                            ..Default::default()
                        })
                    }
                    Err(e) => Ok(InstallProgress {
                        error: format!("{:#}", e),
                        ..Default::default()
                    }),
                };
                let _ = out_tx.try_send(msg);
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
        let installed = sysconfig::config();

        let (image, extensions, new_config) = if !req.config.is_empty() {
            let raw = String::from_utf8(req.config).map_err(|e| {
                Status::invalid_argument(format!("Config is not valid UTF-8: {}", e))
            })?;

            let cfg: sysconfig::HostConfig = sysconfig::parse_from_str(&raw)
                .map_err(|e| Status::invalid_argument(format!("Invalid config: {}", e)))?;

            cfg.validate_for_update(installed)
                .map_err(|e| Status::invalid_argument(format!("Config rejected: {}", e)))?;

            sysconfig::check_no_downgrade(&cfg.system.image, &installed.system.image)
                .map_err(|e| Status::invalid_argument(format!("{}", e)))?;

            let image = cfg.system.image.clone();
            let extensions = cfg.system.extensions.clone();
            (image, extensions, Some(cfg))
        } else {
            let image = if req.image.is_empty() {
                installed.system.image.clone()
            } else {
                sysconfig::check_no_downgrade(&req.image, &installed.system.image)
                    .map_err(|e| Status::invalid_argument(format!("{}", e)))?;
                req.image.clone()
            };
            let extensions = installed.system.extensions.clone();
            (image, extensions, None)
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
                        error: format!("{:#}", e),
                        ..Default::default()
                    }),
                };
                let _ = out_tx.try_send(msg);
            },
        );

        Ok(Response::new(stream))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let update_id = request.into_inner().update_id;

        match tokio::task::spawn_blocking(move || update::kexec::run(&update_id)).await {
            Ok(Ok(_)) => unreachable!("If we're here, something went really wrong in kexec"),
            Ok(Err(e)) => Ok(Response::new(UpdateResponse {
                success: false,
                error: format!("{}", e),
            })),
            Err(e) => Ok(Response::new(UpdateResponse {
                success: false,
                error: format!("Task panicked: {}", e),
            })),
        }
    }

    async fn get_update_status(
        &self,
        request: Request<GetUpdateStatusRequest>,
    ) -> Result<Response<GetUpdateStatusResponse>, Status> {
        let update_id = request.into_inner().update_id;

        let status = tokio::task::spawn_blocking(move || update::status(&update_id))
            .await
            .map_err(|e| Status::internal(format!("Task failed: {}", e)))?;

        let (proto_status, error) = match status {
            update::UpdateStatus::Unknown => (0, String::new()),
            update::UpdateStatus::Pending => (1, String::new()),
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
            .map_err(|e| Status::internal(format!("Failed to list disks: {}", e)))?;

        let proto_disks = disks
            .into_iter()
            .map(|d| DiskInfo {
                name: d.name,
                path: d.path,
                size_bytes: d.size_bytes,
                model: d.model,
                removable: d.removable,
                read_only: d.read_only,
                partitions: d
                    .partitions
                    .into_iter()
                    .map(|p| PartitionInfo {
                        number: p.number,
                        start_sector: p.start_sector,
                        size_bytes: p.size_bytes,
                        name: p.name,
                        path: p.path,
                        fstype: p.fstype,
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
        if let Some(config) = sysconfig::try_config() {
            match sysconfig::serialize(config) {
                Ok(config_bytes) => Ok(Response::new(GetConfigResponse {
                    config: config_bytes.into_bytes(),
                    error: String::new(),
                })),
                Err(e) => Ok(Response::new(GetConfigResponse {
                    config: Vec::new(),
                    error: format!("Failed to serialize config: {}", e),
                })),
            }
        } else {
            Ok(Response::new(GetConfigResponse {
                config: Vec::new(),
                error: "Config not initialized: system has not been installed yet".to_string(),
            }))
        }
    }

    async fn get_config_history(
        &self,
        request: Request<GetConfigHistoryRequest>,
    ) -> Result<Response<GetConfigHistoryResponse>, Status> {
        let limit = request.into_inner().limit as usize;
        let effective_limit = if limit == 0 { 100 } else { limit };

        match tokio::task::spawn_blocking(move || history::list(effective_limit)).await {
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
                error: format!("{:#}", e),
            })),
            Err(e) => Ok(Response::new(GetConfigHistoryResponse {
                entries: Vec::new(),
                error: format!("Task panicked: {}", e),
            })),
        }
    }

    async fn get_config_snapshot(
        &self,
        request: Request<GetConfigSnapshotRequest>,
    ) -> Result<Response<GetConfigSnapshotResponse>, Status> {
        let update_id = request.into_inner().update_id;

        match tokio::task::spawn_blocking(move || history::config(&update_id)).await {
            Ok(Ok(config)) => Ok(Response::new(GetConfigSnapshotResponse {
                config: config.into_bytes(),
                error: String::new(),
            })),
            Ok(Err(e)) => Ok(Response::new(GetConfigSnapshotResponse {
                config: Vec::new(),
                error: format!("{:#}", e),
            })),
            Err(e) => Ok(Response::new(GetConfigSnapshotResponse {
                config: Vec::new(),
                error: format!("Task panicked: {}", e),
            })),
        }
    }

    async fn factory_reset(
        &self,
        _request: Request<FactoryResetRequest>,
    ) -> Result<Response<FactoryResetResponse>, Status> {
        match tokio::task::spawn_blocking(reset::factory_reset).await {
            Ok(Ok(())) => {
                reboot::schedule(1);
                Ok(Response::new(FactoryResetResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("{}", e),
            })),
            Err(e) => Ok(Response::new(FactoryResetResponse {
                success: false,
                error: format!("Task panicked: {}", e),
            })),
        }
    }
}

/// Extracts the mTLS client certificate fingerprint from the request metadata.
fn extract_author<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-client-fingerprint")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
