use crate::log;
use crate::vm::VmManager;
use tonic::transport::Server;

pub async fn main(vm_manager: VmManager) -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051".parse()?;
    log!("grpc", "gRPC server starting on {}", addr);

    let process_service = super::grpc_process::service();
    let vm_service = super::grpc_vm::service(vm_manager);

    Server::builder()
        .add_service(process_service)
        .add_service(vm_service)
        .serve(addr)
        .await?;

    Ok(())
}
