use crate::log;
use crate::vm::{DiskConfig, NetConfig, Vm, VmConfig, VmManager};
use tonic::{Request, Response, Status};

pub mod vm_service {
    tonic::include_proto!("muak.vm.v1");
}

use vm_service::vm_service_server::{VmService, VmServiceServer};
use vm_service::{
    CreateVmRequest, CreateVmResponse, DeleteVmRequest, DeleteVmResponse, ListVmsRequest,
    ListVmsResponse, StartVmRequest, StartVmResponse, StopVmRequest, StopVmResponse, VmInfo,
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

        match self.vm_manager.start(&req.vm_id) {
            Ok(_) => {
                log!("grpc-vm", "Started VM: {}", req.vm_id);
                Ok(Response::new(StartVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(StartVmResponse {
                success: false,
                error: e,
            })),
        }
    }

    async fn stop_vm(
        &self,
        request: Request<StopVmRequest>,
    ) -> Result<Response<StopVmResponse>, Status> {
        let req = request.into_inner();

        match self.vm_manager.stop(&req.vm_id, req.force) {
            Ok(_) => {
                log!("grpc-vm", "Stopped VM: {}", req.vm_id);
                Ok(Response::new(StopVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(StopVmResponse {
                success: false,
                error: e,
            })),
        }
    }

    async fn delete_vm(
        &self,
        request: Request<DeleteVmRequest>,
    ) -> Result<Response<DeleteVmResponse>, Status> {
        let req = request.into_inner();

        match self.vm_manager.delete(&req.vm_id) {
            Ok(_) => {
                log!("grpc-vm", "Deleted VM: {}", req.vm_id);
                Ok(Response::new(DeleteVmResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(DeleteVmResponse {
                success: false,
                error: e,
            })),
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
}

pub fn service(vm_manager: VmManager) -> VmServiceServer<GrpcVmService> {
    VmServiceServer::new(GrpcVmService::new(vm_manager))
}
