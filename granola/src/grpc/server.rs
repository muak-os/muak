use crate::log;
use tonic::transport::Server;

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = crate::config::GRPC_SERVER_ADDR.parse()?;
    log!("grpc", "gRPC server starting on {}", addr);

    let process_service = super::grpc_process::service();
    let vm_service = super::grpc_vm::service();

    Server::builder()
        .add_service(process_service)
        .add_service(vm_service)
        .serve(addr)
        .await?;

    Ok(())
}
