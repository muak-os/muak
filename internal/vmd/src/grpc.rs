use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::actor::VmActorHandle;
use crate::proto::vm::{
    self, CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse, GetVmRequest,
    GetVmResponse, GetVmSerialLogRequest, GetVmSerialLogResponse, ListVmsRequest, ListVmsResponse,
    StartVmRequest, StartVmResponse, StopVmRequest, StopVmResponse, UploadFileRequest,
    UploadFileResponse, upload_file_request,
};

pub struct VmServiceImpl {
    handle: VmActorHandle,
}

impl VmServiceImpl {
    pub fn new(handle: VmActorHandle) -> Self {
        Self { handle }
    }
}

#[tonic::async_trait]
impl vm::vm_service_server::VmService for VmServiceImpl {
    async fn create_vm(
        &self,
        request: Request<CreateVmRequest>,
    ) -> Result<Response<CreateVmResponse>, Status> {
        let req = request.into_inner();
        let config = req
            .config
            .ok_or_else(|| Status::invalid_argument("config is required"))?;

        match self.handle.create(config).await {
            Ok(vm_id) => Ok(Response::new(CreateVmResponse {
                vm_id,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(CreateVmResponse {
                vm_id: String::new(),
                error: format!("Failed to create VM: {}", e),
            })),
        }
    }

    async fn start_vm(
        &self,
        request: Request<StartVmRequest>,
    ) -> Result<Response<StartVmResponse>, Status> {
        let req = request.into_inner();

        match self.handle.start(req.vm_id).await {
            Ok(()) => Ok(Response::new(StartVmResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(StartVmResponse {
                success: false,
                error: format!("Failed to start VM: {}", e),
            })),
        }
    }

    async fn stop_vm(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();

        match self.handle.stop(req.vm_id, req.force).await {
            Ok(()) => Ok(Response::new(StopVmResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(StopVmResponse {
                success: false,
                error: format!("Failed to stop VM: {}", e),
            })),
        }
    }

    async fn delete_vm(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();

        match self.handle.delete(req.vm_id).await {
            Ok(()) => Ok(Response::new(DeleteVmResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(DeleteVmResponse {
                success: false,
                error: format!("Failed to delete VM: {}", e),
            })),
        }
    }

    async fn get_vm(
        &self,
        request: Request<GetVmRequest>,
    ) -> Result<Response<GetVmResponse>, Status> {
        let req = request.into_inner();

        match self.handle.get(req.vm_id).await {
            Ok(vm) => Ok(Response::new(GetVmResponse {
                vm: Some(vm),
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(GetVmResponse {
                vm: None,
                error: format!("VM not found: {}", e),
            })),
        }
    }

    async fn list_vms(
        &self,
        _request: Request<ListVmsRequest>,
    ) -> Result<Response<ListVmsResponse>, Status> {
        let vms = self
            .handle
            .list()
            .await
            .map_err(|e| Status::internal(format!("Failed to list VMs: {}", e)))?;

        Ok(Response::new(ListVmsResponse { vms }))
    }

    async fn upload_file(
        &self,
        request: Request<Streaming<UploadFileRequest>>,
    ) -> Result<Response<UploadFileResponse>, Status> {
        let mut stream = request.into_inner();

        let mut filename = String::new();
        let mut data = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;

            match chunk.request {
                Some(upload_file_request::Request::Metadata(meta)) => {
                    filename = meta.filename;
                    if meta.size > 0 {
                        data.reserve(meta.size as usize);
                    }
                }
                Some(upload_file_request::Request::Chunk(bytes)) => {
                    data.extend_from_slice(&bytes);
                }
                None => {}
            }
        }

        if filename.is_empty() {
            return Err(Status::invalid_argument("filename is required"));
        }

        match self.handle.upload_file(filename, data).await {
            Ok(path) => Ok(Response::new(UploadFileResponse {
                path,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(UploadFileResponse {
                path: String::new(),
                error: format!("Failed to upload file: {}", e),
            })),
        }
    }

    async fn get_vm_serial_log(
        &self,
        request: Request<GetVmSerialLogRequest>,
    ) -> Result<Response<GetVmSerialLogResponse>, Status> {
        let req = request.into_inner();

        match self.handle.get_serial_log(req.vm_id, req.tail_lines).await {
            Ok(output) => Ok(Response::new(GetVmSerialLogResponse {
                output,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(GetVmSerialLogResponse {
                output: String::new(),
                error: format!("Failed to get serial log: {}", e),
            })),
        }
    }
}
