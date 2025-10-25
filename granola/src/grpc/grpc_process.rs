use crate::ipc::{IpcClient, IpcMessage, IpcResponse};
use crate::log;
use crate::process::Process;
use tonic::{Request, Response, Status};

pub mod process_service {
    tonic::include_proto!("muak.process.v1");
}

use process_service::process_service_server::{ProcessService, ProcessServiceServer};
use process_service::{
    ListProcessesRequest, ListProcessesResponse, ProcessInfo, StartProcessRequest,
    StartProcessResponse, StopProcessRequest, StopProcessResponse,
};

pub struct GrpcProcessService {
    ipc_client: std::sync::Mutex<IpcClient>,
}

impl GrpcProcessService {
    pub fn new() -> Self {
        let mut client = IpcClient::new();
        if let Err(e) = client.connect() {
            log!("grpc", "Failed to connect to IPC: {}", e);
        }
        Self {
            ipc_client: std::sync::Mutex::new(client),
        }
    }

    fn send_ipc_message(&self, message: IpcMessage) -> Result<IpcResponse, String> {
        let mut client = self.ipc_client.lock().unwrap();
        client.send_message(&message)
    }
}

#[tonic::async_trait]
impl ProcessService for GrpcProcessService {
    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<StartProcessResponse>, Status> {
        let req = request.into_inner();

        let message = IpcMessage::StartProcess {
            command: req.command,
            args: req.args,
            env: req.env,
        };

        match self.send_ipc_message(message) {
            Ok(IpcResponse::ProcessStarted { pid }) => Ok(Response::new(StartProcessResponse {
                pid,
                error: String::new(),
            })),
            Ok(IpcResponse::Error(e)) => {
                Ok(Response::new(StartProcessResponse { pid: -1, error: e }))
            }
            Err(e) => Ok(Response::new(StartProcessResponse { pid: -1, error: e })),
            _ => Ok(Response::new(StartProcessResponse {
                pid: -1,
                error: "Unexpected response".to_string(),
            })),
        }
    }

    async fn stop_process(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        let req = request.into_inner();
        let signal = if req.signal == 0 { 15 } else { req.signal };

        let message = IpcMessage::StopProcess {
            pid: req.pid,
            signal,
        };

        match self.send_ipc_message(message) {
            Ok(IpcResponse::Ok) => Ok(Response::new(StopProcessResponse {
                success: true,
                error: String::new(),
            })),
            Ok(IpcResponse::Error(e)) => Ok(Response::new(StopProcessResponse {
                success: false,
                error: e,
            })),
            Err(e) => Ok(Response::new(StopProcessResponse {
                success: false,
                error: e,
            })),
            _ => Ok(Response::new(StopProcessResponse {
                success: false,
                error: "Unexpected response".to_string(),
            })),
        }
    }

    async fn list_processes(
        &self,
        _request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        let message = IpcMessage::ListProcesses;

        match self.send_ipc_message(message) {
            Ok(IpcResponse::ProcessList(data)) => {
                let processes: Vec<Process> = bincode::deserialize(&data)
                    .map_err(|e| Status::internal(format!("Deserialization error: {}", e)))?;

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
            Ok(IpcResponse::Error(e)) => Err(Status::internal(e)),
            Err(e) => Err(Status::internal(e)),
            _ => Err(Status::internal("Unexpected response")),
        }
    }
}

pub fn service() -> ProcessServiceServer<GrpcProcessService> {
    ProcessServiceServer::new(GrpcProcessService::new())
}
