use crate::{disk, installer, log};
use tonic::{Request, Response, Status};

pub mod maintenance {
    tonic::include_proto!("muak.maintenance.v1");
}

use maintenance::{
    DiskInfo as ProtoDiskInfo, InstallRequest, InstallResponse, ListDisksRequest,
    ListDisksResponse, PartitionInfo as ProtoPartitionInfo,
    maintenance_service_server::MaintenanceService,
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

        // Spawn blocking task for installation
        let result =
            tokio::task::spawn_blocking(move || installer::install(&req.target_disk, req.force))
                .await;

        match result {
            Ok(Ok(())) => Ok(Response::new(InstallResponse {
                success: true,
                error: String::new(),
            })),
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
        let result = tokio::task::spawn_blocking(move || disk::list_disks()).await;

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
}
