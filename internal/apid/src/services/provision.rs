use std::pin::Pin;
use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("muak.provision.v1");
}

use proto::provision_service_server::{ProvisionService, ProvisionServiceServer};
use proto::{DiskInfo, GetLogsRequest, GetLogsResponse, ListDisksRequest, ListDisksResponse, PartitionInfo};

pub fn service() -> ProvisionServiceServer<ProvisionServiceImpl> {
    ProvisionServiceServer::new(ProvisionServiceImpl)
}

pub struct ProvisionServiceImpl;

#[tonic::async_trait]
impl ProvisionService for ProvisionServiceImpl {
    async fn list_disks(
        &self,
        _request: Request<ListDisksRequest>,
    ) -> Result<Response<ListDisksResponse>, Status> {
        kmsg::info!("ListDisks request");

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
        kmsg::info!("GetLogs request");

        // Stream kernel logs from /dev/kmsg
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

/// List block devices from /sys/block
async fn list_block_devices() -> Result<Vec<DiskInfo>, std::io::Error> {
    let mut disks = Vec::new();

    let mut entries = tokio::fs::read_dir("/sys/block").await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        // Skip loop, ram, and dm devices
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

/// Read disk information from sysfs
async fn read_disk_info(name: &str) -> Result<DiskInfo, std::io::Error> {
    let sys_path = format!("/sys/block/{}", name);
    let dev_path = format!("/dev/{}", name);

    // Read size (in 512-byte sectors)
    let size_sectors: u64 = tokio::fs::read_to_string(format!("{}/size", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);
    let size_bytes = size_sectors * 512;

    // Read model (if available)
    let model = tokio::fs::read_to_string(format!("{}/device/model", sys_path))
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Check if removable
    let removable = tokio::fs::read_to_string(format!("{}/removable", sys_path))
        .await
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    // Check if read-only
    let read_only = tokio::fs::read_to_string(format!("{}/ro", sys_path))
        .await
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    // List partitions
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

/// List partitions for a disk
async fn list_partitions(disk_name: &str) -> Result<Vec<PartitionInfo>, std::io::Error> {
    let mut partitions = Vec::new();

    let sys_path = format!("/sys/block/{}", disk_name);
    let mut entries = tokio::fs::read_dir(&sys_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        // Partitions are directories that start with the disk name
        if name_str.starts_with(disk_name) && name_str != disk_name {
            if let Ok(info) = read_partition_info(disk_name, &name_str).await {
                partitions.push(info);
            }
        }
    }

    // Sort partitions by number
    partitions.sort_by_key(|p| p.number);

    Ok(partitions)
}

/// Read partition information from sysfs
async fn read_partition_info(
    disk_name: &str,
    part_name: &str,
) -> Result<PartitionInfo, std::io::Error> {
    let sys_path = format!("/sys/block/{}/{}", disk_name, part_name);

    // Read partition number
    let number: u32 = tokio::fs::read_to_string(format!("{}/partition", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);

    // Read start sector
    let start_sector: u64 = tokio::fs::read_to_string(format!("{}/start", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);

    // Read size in sectors
    let size_sectors: u64 = tokio::fs::read_to_string(format!("{}/size", sys_path))
        .await?
        .trim()
        .parse()
        .unwrap_or(0);
    let size_bytes = size_sectors * 512;

    // Try to read partition name from GPT
    let name = tokio::fs::read_to_string(format!(
        "/sys/block/{}/{}/partition_name",
        disk_name, part_name
    ))
    .await
    .map(|s| s.trim().to_string())
    .unwrap_or_default();

    let dev_path = format!("/dev/{}", part_name);

    // Try to detect filesystem type (simplified)
    let fstype = String::new(); // Would need blkid or similar

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name,
        path: dev_path,
        fstype,
    })
}

/// Stream kernel logs from /dev/kmsg
async fn stream_kernel_logs(
    tx: tokio::sync::mpsc::Sender<Result<GetLogsResponse, Status>>,
) -> Result<(), std::io::Error> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // Open /dev/kmsg for reading
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open("/dev/kmsg")
        .await?;

    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        // Parse kmsg format: priority,sequence,timestamp,flags;message
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

/// Parse a kmsg line and extract the message
fn parse_kmsg_line(line: &str) -> String {
    // Format: priority,sequence,timestamp,flags;message
    // We want just the message part
    if let Some(idx) = line.find(';') {
        line[idx + 1..].to_string()
    } else {
        line.to_string()
    }
}
