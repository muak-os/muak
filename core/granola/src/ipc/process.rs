use tonic::{Request, Response, Status};

use super::proto::process::process_service_server::{ProcessService, ProcessServiceServer};
use super::proto::process::{ListProcessesRequest, ListProcessesResponse, ProcessInfo};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_state() {
        // ARRANGE
        let stat = "123 (my-proc) R 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "running");
    }

    #[test]
    fn sleeping_state() {
        // ARRANGE
        let stat = "123 (my-proc) S 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "sleeping");
    }

    #[test]
    fn disk_sleep_state() {
        // ARRANGE
        let stat = "123 (my-proc) D 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "disk_sleep");
    }

    #[test]
    fn zombie_state() {
        // ARRANGE
        let stat = "123 (my-proc) Z 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "zombie");
    }

    #[test]
    fn stopped_state() {
        // ARRANGE
        let stat = "123 (my-proc) T 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "stopped");
    }

    #[test]
    fn tracing_stop_state() {
        // ARRANGE
        let stat = "123 (my-proc) t 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "tracing_stop");
    }

    #[test]
    fn dead_state_uppercase() {
        // ARRANGE
        let stat = "123 (my-proc) X 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "dead");
    }

    #[test]
    fn dead_state_lowercase() {
        // ARRANGE
        let stat = "123 (my-proc) x 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "dead");
    }

    #[test]
    fn wakekill_state() {
        // ARRANGE
        let stat = "123 (my-proc) K 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "wakekill");
    }

    #[test]
    fn waking_state() {
        // ARRANGE
        let stat = "123 (my-proc) W 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "waking");
    }

    #[test]
    fn parked_state() {
        // ARRANGE
        let stat = "123 (my-proc) P 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "parked");
    }

    #[test]
    fn idle_state() {
        // ARRANGE
        let stat = "123 (my-proc) I 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "idle");
    }

    #[test]
    fn unknown_state_unrecognised_char() {
        // ARRANGE
        let stat = "123 (my-proc) Q 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "unknown");
    }

    #[test]
    fn no_closing_paren_returns_unknown() {
        // ARRANGE
        let stat = "123 my-proc S 1 123";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "unknown");
    }

    #[test]
    fn closing_paren_at_end_returns_unknown() {
        // ARRANGE
        let stat = "123 (my-proc)";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "unknown");
    }

    #[test]
    fn process_name_containing_parens_uses_last_close_paren() {
        // ARRANGE
        let stat = "123 (my(proc)) S 1 123 123 0 -1 4194560";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "sleeping");
    }

    #[test]
    fn empty_stat_returns_unknown() {
        // ARRANGE
        let stat = "";

        // ACT
        let result = parse_process_status(stat);

        // ASSERT
        assert_eq!(result, "unknown");
    }
}
