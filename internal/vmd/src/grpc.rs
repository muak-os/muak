use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::actor::VmActorHandle;
use crate::proto::vm::{
    self, CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse,
    GetSerialLogRequest, GetSerialLogResponse, GetVmRequest, ListVmsRequest, ListVmsResponse,
    StartVmRequest, StartVmResponse, StopVmRequest, StopVmResponse, UploadFileChunk,
    UploadFileResponse, VmInfo, upload_file_chunk,
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
    async fn create(
        &self,
        request: Request<CreateVmRequest>,
    ) -> Result<Response<CreateVmResponse>, Status> {
        let req = request.into_inner();
        let config = req
            .config
            .ok_or_else(|| Status::invalid_argument("config is required"))?;

        let vm_id = self
            .handle
            .create(config)
            .await
            .map_err(|e| Status::internal(format!("Failed to create VM: {}", e)))?;

        Ok(Response::new(CreateVmResponse { vm_id }))
    }

    async fn start(
        &self,
        request: Request<StartVmRequest>,
    ) -> Result<Response<StartVmResponse>, Status> {
        let req = request.into_inner();

        self.handle
            .start(req.vm_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to start VM: {}", e)))?;

        Ok(Response::new(StartVmResponse {}))
    }

    async fn stop(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();

        self.handle
            .stop(req.vm_id, req.force)
            .await
            .map_err(|e| Status::internal(format!("Failed to stop VM: {}", e)))?;

        Ok(Response::new(StopVmResponse {}))
    }

    async fn delete(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();

        self.handle
            .delete(req.vm_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete VM: {}", e)))?;

        Ok(Response::new(DeleteVmResponse {}))
    }

    async fn get(&self, request: Request<GetVmRequest>) -> Result<Response<VmInfo>, Status> {
        let req = request.into_inner();

        let vm = self
            .handle
            .get(req.vm_id)
            .await
            .map_err(|e| Status::not_found(format!("VM not found: {}", e)))?;

        Ok(Response::new(vm))
    }

    async fn list(
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
        request: Request<Streaming<UploadFileChunk>>,
    ) -> Result<Response<UploadFileResponse>, Status> {
        let mut stream = request.into_inner();

        let mut filename = String::new();
        let mut data = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;

            match chunk.data {
                Some(upload_file_chunk::Data::Metadata(meta)) => {
                    filename = meta.filename;
                    if meta.size > 0 {
                        data.reserve(meta.size as usize);
                    }
                }
                Some(upload_file_chunk::Data::Chunk(bytes)) => {
                    data.extend_from_slice(&bytes);
                }
                None => {}
            }
        }

        if filename.is_empty() {
            return Err(Status::invalid_argument("filename is required"));
        }

        let path = self
            .handle
            .upload_file(filename, data)
            .await
            .map_err(|e| Status::internal(format!("Failed to upload file: {}", e)))?;

        Ok(Response::new(UploadFileResponse { path }))
    }

    async fn get_serial_log(
        &self,
        request: Request<GetSerialLogRequest>,
    ) -> Result<Response<GetSerialLogResponse>, Status> {
        let req = request.into_inner();

        let content = self
            .handle
            .get_serial_log(req.vm_id, req.tail_lines)
            .await
            .map_err(|e| Status::internal(format!("Failed to get serial log: {}", e)))?;

        Ok(Response::new(GetSerialLogResponse { content }))
    }
}
