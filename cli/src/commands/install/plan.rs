//! Rendering of the resolved install plan.

use std::io;

use tui::console::Console;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use crate::client::provision_service::{DiskInfo, InstallPlan, PlannedPartition, PreflightCheck};

const DISK_COLUMNS: [&str; 7] = ["ROLE", "DEVICE", "SIZE", "SERIAL", "WWID", "MODEL", "WIPE"];
const PARTITION_COLUMNS: [&str; 4] = ["DISK", "ROLE", "NAME", "SIZE"];
const CHECK_COLUMNS: [&str; 3] = ["CHECK", "RESULT", "DETAIL"];

/// Prints the resolved disks, the data that will be destroyed and the checks.
pub(crate) fn print(plan: &InstallPlan, console: &Console) -> io::Result<()> {
    console.table(&table(&DISK_COLUMNS, disk_rows(plan)))?;

    if plan.shared_data {
        console.line(Role::Muted, "DATA shares the system disk");
    }

    if plan.wipe {
        for device in wiped(plan) {
            console.line(
                Role::ErrorText,
                &format!("Wiping {device} destroys every partition on it"),
            );
        }
    }

    console.table(&table(&PARTITION_COLUMNS, partition_rows(plan)))?;
    console.table(&table(&CHECK_COLUMNS, check_rows(plan)))
}

/// Returns true when at least one preflight check refuses the plan.
pub(crate) fn refused(plan: &InstallPlan) -> bool {
    plan.checks.iter().any(|check| !check.passed)
}

/// Returns the device path of the system disk, for operator messages.
pub(crate) fn system_path(plan: &InstallPlan) -> &str {
    system_disk(plan).map_or("unknown disk", |disk| &disk.path)
}

fn system_disk(plan: &InstallPlan) -> Option<&DiskInfo> {
    plan.system_disk.as_ref()
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Table {
    let mut table = Table::new().header(headers);

    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        table = table.row(&cells);
    }

    table
}

fn disk_rows(plan: &InstallPlan) -> Vec<Vec<String>> {
    let mut rows = vec![disk_row(plan.system_disk.as_ref(), "system", plan)];
    if plan.shared_data {
        return rows;
    }
    rows.push(disk_row(plan.data_disk.as_ref(), "data", plan));

    rows
}

fn disk_row(disk: Option<&DiskInfo>, role: &str, plan: &InstallPlan) -> Vec<String> {
    let Some(disk) = disk else {
        return vec![
            role.to_owned(),
            "not in inventory".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            yes_no(plan.wipe),
        ];
    };

    vec![
        role.to_owned(),
        disk.path.clone(),
        units::bytes(disk.size_bytes),
        disk.serial.clone(),
        disk.wwid.clone(),
        disk.model.clone(),
        yes_no(plan.wipe),
    ]
}

fn wiped(plan: &InstallPlan) -> Vec<&str> {
    let mut devices: Vec<&str> = plan
        .system_disk
        .iter()
        .map(|disk| disk.path.as_str())
        .collect();
    if !plan.shared_data {
        devices.extend(plan.data_disk.iter().map(|disk| disk.path.as_str()));
    }

    devices
}

fn partition_rows(plan: &InstallPlan) -> Vec<Vec<String>> {
    plan.partitions.iter().map(partition_row).collect()
}

fn partition_row(partition: &PlannedPartition) -> Vec<String> {
    vec![
        partition.disk.clone(),
        partition.role.clone(),
        partition.name.clone(),
        partition_size(partition),
    ]
}

fn partition_size(partition: &PlannedPartition) -> String {
    if partition.fill {
        "remaining".to_owned()
    } else {
        units::bytes(partition.size_bytes)
    }
}

fn check_rows(plan: &InstallPlan) -> Vec<Vec<String>> {
    plan.checks.iter().map(check_row).collect()
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

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str, role: bool) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            size_bytes: 2048,
            model: "Disk".to_owned(),
            serial: "SN123".to_owned(),
            wwid: "naa.5000".to_owned(),
            is_system: role,
            is_data: !role,
            ..Default::default()
        }
    }

    fn plan(shared_data: bool) -> InstallPlan {
        InstallPlan {
            system_disk: Some(disk("sda", true)),
            data_disk: Some(disk("sdb", false)),
            shared_data,
            wipe: true,
            partitions: vec![PlannedPartition {
                disk: "/dev/sda".to_owned(),
                role: "EFI".to_owned(),
                name: "EFI".to_owned(),
                size_bytes: 1024,
                fill: false,
            }],
            checks: vec![
                PreflightCheck {
                    name: "disk_size".to_owned(),
                    passed: true,
                    detail: "'/dev/sda' meets the minimum size".to_owned(),
                },
                PreflightCheck {
                    name: "system_disk_in_use".to_owned(),
                    passed: false,
                    detail: "hosts the running system".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn split_assignments_list_both_disks() {
        // ARRANGE
        let plan = plan(false);

        // ACT
        let rows = disk_rows(&plan);

        // ASSERT
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], "system");
        assert_eq!(rows[0][1], "/dev/sda");
        assert_eq!(rows[1][0], "data");
        assert_eq!(rows[1][1], "/dev/sdb");
    }

    #[test]
    fn shared_assignments_list_only_the_system_disk() {
        // ARRANGE
        let plan = plan(true);

        // ACT
        let rows = disk_rows(&plan);

        // ASSERT
        assert_eq!(rows.len(), 1, "shared data lives on the system disk");
    }

    #[test]
    fn wiped_devices_skip_the_shared_data_duplicate() {
        // ARRANGE
        let plan = plan(true);

        // ACT
        let devices = wiped(&plan);

        // ASSERT
        assert_eq!(devices, ["/dev/sda"]);
    }

    #[test]
    fn missing_disks_render_as_not_in_inventory() {
        // ARRANGE
        let mut plan = plan(false);
        plan.system_disk = None;

        // ACT
        let rows = disk_rows(&plan);

        // ASSERT
        assert_eq!(rows[0][1], "not in inventory");
    }

    #[test]
    fn fixed_partitions_show_their_byte_size() {
        // ARRANGE
        let plan = plan(false);
        let partition = plan.partitions.first().expect("planned partition");

        // ACT
        let row = partition_row(partition);

        // ASSERT
        assert_eq!(row[3], "1 KiB");
    }

    #[test]
    fn fill_partitions_show_the_remaining_space() {
        // ARRANGE
        let plan = plan(false);
        let mut partition = plan.partitions[0].clone();

        // ACT
        partition.fill = true;
        let row = partition_row(&partition);

        // ASSERT
        assert_eq!(row[3], "remaining");
    }

    #[test]
    fn checks_report_their_outcome() {
        // ARRANGE
        let plan = plan(false);

        // ACT
        let rows = check_rows(&plan);

        // ASSERT
        assert_eq!(rows[0][1], "pass");
        assert_eq!(rows[1][1], "fail");
    }

    #[test]
    fn refusal_needs_a_failing_check() {
        // ARRANGE
        let refusing = plan(false);
        let mut accepted = refusing.clone();

        // ACT
        for check in &mut accepted.checks {
            check.passed = true;
        }

        // ASSERT
        assert!(refused(&refusing));
        assert!(!refused(&accepted));
    }

    #[test]
    fn system_path_is_the_system_device() {
        // ARRANGE
        let plan = plan(false);

        // ACT
        let path = system_path(&plan);

        // ASSERT
        assert_eq!(path, "/dev/sda");
    }
}
