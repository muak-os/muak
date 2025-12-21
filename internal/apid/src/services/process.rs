use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("muak.process.v1");
}

use proto::process_service_server::{ProcessService, ProcessServiceServer};
use proto::{ListProcessesRequest, ListProcessesResponse, ProcessInfo};

pub fn service() -> ProcessServiceServer<ProcessServiceImpl> {
    ProcessServiceServer::new(ProcessServiceImpl)
}

pub struct ProcessServiceImpl;

#[tonic::async_trait]
impl ProcessService for ProcessServiceImpl {
    async fn list_processes(
        &self,
        _request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        let mut processes = Vec::new();

        let proc_dir = match tokio::fs::read_dir("/proc").await {
            Ok(dir) => dir,
            Err(e) => {
                return Err(Status::internal(format!("Failed to read /proc: {}", e)));
            }
        };

        let mut entries = proc_dir;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if let Ok(pid) = name_str.parse::<i32>()
                && let Ok(info) = read_process_info(pid).await
            {
                processes.push(info);
            }
        }

        Ok(Response::new(ListProcessesResponse { processes }))
    }
}

async fn read_process_info(pid: i32) -> Result<ProcessInfo, std::io::Error> {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let cmdline = tokio::fs::read_to_string(&cmdline_path).await?;

    let parts: Vec<&str> = cmdline.trim_end_matches('\0').split('\0').collect();
    let command = parts.first().map(|s| s.to_string()).unwrap_or_default();
    let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();

    let stat_path = format!("/proc/{}/stat", pid);
    let stat = tokio::fs::read_to_string(&stat_path).await?;
    let status = parse_process_status(&stat);

    let started_at = 0i64; // Would need to parse /proc/[pid]/stat properly for accurate time

    Ok(ProcessInfo {
        pid,
        command,
        args,
        status,
        started_at,
    })
}

fn parse_process_status(stat: &str) -> String {
    if let Some(close_paren) = stat.rfind(')')
        && let Some(state_char) = stat.chars().nth(close_paren + 2)
    {
        return match state_char {
            'R' => "running",
            'S' => "sleeping",
            'D' => "disk_sleep",
            'Z' => "zombie",
            'T' => "stopped",
            't' => "tracing_stop",
            'X' | 'x' => "dead",
            'K' => "wakekill",
            'W' => "waking",
            'P' => "parked",
            'I' => "idle",
            _ => "unknown",
        }
        .to_string();
    }
    "unknown".to_string()
}
