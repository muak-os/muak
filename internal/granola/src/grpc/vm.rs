use crate::ipc::{IpcClient, IpcMessage, IpcResponse};
use crate::log;
use crate::vm::{DiskConfig, NetConfig, Vm, VmConfig};
use tokio::io::AsyncWriteExt;
use tonic::{Request, Response, Status};

pub mod vm_service {
    tonic::include_proto!("muak.vm.v1");
}

use vm_service::vm_service_server::{VmService, VmServiceServer};
use vm_service::{
    CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse, GetVmSerialLogRequest,
    GetVmSerialLogResponse, ListVmsRequest, ListVmsResponse, StartVmRequest, StartVmResponse,
    StopVmRequest, StopVmResponse, UploadFileRequest, UploadFileResponse, VmInfo,
};

pub struct GrpcVmService {}

impl GrpcVmService {
    pub fn new() -> Self {
        Self {}
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

        // Parse VMM type from string, default to CloudHypervisor
        let vmm_type = if req.vmm_type.is_empty() {
            crate::vmm::VmmType::default()
        } else {
            req.vmm_type.parse().unwrap_or_default()
        };

        let config = VmConfig {
            cpus: req.cpus,
            memory_mb: req.memory_mb,
            kernel: if req.kernel.is_empty() {
                None
            } else {
                Some(req.kernel)
            },
            initrd: if req.initrd.is_empty() {
                None
            } else {
                Some(req.initrd)
            },
            cmdline: if req.cmdline.is_empty() {
                None
            } else {
                Some(req.cmdline)
            },
            disks,
            networks,
            vmm_type,
        };

        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::CreateVm {
            name: req.name.clone(),
            config,
        };

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::VmCreated { vm_id }) => {
                log!("grpc-vm", "Created VM: {}", vm_id);
                Ok(Response::new(CreateVmResponse {
                    vm_id,
                    error: String::new(),
                }))
            }
            Ok(IpcResponse::Error(e)) => Ok(Response::new(CreateVmResponse {
                vm_id: String::new(),
                error: e,
            })),
            Err(e) => Ok(Response::new(CreateVmResponse {
                vm_id: String::new(),
                error: format!("IPC error: {}", e),
            })),
            _ => Ok(Response::new(CreateVmResponse {
                vm_id: String::new(),
                error: "Unexpected IPC response".to_string(),
            })),
        }
    }

    async fn start_vm(
        &self,
        request: Request<StartVmRequest>,
    ) -> Result<Response<StartVmResponse>, Status> {
        let req = request.into_inner();
        log!("grpc-vm", "Attempting to start VM: {}", req.vm_id);

        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::StartVm {
            vm_id: req.vm_id.clone(),
        };

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::Ok) => {
                log!("grpc-vm", "Successfully started VM: {}", req.vm_id);
                Ok(Response::new(StartVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(IpcResponse::Error(e)) => {
                log!("grpc-vm", "Failed to start VM {}: {}", req.vm_id, e);
                Ok(Response::new(StartVmResponse {
                    success: false,
                    error: e,
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to start VM {}: {}", req.vm_id, e);
                Ok(Response::new(StartVmResponse {
                    success: false,
                    error: format!("IPC error: {}", e),
                }))
            }
            _ => Ok(Response::new(StartVmResponse {
                success: false,
                error: "Unexpected IPC response".to_string(),
            })),
        }
    }

    async fn stop_vm(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();
        log!(
            "grpc-vm",
            "Attempting to stop VM: {} (force: {})",
            req.vm_id,
            req.force
        );

        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::StopVm {
            vm_id: req.vm_id.clone(),
            force: req.force,
        };

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::Ok) => {
                log!("grpc-vm", "Successfully stopped VM: {}", req.vm_id);
                Ok(Response::new(StopVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(IpcResponse::Error(e)) => {
                log!("grpc-vm", "Failed to stop VM {}: {}", req.vm_id, e);
                Ok(Response::new(StopVmResponse {
                    success: false,
                    error: e,
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to stop VM {}: {}", req.vm_id, e);
                Ok(Response::new(StopVmResponse {
                    success: false,
                    error: format!("IPC error: {}", e),
                }))
            }
            _ => Ok(Response::new(StopVmResponse {
                success: false,
                error: "Unexpected IPC response".to_string(),
            })),
        }
    }

    async fn delete_vm(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();
        log!("grpc-vm", "Attempting to delete VM: {}", req.vm_id);

        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::DeleteVm {
            vm_id: req.vm_id.clone(),
        };

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::Ok) => {
                log!("grpc-vm", "Successfully deleted VM: {}", req.vm_id);
                Ok(Response::new(DeleteVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Ok(IpcResponse::Error(e)) => {
                log!("grpc-vm", "Failed to delete VM {}: {}", req.vm_id, e);
                Ok(Response::new(DeleteVmResponse {
                    success: false,
                    error: e,
                }))
            }
            Err(e) => {
                log!("grpc-vm", "Failed to delete VM {}: {}", req.vm_id, e);
                Ok(Response::new(DeleteVmResponse {
                    success: false,
                    error: format!("IPC error: {}", e),
                }))
            }
            _ => Ok(Response::new(DeleteVmResponse {
                success: false,
                error: "Unexpected IPC response".to_string(),
            })),
        }
    }

    async fn list_vms(
        &self,
        _request: Request<ListVmsRequest>,
    ) -> Result<Response<ListVmsResponse>, Status> {
        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::ListVms;

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::VmList(data)) => match bincode::deserialize::<Vec<Vm>>(&data) {
                Ok(vms) => {
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
                            vmm_type: v.config.vmm_type.to_string(),
                        })
                        .collect();

                    Ok(Response::new(ListVmsResponse { vms: vm_infos }))
                }
                Err(e) => Err(Status::internal(format!(
                    "Failed to deserialize VMs: {}",
                    e
                ))),
            },
            Ok(IpcResponse::Error(e)) => Err(Status::internal(e)),
            Err(e) => Err(Status::internal(format!("IPC error: {}", e))),
            _ => Err(Status::internal("Unexpected IPC response")),
        }
    }

    async fn upload_file(
        &self,
        request: Request<tonic::Streaming<UploadFileRequest>>,
    ) -> Result<Response<UploadFileResponse>, Status> {
        let mut stream = request.into_inner();
        let mut file: Option<tokio::fs::File> = None;
        let mut filepath = String::new();
        let mut bytes_written: u64 = 0;

        while let Some(req) = stream.message().await? {
            match req.request {
                Some(vm_service::upload_file_request::Request::Metadata(metadata)) => {
                    let filename = metadata.filename;
                    filepath = format!("/tmp/muak/disks/{}", filename);

                    log!(
                        "grpc-vm",
                        "Starting file upload: {} ({} bytes)",
                        filename,
                        metadata.size
                    );

                    // Ensure directory exists
                    if let Err(e) = tokio::fs::create_dir_all("/tmp/muak/disks").await {
                        let error = format!("Failed to create upload directory: {}", e);
                        log!("grpc-vm", "{}", error);
                        return Ok(Response::new(UploadFileResponse {
                            path: String::new(),
                            error,
                        }));
                    }

                    match tokio::fs::File::create(&filepath).await {
                        Ok(f) => {
                            file = Some(f);
                        }
                        Err(e) => {
                            let error = format!("Failed to create file: {}", e);
                            log!("grpc-vm", "{}", error);
                            return Ok(Response::new(UploadFileResponse {
                                path: String::new(),
                                error,
                            }));
                        }
                    }
                }
                Some(vm_service::upload_file_request::Request::Chunk(chunk)) => {
                    if let Some(ref mut f) = file {
                        match f.write_all(&chunk).await {
                            Ok(_) => {
                                bytes_written += chunk.len() as u64;
                            }
                            Err(e) => {
                                let error = format!("Failed to write chunk: {}", e);
                                log!("grpc-vm", "{}", error);
                                return Ok(Response::new(UploadFileResponse {
                                    path: String::new(),
                                    error,
                                }));
                            }
                        }
                    } else {
                        let error = "Received chunk before metadata".to_string();
                        log!("grpc-vm", "{}", error);
                        return Ok(Response::new(UploadFileResponse {
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
                return Ok(Response::new(UploadFileResponse {
                    path: String::new(),
                    error,
                }));
            }
            log!(
                "grpc-vm",
                "File upload complete: {} ({} bytes)",
                filepath,
                bytes_written
            );
        }

        Ok(Response::new(UploadFileResponse {
            path: filepath,
            error: String::new(),
        }))
    }

    async fn get_vm_serial_log(
        &self,
        request: Request<GetVmSerialLogRequest>,
    ) -> Result<Response<GetVmSerialLogResponse>, Status> {
        let req = request.into_inner();
        log!(
            "grpc-vm",
            "Fetching serial log for VM: {} (tail: {})",
            req.vm_id,
            req.tail_lines
        );

        let mut ipc_client = IpcClient::new();
        let message = IpcMessage::GetVmSerialLog {
            vm_id: req.vm_id.clone(),
            tail_lines: req.tail_lines,
        };

        match ipc_client.send_message(&message) {
            Ok(IpcResponse::VmSerialLog(output)) => {
                log!(
                    "grpc-vm",
                    "Retrieved serial log for VM: {} ({} bytes)",
                    req.vm_id,
                    output.len()
                );
                Ok(Response::new(GetVmSerialLogResponse {
                    output,
                    error: String::new(),
                }))
            }
            Ok(IpcResponse::Error(e)) => {
                log!(
                    "grpc-vm",
                    "Failed to get serial log for VM {}: {}",
                    req.vm_id,
                    e
                );
                Ok(Response::new(GetVmSerialLogResponse {
                    output: String::new(),
                    error: e,
                }))
            }
            Err(e) => {
                log!(
                    "grpc-vm",
                    "IPC error getting serial log for VM {}: {}",
                    req.vm_id,
                    e
                );
                Ok(Response::new(GetVmSerialLogResponse {
                    output: String::new(),
                    error: format!("IPC error: {}", e),
                }))
            }
            _ => Ok(Response::new(GetVmSerialLogResponse {
                output: String::new(),
                error: "Unexpected IPC response".to_string(),
            })),
        }
    }
}

pub fn service() -> VmServiceServer<GrpcVmService> {
    VmServiceServer::new(GrpcVmService::new())
}
