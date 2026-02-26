//! gRPC service implementation for provisioning operations.

use std::pin::Pin;

use anyhow::Context;
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use tonic::{Request, Response, Status};

use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::*;
use crate::disk;
use crate::install;
use crate::reset;
use crate::update;
use crate::validation;

/// Creates the ProvisionService gRPC server.
pub fn service() -> ProvisionServiceServer<ProvisionServiceImpl> {
    ProvisionServiceServer::new(ProvisionServiceImpl)
}

/// Implementation of the ProvisionService gRPC interface.
pub struct ProvisionServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ProvisionServiceImpl {
    type InstallStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<InstallProgress, Status>> + Send>>;

    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<Self::InstallStream>, Status> {
        let req = request.into_inner();

        if req.csr.is_empty() {
            return Err(Status::invalid_argument("CSR is required"));
        }

        let config_raw = String::from_utf8(req.config_toml)
            .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8 in config: {}", e)))?;

        let config: sysconfig::HostConfig = sysconfig::parse_from_str(&config_raw)
            .map_err(|e| Status::invalid_argument(format!("Invalid config: {}", e)))?;

        config
            .validate_for_install()
            .map_err(|e| Status::invalid_argument(format!("Invalid config for install: {}", e)))?;

        println!(
            "Install request: disk={}, force={}, image={}",
            config.system.disk, config.system.image, req.force,
        );

        let server_name = config.system.name.clone();
        let force = req.force;
        let csr = req.csr;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<InstallProgress, Status>>(32);

        tokio::spawn(async move {
            let progress_tx = tx.clone();
            let (install_tx, mut install_rx) = tokio::sync::mpsc::channel::<InstallProgress>(32);

            let forward_tx = tx.clone();
            let forward_handle = tokio::spawn(async move {
                while let Some(progress) = install_rx.recv().await {
                    if forward_tx.send(Ok(progress)).await.is_err() {
                        break;
                    }
                }
            });

            let disk = config.system.disk.clone();
            let result = tokio::task::spawn_blocking(move || {
                install::install(&disk, force, &config, &csr, install_tx)
            })
            .await;

            let _ = forward_handle.await;

            let result = match result {
                Ok(inner) => inner,
                Err(e) => Err(anyhow::anyhow!("Install task panicked: {}", e)),
            };

            match result {
                Ok(result) => {
                    let _ = progress_tx
                        .send(Ok(InstallProgress {
                            step: InstallStep::Completed as i32,
                            message: "Installation completed successfully".to_string(),
                            ca_pem: result.ca_pem,
                            client_cert_pem: result.admin_cert_pem,
                            server_name,
                            ..Default::default()
                        }))
                        .await;

                    tokio::spawn(async {
                        kmsg::info!("System will reboot in 3 seconds...");
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        kmsg::info!("Rebooting now...");
                        tokio::task::spawn_blocking(|| {
                            sync();
                            let _ = reboot(RebootCommand::Restart);
                        })
                        .await
                        .ok();
                    });
                }
                Err(e) => {
                    let _ = progress_tx
                        .send(Ok(InstallProgress {
                            step: InstallStep::Failed as i32,
                            message: "Installation failed".to_string(),
                            error: format!("{}", e),
                            ..Default::default()
                        }))
                        .await;
                }
            }
        });

        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
    }

    async fn prepare_update(
        &self,
        request: Request<PrepareUpdateRequest>,
    ) -> Result<Response<PrepareUpdateResponse>, Status> {
        let req = request.into_inner();
        println!("Update request: image={}", req.image);

        let config = sysconfig::config();
        let extensions = config.system.extensions.clone();
        let image = req.image;

        match tokio::task::spawn_blocking(move || update::prepare(&image, &extensions))
            .await
            .context("Prepare update task panicked")
        {
            Ok(Ok(update_id)) => Ok(Response::new(PrepareUpdateResponse {
                success: true,
                update_id,
                error: String::new(),
            })),
            Ok(Err(e)) | Err(e) => Ok(Response::new(PrepareUpdateResponse {
                success: false,
                update_id: String::new(),
                error: format!("{}", e),
            })),
        }
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let update_id = request.into_inner().update_id;

        match tokio::task::spawn_blocking(move || update::update(&update_id)).await {
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

        let status = tokio::task::spawn_blocking(move || validation::get_update_status(&update_id))
            .await
            .map_err(|e| Status::internal(format!("Task failed: {}", e)))?;

        let (proto_status, error) = match status {
            validation::UpdateStatus::Unknown => (0, String::new()),
            validation::UpdateStatus::Pending => (1, String::new()),
            validation::UpdateStatus::Committed => (2, String::new()),
            validation::UpdateStatus::RolledBack(reason) => (3, reason),
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

        let proto_disks: Vec<DiskInfo> = disks
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
                Ok(config_toml) => Ok(Response::new(GetConfigResponse {
                    config: config_toml.into_bytes(),
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

    async fn factory_reset(
        &self,
        _request: Request<FactoryResetRequest>,
    ) -> Result<Response<FactoryResetResponse>, Status> {
        match tokio::task::spawn_blocking(reset::factory_reset).await {
            Ok(Ok(())) => {
                tokio::spawn(async {
                    kmsg::info!("System will reboot in 3 seconds...");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    kmsg::info!("Rebooting now...");
                    tokio::task::spawn_blocking(|| {
                        sync();
                        let _ = reboot(RebootCommand::Restart);
                    })
                    .await
                    .ok();
                });

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
