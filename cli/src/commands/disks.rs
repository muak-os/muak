use anyhow::Result;
use tonic::transport::Channel;

use crate::client::provision_service::{
    DiskInfo, ListDisksRequest, provision_service_client::ProvisionServiceClient,
};
use crate::format::bytes::format_size;
use crate::ui;

/// Lists available disks on the system.
pub async fn handle(client: &mut ProvisionServiceClient<Channel>) -> Result<()> {
    let request = tonic::Request::new(ListDisksRequest {});

    let response = client.list_disks(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("Error listing disks: {}", resp.error));
    }

    if resp.disks.is_empty() {
        println!("{}", ui::style::warn("No disks found"));
        return Ok(());
    }

    let mut table = ui::table::Table::new().header(&[
        "DISK",
        "SIZE",
        "FS",
        "POSITION",
        "MODEL",
        "RO",
        "REM",
        "PARTITIONS",
    ]);

    for disk in resp.disks {
        let size_str = format_size(disk.size_bytes);
        let ro_str = if disk.read_only { "Yes" } else { "No" };
        let rem_str = if disk.removable { "Yes" } else { "No" };
        let part_count = disk.partitions.len().to_string();

        table = table.row(&[
            &disk.path,
            &size_str,
            "",
            "",
            &disk.model,
            ro_str,
            rem_str,
            &part_count,
        ]);

        table = partition_rows(table, &disk);
    }

    table.print();

    Ok(())
}

/// Appends rows for each partition of a disk to the table.
fn partition_rows(mut table: ui::table::Table, disk: &DiskInfo) -> ui::table::Table {
    for (idx, part) in disk.partitions.iter().enumerate() {
        let is_last = idx == disk.partitions.len().saturating_sub(1);
        let prefix = if is_last {
            "\u{2514}\u{2500}"
        } else {
            "\u{251C}\u{2500}"
        };
        let part_size_str = format_size(part.size_bytes);
        let fstype_display = if part.fstype.is_empty() {
            "unknown".to_owned()
        } else {
            part.fstype.clone()
        };
        let start = part.start_sector.to_string();

        table = table.sub_row(
            prefix,
            &[&part.path, &part_size_str, &fstype_display, &start],
        );
    }
    table
}
