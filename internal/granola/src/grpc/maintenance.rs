use crate::{disk, log, provisioning};
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("muak.maintenance.v1");
}

use proto::{
    maintenance_service_server::MaintenanceService, DiskInfo as ProtoDiskInfo, InstallRequest,
    InstallResponse, ListDisksRequest, ListDisksResponse, PartitionInfo as ProtoPartitionInfo,
    UpdateRequest, UpdateResponse,
};

pub struct MaintenanceServiceImpl;

#[tonic::async_trait]
impl MaintenanceService for MaintenanceServiceImpl {
    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<InstallResponse>, Status> {
        let req = request.into_inner();

        log!(
            "maintenance",
            "Install request: target={}, force={}",
            req.target_disk,
            req.force
        );

        let version = if req.version.is_empty() {
            "latest".to_string()
        } else {
            req.version.clone()
        };
        let extensions = req.extensions.clone();
        let target_disk = req.target_disk.clone();
        let force = req.force;

        let result = tokio::task::spawn_blocking(move || {
            provisioning::install(&target_disk, force, &version, &extensions)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let response = Response::new(InstallResponse {
                    success: true,
                    error: String::new(),
                });

                tokio::spawn(async {
                    log!(
                        "maintenance",
                        "Installation successful, rebooting in 3 seconds..."
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
                    if nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT).is_err()
                    {
                        log!("maintenance", "Failed to reboot");
                    }
                });

                Ok(response)
            }
            Ok(Err(e)) => {
                log!("maintenance", "Installation failed: {}", e);
                Ok(Response::new(InstallResponse {
                    success: false,
                    error: format!("{}", e),
                }))
            }
            Err(e) => {
                log!("maintenance", "Installation task failed: {}", e);
                Ok(Response::new(InstallResponse {
                    success: false,
                    error: format!("{}", e),
                }))
            }
        }
    }

    async fn list_disks(
        &self,
        _request: Request<ListDisksRequest>,
    ) -> Result<Response<ListDisksResponse>, Status> {
        let result = tokio::task::spawn_blocking(disk::list_disks).await;

        match result {
            Ok(Ok(disks)) => {
                let proto_disks: Vec<ProtoDiskInfo> = disks
                    .into_iter()
                    .map(|d| ProtoDiskInfo {
                        name: d.name,
                        path: d.path,
                        size_bytes: d.size_bytes,
                        model: d.model,
                        removable: d.removable,
                        read_only: d.read_only,
                        partitions: d
                            .partitions
                            .into_iter()
                            .map(|p| ProtoPartitionInfo {
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
            Ok(Err(e)) => {
                log!("maintenance", "List disks failed: {}", e);
                Ok(Response::new(ListDisksResponse {
                    disks: Vec::new(),
                    error: format!("{}", e),
                }))
            }
            Err(e) => {
                log!("maintenance", "List disks task failed: {}", e);
                Ok(Response::new(ListDisksResponse {
                    disks: Vec::new(),
                    error: format!("{}", e),
                }))
            }
        }
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let req = request.into_inner();

        let version = if req.version.is_empty() {
            "latest".to_string()
        } else {
            req.version.clone()
        };

        let extensions = req.extensions.clone();

        let result = provisioning::update(&version, &extensions);

        match result {
            Ok(update_result) => Ok(Response::new(UpdateResponse {
                success: true,
                error: String::new(),
                update_id: update_result.update_id,
            })),
            Err(e) => {
                log!("maintenance", "Update failed: {}", e);
                Ok(Response::new(UpdateResponse {
                    success: false,
                    error: format!("{}", e),
                    update_id: String::new(),
                }))
            }
        }
    }
}
