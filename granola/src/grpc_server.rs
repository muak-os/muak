use crate::process::ProcessManager;
use tonic::{transport::Server, Request, Response, Status};

pub mod process_service {
    tonic::include_proto!("muak.process.v1");
}

use process_service::process_service_server::{ProcessService, ProcessServiceServer};
use process_service::{
    ListProcessesRequest, ListProcessesResponse, ProcessInfo, StartProcessRequest,
    StartProcessResponse, StopProcessRequest, StopProcessResponse,
};

pub struct GrpcProcessService {
    process_manager: ProcessManager,
}

impl GrpcProcessService {
    pub fn new(process_manager: ProcessManager) -> Self {
        Self { process_manager }
    }
}

#[tonic::async_trait]
impl ProcessService for GrpcProcessService {
    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<StartProcessResponse>, Status> {
        let req = request.into_inner();

        match self.process_manager.spawn(req.command, req.args, req.env) {
            Ok(pid) => Ok(Response::new(StartProcessResponse {
                pid,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(StartProcessResponse { pid: -1, error: e })),
        }
    }

    async fn stop_process(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        let req = request.into_inner();
        let signal = if req.signal == 0 { 15 } else { req.signal };

        match self.process_manager.stop(req.pid, signal) {
            Ok(_) => Ok(Response::new(StopProcessResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(StopProcessResponse {
                success: false,
                error: e,
            })),
        }
    }

    async fn list_processes(
        &self,
        _request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        let processes = self.process_manager.list();

        let process_infos: Vec<ProcessInfo> = processes
            .into_iter()
            .map(|p| ProcessInfo {
                pid: p.pid,
                command: p.command,
                args: p.args,
                status: p.status.to_string(),
                started_at: p.started_at,
            })
            .collect();

        Ok(Response::new(ListProcessesResponse {
            processes: process_infos,
        }))
    }
}

pub async fn run_grpc_server(
    process_manager: ProcessManager,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051".parse()?;

    let service = GrpcProcessService::new(process_manager);

    Server::builder()
        .add_service(ProcessServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
