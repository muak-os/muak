use anyhow::Result;
use tonic::transport::Channel;
use tui::console::Console;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{DiskInfo, ListDisksRequest, PartitionInfo};

const COLUMNS: [&str; 10] = [
    "ID",
    "SIZE",
    "TRANSPORT",
    "ROT",
    "SERIAL",
    "WWID",
    "MODEL",
    "RO",
    "PARTS",
    "ROLE",
];
const WIDE_COLUMNS: [&str; 3] = ["BUS PATH", "SUBSYSTEM", "REM"];
const PARTITION_COLUMNS: [&str; 3] = ["FS", "NAME", "MOUNT"];

/// Disk subcommands.
#[derive(clap::Subcommand, Clone)]
pub enum Action {
    /// Lists available disks on the system.
    List,
}

/// Handles the disk command group, defaulting to the list view.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    action: Option<Action>,
    wide: bool,
    console: &Console,
) -> Result<()> {
    let request = tonic::Request::new(ListDisksRequest {});
    let resp = client.list_disks(request).await?.into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("Error listing disks: {}", resp.error));
    }

    match dispatch(action) {
        Action::List => list(&resp.disks, wide, console),
    }
}

fn dispatch(action: Option<Action>) -> Action {
    action.unwrap_or(Action::List)
}

fn list(disks: &[DiskInfo], wide: bool, console: &Console) -> Result<()> {
    if disks.is_empty() {
        console.line(Role::Warn, "No disks found");
        return Ok(());
    }

    let mut table = Table::new().header(&headers(wide));
    for disk in disks {
        table = render(table, disk, wide);
    }

    console.table(&table)?;

    Ok(())
}

fn render(table: Table, disk: &DiskInfo, wide: bool) -> Table {
    let row = row_cells(disk, wide);
    let table = table.row(&column_refs(&row));

    partition_rows(table, disk, wide)
}

fn row_cells(disk: &DiskInfo, wide: bool) -> Vec<String> {
    let mut row = vec![
        disk.name.clone(),
        units::bytes(disk.size_bytes),
        transport(disk).to_owned(),
        yes_no(disk.rotational),
        disk.serial.clone(),
        disk.wwid.clone(),
        disk.model.clone(),
        yes_no(disk.read_only),
        disk.partitions.len().to_string(),
        role(disk),
    ];

    if wide {
        row.push(disk.bus_path.clone());
        row.push(disk.subsystem.clone());
        row.push(yes_no(disk.removable));
    }

    row.resize(
        row.len().saturating_add(PARTITION_COLUMNS.len()),
        String::new(),
    );

    row
}

fn transport(disk: &DiskInfo) -> &str {
    if disk.subsystem == "nvme" || disk.bus_path.contains("/nvme") {
        "nvme"
    } else if disk.bus_path.contains("/usb") {
        "usb"
    } else if disk.bus_path.contains("/ata") {
        "sata"
    } else if disk.subsystem == "virtio" {
        "virtio"
    } else if !disk.subsystem.is_empty() {
        disk.subsystem.as_str()
    } else {
        "-"
    }
}

fn role(disk: &DiskInfo) -> String {
    match (disk.is_system, disk.is_data) {
        (true, true) => "system,data".to_owned(),
        (true, false) => "system".to_owned(),
        (false, true) => "data".to_owned(),
        (false, false) => String::new(),
    }
}

fn yes_no(value: bool) -> String {
    if value { "Yes" } else { "No" }.to_owned()
}

fn partition_rows(mut table: Table, disk: &DiskInfo, wide: bool) -> Table {
    let last = disk.partitions.len().saturating_sub(1);

    for (index, part) in disk.partitions.iter().enumerate() {
        let prefix = if index == last {
            "\u{2514}\u{2500}"
        } else {
            "\u{251C}\u{2500}"
        };
        let cells = partition_cells(part, wide);

        table = table.sub_row(prefix, &column_refs(&cells));
    }

    table
}

fn partition_cells(part: &PartitionInfo, wide: bool) -> Vec<String> {
    let mut row = vec![part.path.clone(), units::bytes(part.size_bytes)];

    row.resize(COLUMNS.len(), String::new());
    if wide {
        row.resize(
            COLUMNS.len().saturating_add(WIDE_COLUMNS.len()),
            String::new(),
        );
    }

    row.push(fstype(part));
    row.push(part.gpt_name.clone());
    row.push(mount(part));

    row
}

fn fstype(part: &PartitionInfo) -> String {
    if part.fstype.is_empty() {
        "unknown".to_owned()
    } else {
        part.fstype.clone()
    }
}

fn mount(part: &PartitionInfo) -> String {
    if !part.mount_point.is_empty() {
        part.mount_point.clone()
    } else if part.in_use {
        "(in use)".to_owned()
    } else {
        String::new()
    }
}

fn column_refs(cells: &[String]) -> Vec<&str> {
    cells.iter().map(String::as_str).collect()
}

fn headers(wide: bool) -> Vec<&'static str> {
    let mut names = COLUMNS.to_vec();
    if wide {
        names.extend_from_slice(&WIDE_COLUMNS);
    }
    names.extend_from_slice(&PARTITION_COLUMNS);

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn disk(name: &str) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            size_bytes: GIB,
            model: "Model".to_owned(),
            removable: false,
            read_only: false,
            partitions: Vec::new(),
            serial: "Serial".to_owned(),
            wwid: "Wwid".to_owned(),
            bus_path: String::new(),
            subsystem: String::new(),
            rotational: true,
            cdrom: false,
            symlinks: Vec::new(),
            is_system: false,
            is_data: false,
        }
    }

    fn partition() -> PartitionInfo {
        PartitionInfo {
            number: 1,
            start_sector: 2048,
            size_bytes: GIB,
            name: "sda1".to_owned(),
            path: "/dev/sda1".to_owned(),
            fstype: "ext4".to_owned(),
            gpt_name: "STATE".to_owned(),
            mount_point: "/run/muak".to_owned(),
            in_use: true,
        }
    }

    #[test]
    fn row_cells_follow_the_column_order() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.partitions.push(partition());

        // ACT
        let cells = row_cells(&disk, false);

        // ASSERT
        assert_eq!(cells.len(), headers(false).len());
        assert_eq!(cells[0], "sda");
        assert_eq!(cells[1], units::bytes(GIB));
        assert_eq!(cells[3], "Yes");
        assert_eq!(cells[4], "Serial");
        assert_eq!(cells[5], "Wwid");
        assert_eq!(cells[6], "Model");
        assert_eq!(cells[7], "No");
        assert_eq!(cells[8], "1");
        assert!(cells[COLUMNS.len()..].iter().all(String::is_empty));
    }

    #[test]
    fn wide_row_cells_append_bus_path_subsystem_and_removable() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.bus_path = "/pci0000:00/ata1".to_owned();
        disk.subsystem = "scsi".to_owned();
        disk.removable = true;

        // ACT
        let cells = row_cells(&disk, true);

        // ASSERT
        assert_eq!(cells.len(), headers(true).len());
        assert_eq!(cells[COLUMNS.len()], "/pci0000:00/ata1");
        assert_eq!(cells[COLUMNS.len() + 1], "scsi");
        assert_eq!(cells[COLUMNS.len() + 2], "Yes");
    }

    #[test]
    fn transport_prefers_nvme_subsystem() {
        // ARRANGE
        let mut disk = disk("nvme0n1");
        disk.subsystem = "nvme".to_owned();

        // ACT
        let transport = transport(&disk);

        // ASSERT
        assert_eq!(transport, "nvme");
    }

    #[test]
    fn transport_detects_usb_from_bus_path() {
        // ARRANGE
        let mut disk = disk("sdb");
        disk.bus_path = "/pci0000:00/usb1/1-1".to_owned();

        // ACT
        let transport = transport(&disk);

        // ASSERT
        assert_eq!(transport, "usb");
    }

    #[test]
    fn transport_maps_ata_bus_path_to_sata() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.bus_path = "/pci0000:00/ata1/host0".to_owned();

        // ACT
        let transport = transport(&disk);

        // ASSERT
        assert_eq!(transport, "sata");
    }

    #[test]
    fn transport_falls_back_to_subsystem_then_unknown() {
        // ARRANGE
        let mut scsi = disk("sda");
        scsi.subsystem = "scsi".to_owned();
        let empty = disk("sdz");

        // ACT
        let scsi_transport = transport(&scsi);
        let empty_transport = transport(&empty);

        // ASSERT
        assert_eq!(scsi_transport, "scsi");
        assert_eq!(empty_transport, "-");
    }

    #[test]
    fn role_combines_system_and_data_flags() {
        // ARRANGE
        let mut system = disk("sda");
        system.is_system = true;
        let mut data = disk("sdb");
        data.is_data = true;
        let mut both = disk("sdc");
        both.is_system = true;
        both.is_data = true;
        let none = disk("sdd");

        // ACT
        let roles = [role(&system), role(&data), role(&both), role(&none)];

        // ASSERT
        assert_eq!(roles, ["system", "data", "system,data", ""]);
    }

    #[test]
    fn partition_cells_fill_the_partition_headers() {
        // ARRANGE
        let part = partition();
        let base = headers(false).len() - PARTITION_COLUMNS.len();

        // ACT
        let cells = partition_cells(&part, false);

        // ASSERT
        assert_eq!(cells.len(), headers(false).len());
        assert_eq!(cells[0], "/dev/sda1");
        assert_eq!(cells[1], units::bytes(GIB));
        assert_eq!(headers(false)[base], "FS");
        assert_eq!(cells[base], "ext4");
        assert_eq!(cells[base + 1], "STATE");
        assert_eq!(cells[base + 2], "/run/muak");
    }

    #[test]
    fn wide_partition_cells_stay_aligned_with_the_headers() {
        // ARRANGE
        let part = partition();
        let base = headers(true).len() - PARTITION_COLUMNS.len();

        // ACT
        let cells = partition_cells(&part, true);

        // ASSERT
        assert_eq!(cells.len(), headers(true).len());
        assert_eq!(cells[base], "ext4");
        assert_eq!(cells[base + 1], "STATE");
        assert_eq!(cells[base + 2], "/run/muak");
    }

    #[test]
    fn partition_cells_report_unknown_fstype() {
        // ARRANGE
        let mut part = partition();
        part.fstype.clear();
        let base = headers(false).len() - PARTITION_COLUMNS.len();

        // ACT
        let cells = partition_cells(&part, false);

        // ASSERT
        assert_eq!(cells[base], "unknown");
    }

    #[test]
    fn partition_cells_flag_in_use_without_a_mount_point() {
        // ARRANGE
        let mut part = partition();
        part.mount_point.clear();
        let base = headers(false).len() - PARTITION_COLUMNS.len();

        // ACT
        let cells = partition_cells(&part, false);

        // ASSERT
        assert_eq!(cells[base + 2], "(in use)");
    }

    #[test]
    fn headers_extend_the_default_headers() {
        // ARRANGE
        let default: Vec<&str> = COLUMNS
            .iter()
            .chain(PARTITION_COLUMNS.iter())
            .copied()
            .collect();
        let wide: Vec<&str> = COLUMNS
            .iter()
            .chain(WIDE_COLUMNS.iter())
            .chain(PARTITION_COLUMNS.iter())
            .copied()
            .collect();

        // ACT
        let actual_default = headers(false);
        let actual_wide = headers(true);

        // ASSERT
        assert_eq!(actual_default, default);
        assert_eq!(actual_wide, wide);
    }

    #[test]
    fn dispatch_defaults_to_the_list_view() {
        // ARRANGE
        let actions = [None, Some(Action::List)];

        // ACT
        let dispatched = actions.map(dispatch);

        // ASSERT
        assert!(
            dispatched
                .iter()
                .all(|action| matches!(action, Action::List))
        );
    }
}
