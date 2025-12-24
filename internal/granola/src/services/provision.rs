use std::pin::Pin;
use tonic::{Request, Response, Status};

use super::proto::provision::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::proto::provision::{
    DiskInfo, GetLogsRequest, GetLogsResponse, InstallRequest, InstallResponse, ListDisksRequest,
    ListDisksResponse, PartitionInfo, UpdateRequest, UpdateResponse,
};

use crate::provisioning;

pub fn service() -> ProvisionServiceServer<ProvisionServiceImpl> {
    ProvisionServiceServer::new(ProvisionServiceImpl)
}

pub struct ProvisionServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ProvisionServiceImpl {
    async fn install(
        &self,
        request: Request<InstallRequest>,
    ) -> Result<Response<InstallResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!(
            "Install request: target_disk={}, force={}, version={}",
            req.target_disk,
            req.force,
            req.version
        );

        match provisioning::install(&req.target_disk, req.force, &req.version, &req.extensions)
            .await
        {
            Ok(()) => {
                tokio::spawn(async {
                    kmsg::info!("System will reboot in 3 seconds...");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    kmsg::info!("Rebooting now...");
                    nix::unistd::sync();
                    unsafe {
                        nix::libc::reboot(nix::libc::RB_AUTOBOOT);
                    }
                });

                Ok(Response::new(InstallResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(InstallResponse {
                success: false,
                error: format!("{}", e),
            })),
        }
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let req = request.into_inner();
        kmsg::info!("Update request: version={}", req.version);

        match provisioning::update(&req.version, &req.extensions).await {
            Ok(update_id) => Ok(Response::new(UpdateResponse {
                success: true,
                update_id,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(UpdateResponse {
                success: false,
                update_id: String::new(),
                error: format!("{}", e),
            })),
        }
    }

    async fn list_disks(
        &self,
        _request: Request<ListDisksRequest>,
    ) -> Result<Response<ListDisksResponse>, Status> {
        let disks = list_block_devices()
            .await
            .map_err(|e| Status::internal(format!("Failed to list disks: {}", e)))?;

        Ok(Response::new(ListDisksResponse {
            disks,
            error: String::new(),
        }))
    }

    type GetLogsStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<GetLogsResponse, Status>> + Send>>;

    async fn get_logs(
        &self,
        _request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(e) = stream_kernel_logs(tx).await {
                kmsg::warn!("Error streaming logs: {}", e);
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }
}

async fn list_block_devices() -> Result<Vec<DiskInfo>, std::io::Error> {
    let mut disks = Vec::new();

    let mut entries = tokio::fs::read_dir("/sys/block").await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        if name_str.starts_with("loop")
            || name_str.starts_with("ram")
            || name_str.starts_with("dm-")
        {
            continue;
        }

        if let Ok(info) = read_disk_info(&name_str).await {
            disks.push(info);
        }
    }

    Ok(disks)
}

async fn read_disk_info(name: &str) -> Result<DiskInfo, std::io::Error> {
    let sys_path = format!("/sys/block/{}", name);
    let dev_path = format!("/dev/{}", name);

    let size_sectors: u64 = tokio::fs::read_to_string(format!("{}/size", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);
    let size_bytes = size_sectors * 512;

    let model = tokio::fs::read_to_string(format!("{}/device/model", sys_path))
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let removable = tokio::fs::read_to_string(format!("{}/removable", sys_path))
        .await
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    let read_only = tokio::fs::read_to_string(format!("{}/ro", sys_path))
        .await
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    let partitions = list_partitions(name).await.unwrap_or_default();

    Ok(DiskInfo {
        name: name.to_string(),
        path: dev_path,
        size_bytes,
        model,
        removable,
        read_only,
        partitions,
    })
}

async fn list_partitions(disk_name: &str) -> Result<Vec<PartitionInfo>, std::io::Error> {
    let mut partitions = Vec::new();

    let sys_path = format!("/sys/block/{}", disk_name);
    let mut entries = tokio::fs::read_dir(&sys_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        if name_str.starts_with(disk_name)
            && name_str != disk_name
            && let Ok(info) = read_partition_info(disk_name, &name_str).await
        {
            partitions.push(info);
        }
    }

    partitions.sort_by_key(|p| p.number);

    Ok(partitions)
}

async fn read_partition_info(
    disk_name: &str,
    part_name: &str,
) -> Result<PartitionInfo, std::io::Error> {
    let sys_path = format!("/sys/block/{}/{}", disk_name, part_name);

    let number: u32 = tokio::fs::read_to_string(format!("{}/partition", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);

    let start_sector: u64 = tokio::fs::read_to_string(format!("{}/start", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);

    let size_sectors: u64 = tokio::fs::read_to_string(format!("{}/size", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);
    let size_bytes = size_sectors * 512;

    let name = tokio::fs::read_to_string(format!(
        "/sys/block/{}/{}/partition_name",
        disk_name, part_name
    ))
    .await
    .map(|s| s.trim().to_string())
    .unwrap_or_default();

    let dev_path = format!("/dev/{}", part_name);

    // TODO: properly detect filesystem type
    let fstype = String::new();

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name,
        path: dev_path,
        fstype,
    })
}

async fn stream_kernel_logs(
    tx: tokio::sync::mpsc::Sender<Result<GetLogsResponse, Status>>,
) -> Result<(), std::io::Error> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open("/dev/kmsg")
        .await?;

    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let message = parse_kmsg_line(&line);

        if tx
            .send(Ok(GetLogsResponse {
                line: message,
                error: String::new(),
            }))
            .await
            .is_err()
        {
            break;
        }
    }

    Ok(())
}

fn parse_kmsg_line(line: &str) -> String {
    if let Some(idx) = line.find(';') {
        line[idx + 1..].to_string()
    } else {
        line.to_string()
    }
}
