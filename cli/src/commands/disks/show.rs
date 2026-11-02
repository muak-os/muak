//! Detail view for a single disk.

use anyhow::Result;
use tonic::transport::Channel;
use tui::console::Console;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use super::{column_refs, fetch, fstype, mount, role, transport, yes_no};
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{DiskInfo, PartitionInfo};

const FIELD_COLUMNS: [&str; 2] = ["FIELD", "VALUE"];
const PARTITION_COLUMNS: [&str; 5] = ["DEVICE", "GPT NAME", "FS", "SIZE", "MOUNT"];

/// Shows the detail view of the disk matching `id`.
pub(crate) async fn detail(
    client: &mut ProvisionServiceClient<Channel>,
    id: &str,
    console: &Console,
) -> Result<()> {
    let disks = fetch(client).await?;
    let disk = find(&disks, id).ok_or_else(|| not_found(&disks, id))?;

    console.table(&fields_table(disk))?;

    if disk.partitions.is_empty() {
        console.line(Role::Muted, "No partitions");
        return Ok(());
    }

    console.line(Role::Muted, "Partitions");
    console.table(&partitions_table(disk))?;

    Ok(())
}

fn find<'a>(disks: &'a [DiskInfo], id: &str) -> Option<&'a DiskInfo> {
    disks.iter().find(|disk| disk.name == id || disk.path == id)
}

fn not_found(disks: &[DiskInfo], id: &str) -> anyhow::Error {
    match disk_of(disks, id) {
        Some(disk) => anyhow::anyhow!(
            "'{}' is a partition of '{}'. Run 'muakctl disks show {}'.",
            id,
            disk.name,
            disk.name
        ),
        None => anyhow::anyhow!("Disk '{id}' not found. Run 'muakctl disks list'."),
    }
}

fn disk_of<'a>(disks: &'a [DiskInfo], id: &str) -> Option<&'a DiskInfo> {
    disks.iter().find(|disk| {
        disk.partitions
            .iter()
            .any(|part| part.path == id || part.name == id)
    })
}

fn fields_table(disk: &DiskInfo) -> Table {
    fields(disk).into_iter().fold(
        Table::new().header(&FIELD_COLUMNS),
        |table, (name, value)| table.row(&[name, &value]),
    )
}

fn fields(disk: &DiskInfo) -> Vec<(&'static str, String)> {
    vec![
        ("PATH", disk.path.clone()),
        ("SIZE", units::bytes(disk.size_bytes)),
        ("MODEL", text(&disk.model)),
        ("SERIAL", text(&disk.serial)),
        ("WWID", text(&disk.wwid)),
        ("TRANSPORT", transport(disk).to_owned()),
        ("ROTATIONAL", yes_no(disk.rotational)),
        ("READ-ONLY", yes_no(disk.read_only)),
        ("BUS PATH", text(&disk.bus_path)),
        ("SUBSYSTEM", text(&disk.subsystem)),
        ("REMOVABLE", yes_no(disk.removable)),
        ("ROLE", role(disk)),
    ]
}

fn partitions_table(disk: &DiskInfo) -> Table {
    disk.partitions
        .iter()
        .fold(Table::new().header(&PARTITION_COLUMNS), |table, part| {
            let cells = partition_cells(part);
            table.row(&column_refs(&cells))
        })
}

fn partition_cells(part: &PartitionInfo) -> Vec<String> {
    vec![
        part.path.clone(),
        text(&part.gpt_name),
        fstype(part),
        units::bytes(part.size_bytes),
        mount(part),
    ]
}

fn text(value: &str) -> String {
    if value.is_empty() {
        "-".to_owned()
    } else {
        value.to_owned()
    }
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
            serial: "Serial".to_owned(),
            wwid: "Wwid".to_owned(),
            bus_path: "/pci0000:00/nvme0".to_owned(),
            subsystem: "nvme".to_owned(),
            rotational: false,
            is_system: true,
            ..DiskInfo::default()
        }
    }

    fn partition() -> PartitionInfo {
        PartitionInfo {
            number: 1,
            size_bytes: GIB,
            name: "sda1".to_owned(),
            path: "/dev/sda1".to_owned(),
            fstype: "ext4".to_owned(),
            gpt_name: "STATE".to_owned(),
            mount_point: "/run/muak".to_owned(),
            in_use: true,
            ..PartitionInfo::default()
        }
    }

    #[test]
    fn find_matches_the_kernel_name() {
        // ARRANGE
        let disks = [disk("sda"), disk("sdb")];

        // ACT
        let found = find(&disks, "sdb");

        // ASSERT
        assert_eq!(found.map(|disk| disk.name.as_str()), Some("sdb"));
    }

    #[test]
    fn find_matches_the_device_path() {
        // ARRANGE
        let disks = [disk("sda"), disk("sdb")];

        // ACT
        let found = find(&disks, "/dev/sdb");

        // ASSERT
        assert_eq!(found.map(|disk| disk.name.as_str()), Some("sdb"));
    }

    #[test]
    fn find_returns_none_for_an_unknown_id() {
        // ARRANGE
        let disks = [disk("sda")];

        // ACT
        let found = find(&disks, "sdz");

        // ASSERT
        assert!(found.is_none());
    }

    #[test]
    fn not_found_points_a_partition_at_its_disk() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.partitions.push(partition());

        // ACT
        let message = not_found(&[disk], "/dev/sda1").to_string();

        // ASSERT
        assert_eq!(
            message,
            "'/dev/sda1' is a partition of 'sda'. Run 'muakctl disks show sda'."
        );
    }

    #[test]
    fn not_found_suggests_the_list_view() {
        // ARRANGE
        let disks = [disk("sda")];

        // ACT
        let message = not_found(&disks, "sdz").to_string();

        // ASSERT
        assert_eq!(message, "Disk 'sdz' not found. Run 'muakctl disks list'.");
    }

    #[test]
    fn fields_follow_the_detail_order() {
        // ARRANGE
        let disk = disk("sda");

        // ACT
        let fields = fields(&disk);

        // ASSERT
        let names: Vec<&str> = fields.iter().map(|field| field.0).collect();
        assert_eq!(
            names,
            [
                "PATH",
                "SIZE",
                "MODEL",
                "SERIAL",
                "WWID",
                "TRANSPORT",
                "ROTATIONAL",
                "READ-ONLY",
                "BUS PATH",
                "SUBSYSTEM",
                "REMOVABLE",
                "ROLE",
            ]
        );
        assert_eq!(fields[0].1, "/dev/sda");
        assert_eq!(fields[1].1, units::bytes(GIB));
        assert_eq!(fields[5].1, "nvme");
        assert_eq!(fields[6].1, "No");
        assert_eq!(fields[7].1, "No");
        assert_eq!(fields[9].1, "nvme");
        assert_eq!(fields[10].1, "No");
        assert_eq!(fields[11].1, "system");
    }

    #[test]
    fn fields_replace_empty_values_with_a_dash() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial.clear();
        disk.wwid.clear();
        disk.bus_path.clear();
        disk.subsystem.clear();
        disk.is_system = false;

        // ACT
        let fields = fields(&disk);

        // ASSERT
        assert!(fields.iter().all(|field| !field.1.is_empty()));
        assert_eq!(fields[3].1, "-");
        assert_eq!(fields[4].1, "-");
        assert_eq!(fields[8].1, "-");
        assert_eq!(fields[9].1, "-");
        assert_eq!(fields[11].1, "-");
    }

    #[test]
    fn partition_cells_report_the_gpt_name_fstype_size_and_mount() {
        // ARRANGE
        let part = partition();

        // ACT
        let cells = partition_cells(&part);

        // ASSERT
        assert_eq!(cells.len(), PARTITION_COLUMNS.len());
        assert_eq!(cells[0], "/dev/sda1");
        assert_eq!(cells[1], "STATE");
        assert_eq!(cells[2], "ext4");
        assert_eq!(cells[3], units::bytes(GIB));
        assert_eq!(cells[4], "/run/muak");
    }

    #[test]
    fn partition_cells_replace_an_empty_gpt_name() {
        // ARRANGE
        let mut part = partition();
        part.gpt_name.clear();

        // ACT
        let cells = partition_cells(&part);

        // ASSERT
        assert_eq!(cells[1], "-");
    }

    #[test]
    fn partition_cells_flag_in_use_without_a_mount_point() {
        // ARRANGE
        let mut part = partition();
        part.mount_point.clear();

        // ACT
        let cells = partition_cells(&part);

        // ASSERT
        assert_eq!(cells[4], "(in use)");
        assert_eq!(cells[4], mount(&part));
    }

    #[test]
    fn partition_cells_dash_an_unused_partition() {
        // ARRANGE
        let mut part = partition();
        part.mount_point.clear();
        part.in_use = false;

        // ACT
        let cells = partition_cells(&part);

        // ASSERT
        assert_eq!(cells[4], "-");
    }

    #[test]
    fn partition_cells_report_unknown_fstype() {
        // ARRANGE
        let mut part = partition();
        part.fstype.clear();

        // ACT
        let cells = partition_cells(&part);

        // ASSERT
        assert_eq!(cells[2], "unknown");
    }
}
