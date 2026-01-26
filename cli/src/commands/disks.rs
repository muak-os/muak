use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{ListDisksRequest, ProvisionServiceClient};
use crate::format::format_size;

/// Lists available disks on the system.
pub async fn handle(client: &mut ProvisionServiceClient<Channel>) -> Result<()> {
    let request = tonic::Request::new(ListDisksRequest {});

    let response = client.list_disks(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        eprintln!("{}", format!("Error listing disks: {}", resp.error).red());
        std::process::exit(1);
    }

    if resp.disks.is_empty() {
        println!("{}", "No disks found".yellow());
        return Ok(());
    }

    println!(
        "{}",
        format!(
            "{:<20}  {:<8}  {:<9}  {:<11} {:<40} {:<3} {:<3} PARTITIONS",
            "DISK", "SIZE", "FS", "POSITION", "MODEL", "RO", "REM"
        )
        .green()
        .bold()
    );

    for disk in resp.disks {
        let size_str = format_size(disk.size_bytes);
        let ro_str = if disk.read_only { "Yes" } else { "No" };
        let rem_str = if disk.removable { "Yes" } else { "No" };
        let part_count = disk.partitions.len();

        println!(
            "{:<20}  {:<8}  {:<9}  {:<11} {:<40} {:<3} {:<3} {}",
            disk.path, size_str, "", "", disk.model, ro_str, rem_str, part_count
        );

        for (idx, part) in disk.partitions.iter().enumerate() {
            let is_last = idx == disk.partitions.len() - 1;
            let prefix = if is_last { "└─" } else { "├─" };
            let part_size_str = format_size(part.size_bytes);
            let fstype_display = if part.fstype.is_empty() {
                "unknown".to_string()
            } else {
                part.fstype.clone()
            };

            println!(
                "  {} {:<15}  {:<8}  {:<9}  {}",
                prefix, part.path, part_size_str, fstype_display, part.start_sector
            );
        }
    }

    Ok(())
}
