use crate::{disk, log, provisioning};
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("muak.maintenance.v1");
}

use proto::{
    DiskInfo as ProtoDiskInfo, GetLogsRequest, GetLogsResponse, InstallRequest, InstallResponse,
    ListDisksRequest, ListDisksResponse, PartitionInfo as ProtoPartitionInfo, UpdateRequest,
    UpdateResponse, maintenance_service_server::MaintenanceService,
};

pub struct MaintenanceServiceImpl;

#[tonic::async_trait]
impl MaintenanceService for MaintenanceServiceImpl {
    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<InstallResponse>, Status> {
        let req = request.into_inner();

        log!(
            "maintenance",
            "Install request: target={}, force={}",
            req.target_disk,
            req.force
        );

        let version = if req.version.is_empty() {
            "latest".to_string()
        } else {
            req.version.clone()
        };

        let extensions = req.extensions.clone();
        let target_disk = req.target_disk.clone();
        let force = req.force;

        let result = tokio::task::spawn_blocking(move || {
            provisioning::install(&target_disk, force, &version, &extensions)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let response = Response::new(InstallResponse {
                    success: true,
                    error: String::new(),
                });

                tokio::spawn(async {
                    log!(
                        "maintenance",
                        "Installation successful, rebooting in 3 seconds..."
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
                    if nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT).is_err()
                    {
                        log!("maintenance", "Failed to reboot");
                    }
                });

                Ok(response)
            }
            Ok(Err(e)) => {
                log!("maintenance", "Installation failed: {}", e);
                Ok(Response::new(InstallResponse {
                    success: false,
                    error: format!("{}", e),
                }))
            }
            Err(e) => {
                log!("maintenance", "Installation task failed: {}", e);
                Ok(Response::new(InstallResponse {
                    success: false,
                    error: format!("{}", e),
                }))
            }
        }
    }

    async fn list_disks(
        &self,
        _request: Request<ListDisksRequest>,
    ) -> Result<Response<ListDisksResponse>, Status> {
        let result = tokio::task::spawn_blocking(disk::list_disks).await;

        match result {
            Ok(Ok(disks)) => {
                let proto_disks: Vec<ProtoDiskInfo> = disks
                    .into_iter()
                    .map(|d| ProtoDiskInfo {
                        name: d.name,
                        path: d.path,
                        size_bytes: d.size_bytes,
                        model: d.model,
                        removable: d.removable,
                        read_only: d.read_only,
                        partitions: d
                            .partitions
                            .into_iter()
                            .map(|p| ProtoPartitionInfo {
                                number: p.number,
                                start_sector: p.start_sector,
                                size_bytes: p.size_bytes,
                                name: p.name,
                                path: p.path,
                                fstype: p.fstype,
                            })
                            .collect(),
                    })
                    .collect();

                Ok(Response::new(ListDisksResponse {
                    disks: proto_disks,
                    error: String::new(),
                }))
            }
            Ok(Err(e)) => {
                log!("maintenance", "List disks failed: {}", e);
                Ok(Response::new(ListDisksResponse {
                    disks: Vec::new(),
                    error: format!("{}", e),
                }))
            }
            Err(e) => {
                log!("maintenance", "List disks task failed: {}", e);
                Ok(Response::new(ListDisksResponse {
                    disks: Vec::new(),
                    error: format!("{}", e),
                }))
            }
        }
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let req = request.into_inner();

        let version = if req.version.is_empty() {
            "latest".to_string()
        } else {
            req.version.clone()
        };

        let extensions = req.extensions.clone();

        let result = provisioning::update(&version, &extensions);

        match result {
            Ok(update_result) => Ok(Response::new(UpdateResponse {
                success: true,
                error: String::new(),
                update_id: update_result.update_id,
            })),
            Err(e) => {
                log!("maintenance", "Update failed: {}", e);
                Ok(Response::new(UpdateResponse {
                    success: false,
                    error: format!("{}", e),
                    update_id: String::new(),
                }))
            }
        }
    }

    type GetLogsStream =
        Pin<Box<dyn Stream<Item = Result<GetLogsResponse, Status>> + Send + 'static>>;

    async fn get_logs(
        &self,
        _request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            match tokio::fs::File::open("/dev/kmsg").await {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();

                    loop {
                        match lines.next_line().await {
                            Ok(Some(line)) => {
                                if let Some(formatted) = parse_kmsg_line(&line) {
                                    let response = GetLogsResponse {
                                        line: formatted,
                                        error: String::new(),
                                    };
                                    if tx.send(Ok(response)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(None) => {
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(Ok(GetLogsResponse {
                                        line: String::new(),
                                        error: format!("Read error: {}", e),
                                    }))
                                    .await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(GetLogsResponse {
                            line: String::new(),
                            error: format!("Failed to open /dev/kmsg: {}", e),
                        }))
                        .await;
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }
}

fn parse_kmsg_line(line: &str) -> Option<String> {
    if line.starts_with(' ') {
        return None;
    }

    if let Some(semicolon_pos) = line.find(';') {
        let metadata = &line[..semicolon_pos];
        let message = &line[semicolon_pos + 1..];

        let parts: Vec<&str> = metadata.split(',').collect();
        if parts.len() >= 3
            && let Ok(timestamp_us) = parts[2].parse::<u64>()
        {
            let timestamp_secs = timestamp_us as f64 / 1_000_000.0;
            return Some(format!("[{:>12.6}] {}", timestamp_secs, message));
        }

        Some(message.to_string())
    } else {
        Some(line.to_string())
    }
}
