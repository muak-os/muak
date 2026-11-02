//! Rendering of the resolved wipe target and its safety checks.

use std::io;

use tui::console::Console;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use super::{column_refs, field_table, mount, text};
use crate::client::provision_service::{DiskInfo, PartitionInfo, PreflightCheck, WipeDiskResponse};

const PARTITION_COLUMNS: [&str; 3] = ["GPT NAME", "SIZE", "MOUNT"];
const CHECK_COLUMNS: [&str; 3] = ["CHECK", "RESULT", "DETAIL"];

/// Prints the resolved disk identity, its partitions and every check.
pub(crate) fn print(response: &WipeDiskResponse, console: &Console) -> io::Result<()> {
    if let Some(disk) = response.disk.as_ref() {
        console.table(&fields_table(disk))?;
        if disk.partitions.is_empty() {
            console.line(Role::Muted, "No partitions");
        } else {
            console.table(&partitions_table(disk))?;
        }
    }

    console.table(&checks_table(&response.checks))
}

fn fields_table(disk: &DiskInfo) -> Table {
    field_table(identity_rows(disk))
}

fn partitions_table(disk: &DiskInfo) -> Table {
    partition_rows(disk)
        .into_iter()
        .fold(Table::new().header(&PARTITION_COLUMNS), |table, row| {
            table.row(&column_refs(&row))
        })
}

fn checks_table(checks: &[PreflightCheck]) -> Table {
    checks
        .iter()
        .fold(Table::new().header(&CHECK_COLUMNS), |table, check| {
            let row = check_row(check);
            table.row(&column_refs(&row))
        })
}

fn identity_rows(disk: &DiskInfo) -> Vec<(&'static str, String)> {
    vec![
        ("PATH", disk.path.clone()),
        ("SIZE", units::bytes(disk.size_bytes)),
        ("MODEL", text(&disk.model)),
        ("SERIAL", text(&disk.serial)),
        ("WWID", text(&disk.wwid)),
    ]
}

fn partition_rows(disk: &DiskInfo) -> Vec<Vec<String>> {
    disk.partitions.iter().map(partition_row).collect()
}

fn partition_row(part: &PartitionInfo) -> Vec<String> {
    vec![
        text(&part.gpt_name),
        units::bytes(part.size_bytes),
        mount(part),
    ]
}

fn check_row(check: &PreflightCheck) -> Vec<String> {
    vec![
        check.name.clone(),
        result(check.passed),
        check.detail.clone(),
    ]
}

fn result(passed: bool) -> String {
    if passed {
        "pass".to_owned()
    } else {
        "fail".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk() -> DiskInfo {
        DiskInfo {
            name: "nvme0n1".to_owned(),
            path: "/dev/nvme0n1".to_owned(),
            size_bytes: 1024,
            model: "Model".to_owned(),
            serial: "SN123".to_owned(),
            wwid: "naa.5000".to_owned(),
            ..DiskInfo::default()
        }
    }

    fn partition() -> PartitionInfo {
        PartitionInfo {
            number: 1,
            size_bytes: 512,
            name: "nvme0n1p1".to_owned(),
            path: "/dev/nvme0n1p1".to_owned(),
            gpt_name: "STATE".to_owned(),
            mount_point: "/run/muak".to_owned(),
            in_use: true,
            ..PartitionInfo::default()
        }
    }

    #[test]
    fn identity_rows_show_the_resolved_identity() {
        // ARRANGE
        let disk = disk();

        // ACT
        let rows = identity_rows(&disk);

        // ASSERT
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].0, "PATH");
        assert_eq!(rows[0].1, "/dev/nvme0n1");
        assert_eq!(rows[1].1, units::bytes(1024));
        assert_eq!(rows[2].1, "Model");
        assert_eq!(rows[3].1, "SN123");
        assert_eq!(rows[4].1, "naa.5000");
    }

    #[test]
    fn partition_rows_show_gpt_name_size_and_mount() {
        // ARRANGE
        let mut disk = disk();
        disk.partitions.push(partition());

        // ACT
        let rows = partition_rows(&disk);

        // ASSERT
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], "STATE");
        assert_eq!(rows[0][1], units::bytes(512));
        assert_eq!(rows[0][2], "/run/muak");
    }

    #[test]
    fn check_rows_report_pass_fail_and_detail() {
        // ARRANGE
        let checks = [
            PreflightCheck {
                name: "read_only".to_owned(),
                passed: true,
                detail: "writable".to_owned(),
            },
            PreflightCheck {
                name: "mounted_partitions".to_owned(),
                passed: false,
                detail: "mounted".to_owned(),
            },
        ];

        // ACT
        let rows: Vec<Vec<String>> = checks.iter().map(check_row).collect();

        // ASSERT
        assert_eq!(rows[0], ["read_only", "pass", "writable"]);
        assert_eq!(rows[1], ["mounted_partitions", "fail", "mounted"]);
    }
}
