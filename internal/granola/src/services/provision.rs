use std::pin::Pin;

use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use tonic::{Request, Response, Status};

use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::*;

use crate::disk;
use crate::provisioning;
use sysconfig;

pub fn service() -> ProvisionServiceServer<ProvisionServiceImpl> {
    ProvisionServiceServer::new(ProvisionServiceImpl)
}

pub struct ProvisionServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ProvisionServiceImpl {
    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<InstallResponse>, Status> {
        let req = request.into_inner();

        if req.csr.is_empty() {
            return Err(Status::invalid_argument("CSR is required"));
        }

        let config_toml = String::from_utf8(req.config_toml)
            .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8 in config: {}", e)))?;

        let config = sysconfig::parse_from_str(&config_toml)
            .map_err(|e| Status::invalid_argument(format!("Invalid config: {}", e)))?;

        config
            .validate_for_install()
            .map_err(|e| Status::invalid_argument(format!("Invalid config for install: {}", e)))?;

        kmsg::info!(
            "Install request: disk={}, force={}, image={}",
            config.system.disk,
            req.force,
            config.system.image
        );

        let server_name = config.system.name.clone();

        match provisioning::install(req.force, config, req.csr).await {
            Ok(result) => {
                let ca_pem = result.ca_pem.clone();
                let client_cert_pem = result.admin_cert_pem.clone();

                tokio::spawn(async {
                    kmsg::info!("System will reboot in 3 seconds...");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    kmsg::info!("Rebooting now...");
                    sync();
                    let _ = reboot(RebootCommand::Restart);
                });

                Ok(Response::new(InstallResponse {
                    success: true,
                    error: String::new(),
                    ca_pem,
                    client_cert_pem,
                    server_name,
                }))
            }
            Err(e) => Ok(Response::new(InstallResponse {
                success: false,
                error: format!("{}", e),
                ca_pem: String::new(),
                client_cert_pem: String::new(),
                server_name: String::new(),
            })),
        }
    }

    async fn prepare_update(
        &self,
        request: Request<PrepareUpdateRequest>,
    ) -> Result<Response<PrepareUpdateResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!("Update request: image={}", req.image);

        let config = sysconfig::config();

        match provisioning::prepare_update(&req.image, &config.system.extensions).await {
            Ok(update_id) => Ok(Response::new(PrepareUpdateResponse {
                success: true,
                update_id,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(PrepareUpdateResponse {
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
        if let Err(e) = provisioning::update(&request.into_inner().update_id) {
            return Ok(Response::new(UpdateResponse {
                success: false,
                error: format!("{}", e),
            }));
        }

        unreachable!("If we're here, something went really wrong in kexec")
    }

    async fn get_update_status(
        &self,
        request: Request<GetUpdateStatusRequest>,
    ) -> Result<Response<GetUpdateStatusResponse>, Status> {
        let update_id = request.into_inner().update_id;

        let status = tokio::task::spawn_blocking({
            let update_id = update_id.clone();
            move || provisioning::get_update_status(&update_id)
        })
        .await
        .map_err(|e| Status::internal(format!("Task failed: {}", e)))?;

        let (proto_status, error) = match status {
            provisioning::UpdateStatus::Unknown => (0, String::new()),
            provisioning::UpdateStatus::Pending => (1, String::new()),
            provisioning::UpdateStatus::Committed => (2, String::new()),
            provisioning::UpdateStatus::RolledBack(reason) => (3, reason),
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
        let disks = tokio::task::spawn_blocking(disk::list_disks)
            .await
            .map_err(|e| Status::internal(format!("Task failed: {}", e)))?
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

    type GetLogsStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<GetLogsResponse, Status>> + Send>>;

    async fn get_logs(
        &self,
        _request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(e) = stream_kernel_logs(tx).await {
                kmsg::warn!("Error streaming logs: {}", e);
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
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
        match tokio::task::spawn_blocking(provisioning::factory_reset).await {
            Ok(Ok(())) => {
                tokio::spawn(async {
                    kmsg::info!("System will reboot in 3 seconds...");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    kmsg::info!("Rebooting now...");
                    sync();
                    let _ = reboot(RebootCommand::Restart);
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

async fn stream_kernel_logs(
    tx: tokio::sync::mpsc::Sender<Result<GetLogsResponse, Status>>,
) -> Result<(), std::io::Error> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open("/dev/kmsg")
        .await?;

    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let message = parse_kmsg_line(&line);

        if tx
            .send(Ok(GetLogsResponse {
                line: message,
                error: String::new(),
            }))
            .await
            .is_err()
        {
            break;
        }
    }

    Ok(())
}

fn parse_kmsg_line(line: &str) -> String {
    if let Some(idx) = line.find(';') {
        line[idx + 1..].to_string()
    } else {
        line.to_string()
    }
}
