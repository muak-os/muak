use super::maintenance::{
    MaintenanceServiceImpl, maintenance::maintenance_service_server::MaintenanceServiceServer,
};
use crate::log;
use tonic::transport::Server;

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = crate::config::GRPC_SERVER_ADDR.parse()?;
    log!("grpc", "gRPC server starting on {}", addr);

    let process_service = super::process::service();
    let vm_service = super::vm::service();
    let maintenance_service = MaintenanceServiceServer::new(MaintenanceServiceImpl);

    Server::builder()
        .add_service(process_service)
        .add_service(vm_service)
        .add_service(maintenance_service)
        .serve(addr)
        .await?;

    Ok(())
}
