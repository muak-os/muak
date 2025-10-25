use crate::log;
use crate::vm::{DiskConfig, NetConfig, Vm, VmConfig, VmManager};
use tonic::{Request, Response, Status};
use tokio::io::AsyncWriteExt;

pub mod vm_service {
    tonic::include_proto!("muak.vm.v1");
}

use vm_service::vm_service_server::{VmService, VmServiceServer};
use vm_service::{
    CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse, ListVmsRequest,
    ListVmsResponse, StartVmRequest, StartVmResponse, StopVmRequest, StopVmResponse, VmInfo,
    UploadDiskRequest, UploadDiskResponse,
};

pub struct GrpcVmService {
    vm_manager: VmManager,
}

impl GrpcVmService {
    pub fn new(vm_manager: VmManager) -> Self {
        Self { vm_manager }
    }
}

#[tonic::async_trait]
impl VmService for GrpcVmService {
    async fn create_vm(
        &self,
        request: Request<CreateVmRequest>,
    ) -> Result<Response<CreateVmResponse>, Status> {
        let req = request.into_inner();

        let disks: Vec<DiskConfig> = req
            .disks
            .into_iter()
            .map(|d| DiskConfig {
                path: d.path,
                readonly: d.readonly,
            })
            .collect();

        let networks: Vec<NetConfig> = req
            .networks
            .into_iter()
            .map(|n| NetConfig {
                tap: n.tap,
                mac: n.mac,
            })
            .collect();

        let config = VmConfig {
            cpus: req.cpus,
            memory_mb: req.memory_mb,
            kernel: req.kernel,
            cmdline: if req.cmdline.is_empty() {
                None
            } else {
                Some(req.cmdline)
            },
            disks,
            networks,
        };

        match self.vm_manager.create(req.name, config) {
            Ok(vm_id) => {
                log!("grpc-vm", "Created VM: {}", vm_id);
                Ok(Response::new(CreateVmResponse {
                    vm_id,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(CreateVmResponse {
                vm_id: String::new(),
                error: e,
            })),
        }
    }

    async fn start_vm(
        &self,
        request: Request<StartVmRequest>,
    ) -> Result<Response<StartVmResponse>, Status> {
        let req = request.into_inner();
        log!("grpc-vm", "Attempting to start VM: {}", req.vm_id);

        match self.vm_manager.start(&req.vm_id) {
            Ok(_) => {
                log!("grpc-vm", "Successfully started VM: {}", req.vm_id);
                Ok(Response::new(StartVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to start VM {}: {}", req.vm_id, e);
                Ok(Response::new(StartVmResponse {
                    success: false,
                    error: e,
                }))
            }
        }
    }

    async fn stop_vm(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();
        log!("grpc-vm", "Attempting to stop VM: {} (force: {})", req.vm_id, req.force);

        match self.vm_manager.stop(&req.vm_id, req.force) {
            Ok(_) => {
                log!("grpc-vm", "Successfully stopped VM: {}", req.vm_id);
                Ok(Response::new(StopVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to stop VM {}: {}", req.vm_id, e);
                Ok(Response::new(StopVmResponse {
                    success: false,
                    error: e,
                }))
            }
        }
    }

    async fn delete_vm(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();
        log!("grpc-vm", "Attempting to delete VM: {}", req.vm_id);

        match self.vm_manager.delete(&req.vm_id) {
            Ok(_) => {
                log!("grpc-vm", "Successfully deleted VM: {}", req.vm_id);
                Ok(Response::new(DeleteVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to delete VM {}: {}", req.vm_id, e);
                Ok(Response::new(DeleteVmResponse {
                    success: false,
                    error: e,
                }))
            }
        }
    }

    async fn list_vms(
        &self,
        _request: Request<ListVmsRequest>,
    ) -> Result<Response<ListVmsResponse>, Status> {
        let vms: Vec<Vm> = self.vm_manager.list();

        let vm_infos: Vec<VmInfo> = vms
            .into_iter()
            .map(|v| VmInfo {
                vm_id: v.vm_id,
                name: v.name,
                state: v.state.to_string(),
                cpus: v.config.cpus,
                memory_mb: v.config.memory_mb,
                pid: v.pid.unwrap_or(-1),
                created_at: v.created_at,
            })
            .collect();

        Ok(Response::new(ListVmsResponse { vms: vm_infos }))
    }

    async fn upload_disk(
        &self,
        request: Request<tonic::Streaming<UploadDiskRequest>>,
    ) -> Result<Response<UploadDiskResponse>, Status> {
        let mut stream = request.into_inner();
        let mut file: Option<tokio::fs::File> = None;
        let mut filepath = String::new();
        let mut bytes_written: u64 = 0;

        while let Some(req) = stream.message().await? {
            match req.request {
                Some(vm_service::upload_disk_request::Request::Metadata(metadata)) => {
                    let filename = metadata.filename;
                    filepath = format!("/tmp/muak/disks/{}", filename);
                    
                    log!("grpc-vm", "Starting disk upload: {} ({} bytes)", filename, metadata.size);
                    
                    match tokio::fs::File::create(&filepath).await {
                        Ok(f) => {
                            file = Some(f);
                        }
                        Err(e) => {
                            let error = format!("Failed to create file: {}", e);
                            log!("grpc-vm", "{}", error);
                            return Ok(Response::new(UploadDiskResponse {
                                path: String::new(),
                                error,
                            }));
                        }
                    }
                }
                Some(vm_service::upload_disk_request::Request::Chunk(chunk)) => {
                    if let Some(ref mut f) = file {
                        match f.write_all(&chunk).await {
                            Ok(_) => {
                                bytes_written += chunk.len() as u64;
                            }
                            Err(e) => {
                                let error = format!("Failed to write chunk: {}", e);
                                log!("grpc-vm", "{}", error);
                                return Ok(Response::new(UploadDiskResponse {
                                    path: String::new(),
                                    error,
                                }));
                            }
                        }
                    } else {
                        let error = "Received chunk before metadata".to_string();
                        log!("grpc-vm", "{}", error);
                        return Ok(Response::new(UploadDiskResponse {
                            path: String::new(),
                            error,
                        }));
                    }
                }
                None => {}
            }
        }

        if let Some(mut f) = file {
            if let Err(e) = f.flush().await {
                let error = format!("Failed to flush file: {}", e);
                log!("grpc-vm", "{}", error);
                return Ok(Response::new(UploadDiskResponse {
                    path: String::new(),
                    error,
                }));
            }
            log!("grpc-vm", "Disk upload complete: {} ({} bytes)", filepath, bytes_written);
        }

        Ok(Response::new(UploadDiskResponse {
            path: filepath,
            error: String::new(),
        }))
    }
}

pub fn service(vm_manager: VmManager) -> VmServiceServer<GrpcVmService> {
    VmServiceServer::new(GrpcVmService::new(vm_manager))
}
