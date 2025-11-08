use crate::{installer, log};
use tonic::{Request, Response, Status};

pub mod maintenance {
    tonic::include_proto!("muak.maintenance.v1");
}

use maintenance::{
    InstallRequest, InstallResponse,
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
}
