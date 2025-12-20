//! VmService implementation
//!
//! This service provides VM lifecycle management.
//! Note: In the full architecture, this will delegate to vmd over Unix socket.
//! For now, VM operations are stubbed out until vmd is implemented.

use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("muak.vm.v1");
}

use proto::vm_service_server::{VmService, VmServiceServer};
use proto::{
    CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse, GetVmSerialLogRequest,
    GetVmSerialLogResponse, ListVmsRequest, ListVmsResponse, StartVmRequest, StartVmResponse,
    StopVmRequest, StopVmResponse, UploadFileRequest, UploadFileResponse,
};

/// Create the VmService gRPC service
pub fn service() -> VmServiceServer<VmServiceImpl> {
    VmServiceServer::new(VmServiceImpl)
}

pub struct VmServiceImpl;

#[tonic::async_trait]
impl VmService for VmServiceImpl {
    async fn create_vm(
        &self,
        request: Request<CreateVmRequest>,
    ) -> Result<Response<CreateVmResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!(
            "CreateVm request: name={}, cpus={}, memory={}MB",
            req.name,
            req.cpus,
            req.memory_mb
        );

        // TODO: Delegate to vmd when implemented
        Err(Status::unimplemented(
            "VM management requires vmd service (not yet implemented)",
        ))
    }

    async fn start_vm(
        &self,
        request: Request<StartVmRequest>,
    ) -> Result<Response<StartVmResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!("StartVm request: vm_id={}", req.vm_id);

        // TODO: Delegate to vmd when implemented
        Err(Status::unimplemented(
            "VM management requires vmd service (not yet implemented)",
        ))
    }

    async fn stop_vm(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!("StopVm request: vm_id={}, force={}", req.vm_id, req.force);

        // TODO: Delegate to vmd when implemented
        Err(Status::unimplemented(
            "VM management requires vmd service (not yet implemented)",
        ))
    }

    async fn delete_vm(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!("DeleteVm request: vm_id={}", req.vm_id);

        // TODO: Delegate to vmd when implemented
        Err(Status::unimplemented(
            "VM management requires vmd service (not yet implemented)",
        ))
    }

    async fn list_vms(
        &self,
        _request: Request<ListVmsRequest>,
    ) -> Result<Response<ListVmsResponse>, Status> {
        kmsg::info!("ListVms request");

        // TODO: Delegate to vmd when implemented
        // For now, return empty list
        Ok(Response::new(ListVmsResponse { vms: vec![] }))
    }

    async fn upload_file(
        &self,
        _request: Request<tonic::Streaming<UploadFileRequest>>,
    ) -> Result<Response<UploadFileResponse>, Status> {
        kmsg::info!("UploadFile request");

        // TODO: Implement file upload for VM kernels/initrds
        Err(Status::unimplemented("File upload not yet implemented"))
    }

    async fn get_vm_serial_log(
        &self,
        request: Request<GetVmSerialLogRequest>,
    ) -> Result<Response<GetVmSerialLogResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!(
            "GetVmSerialLog request: vm_id={}, tail_lines={}",
            req.vm_id,
            req.tail_lines
        );

        // TODO: Delegate to vmd when implemented
        Err(Status::unimplemented(
            "VM management requires vmd service (not yet implemented)",
        ))
    }
}
