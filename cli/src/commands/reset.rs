use anyhow::{Context as _, Result};
use config::user::ClientConfig;
use tonic::transport::Channel;
use tui::console::Console;
use tui::prompt::Prompter;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{
    DiskInfo, FactoryResetRequest, GetConfigRequest, PartitionInfo,
};
use crate::commands::Failure;
use crate::commands::disks::{self, column_refs, text};

const CONFIRM_PHRASE: &str = "FACTORY RESET";
const PLAN_COLUMNS: [&str; 4] = ["PARTITION", "ROLE", "SIZE", "MOUNT"];
// Mirrors the GPT names returned by `Role::gpt_name()` in `libs/disk/src/role.rs`.
// The plan lists every matching partition, while the reset deletes the first
// partition of each role, so duplicates across disks can be over-reported.
const REMOVABLE_ROLES: [&str; 3] = ["STATE", "EPHEMERAL", "DATA"];

struct Plan<'a> {
    installed: bool,
    context_name: Option<&'a str>,
    rows: Vec<[String; 4]>,
}

impl Plan<'_> {
    fn table(&self) -> Table {
        let table = Table::new().header(&PLAN_COLUMNS);

        self.rows
            .iter()
            .fold(table, |table, row| table.row(&column_refs(row)))
    }

    fn context_message(&self) -> Option<String> {
        self.context_name
            .map(|name| format!("Context '{name}' would be removed"))
    }
}

/// Handles the reset command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    dry_run: bool,
    yes: bool,
    context_name: Option<String>,
    console: &Console,
) -> Result<()> {
    console.line(Role::Error, "WARNING: This will perform a factory reset!");
    console.line(
        Role::ErrorText,
        "All data, VMs, and configuration will be permanently deleted.",
    );
    console.line(
        Role::ErrorText,
        "The system will reboot into maintenance mode.\n",
    );

    if dry_run {
        let config_resp = client
            .get_config(tonic::Request::new(GetConfigRequest {}))
            .await?;
        let installed = config_resp.into_inner().error.is_empty();
        let disks = disks::fetch(client).await?;

        return print_plan(&plan(&disks, installed, context_name.as_deref()), console);
    }

    if !yes {
        let confirmed = match Prompter::stdio() {
            Some(mut prompter) => prompter.confirm_phrase("", CONFIRM_PHRASE)?,
            None => false,
        };

        if !confirmed {
            console.line(Role::Warn, "Factory reset cancelled.");
            return Err(Failure::declined("Factory reset cancelled").into());
        }
    }

    let steps = console.steps()?;

    steps.start("Initiating factory reset...");

    let request = tonic::Request::new(FactoryResetRequest {});

    let response = match client.factory_reset(request).await {
        Ok(resp) => resp,
        Err(e) => {
            steps.fail("Factory reset failed");
            let _finish_result = steps.finish();
            return Err(e).context("Failed to send factory reset request");
        }
    };

    let resp = response.into_inner();

    if !resp.success {
        let msg = format!("Factory reset failed: {}", resp.error);
        steps.fail(&msg);
        let _finish_result = steps.finish();
        return Err(anyhow::anyhow!("{msg}"));
    }

    steps.complete("Factory reset initiated");

    if let Some(ctx_name) = context_name
        && let Ok(mut config) = ClientConfig::load()
        && config.remove_context(&ctx_name).is_ok()
        && config.save().is_ok()
    {
        let msg = format!("Removed context '{ctx_name}'");
        steps.complete(&msg);
    }

    steps.start("Rebooting into maintenance mode...");
    steps.finish()?;

    Ok(())
}

fn plan<'a>(disks: &[DiskInfo], installed: bool, context_name: Option<&'a str>) -> Plan<'a> {
    Plan {
        installed,
        context_name,
        rows: plan_rows(disks),
    }
}

fn plan_rows(disks: &[DiskInfo]) -> Vec<[String; 4]> {
    disks
        .iter()
        .flat_map(|disk| disk.partitions.iter())
        .filter(|part| REMOVABLE_ROLES.contains(&part.gpt_name.as_str()))
        .map(plan_row)
        .collect()
}

fn plan_row(part: &PartitionInfo) -> [String; 4] {
    [
        part.path.clone(),
        part.gpt_name.clone(),
        units::bytes(part.size_bytes),
        text(&part.mount_point),
    ]
}

fn print_plan(plan: &Plan<'_>, console: &Console) -> Result<()> {
    if !plan.installed {
        console.line(Role::Warn, "System is not installed yet");
    }

    if plan.rows.is_empty() {
        console.line(Role::Warn, "No STATE, EPHEMERAL or DATA partitions found");
    } else {
        console.info("Dry run: a factory reset would remove:");
        console.table(&plan.table())?;
    }

    if let Some(message) = plan.context_message() {
        console.info(&message);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn partition(name: &str, gpt_name: &str, mount_point: &str) -> PartitionInfo {
        PartitionInfo {
            number: 1,
            start_sector: 2048,
            size_bytes: GIB,
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            fstype: "ext4".to_owned(),
            gpt_name: gpt_name.to_owned(),
            mount_point: mount_point.to_owned(),
            in_use: true,
        }
    }

    fn disk(name: &str, partitions: Vec<PartitionInfo>) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            size_bytes: GIB,
            model: "Model".to_owned(),
            removable: false,
            read_only: false,
            partitions,
            serial: "Serial".to_owned(),
            wwid: "Wwid".to_owned(),
            bus_path: String::new(),
            subsystem: String::new(),
            rotational: true,
            cdrom: false,
            symlinks: Vec::new(),
            is_system: true,
            is_data: false,
        }
    }

    fn render(table: &Table) -> String {
        let mut screen = tui::screen::Ansi::new(Vec::new(), tui::size::Size::default());
        table
            .render(&mut screen, tui::style::Theme::plain())
            .unwrap();

        String::from_utf8(screen.into_inner()).unwrap()
    }

    #[test]
    fn plan_lists_the_removable_partitions_of_an_installed_system() {
        // ARRANGE
        let disks = vec![disk(
            "sda",
            vec![
                partition("sda1", "EFI", "/boot"),
                partition("sda2", "STATE", "/run/muak"),
                partition("sda3", "EPHEMERAL", ""),
                partition("sda4", "DATA", ""),
            ],
        )];

        // ACT
        let plan = plan(&disks, true, None);

        // ASSERT
        assert!(plan.installed);
        assert_eq!(plan.rows.len(), 3);
        assert_eq!(plan.rows[0][0], "/dev/sda2");
        assert_eq!(plan.rows[0][1], "STATE");
        assert_eq!(plan.rows[0][2], units::bytes(GIB));
        assert_eq!(plan.rows[0][3], "/run/muak");
        assert_eq!(plan.rows[1][0], "/dev/sda3");
        assert_eq!(plan.rows[1][1], "EPHEMERAL");
        assert_eq!(plan.rows[1][3], "-");
        assert_eq!(plan.rows[2][0], "/dev/sda4");
        assert_eq!(plan.rows[2][1], "DATA");
        assert_eq!(plan.rows[2][3], "-");
    }

    #[test]
    fn plan_reports_an_uninstalled_system() {
        // ARRANGE
        let disks = Vec::new();

        // ACT
        let plan = plan(&disks, false, None);

        // ASSERT
        assert!(!plan.installed);
        assert_eq!(plan.rows.len(), 0);
        assert!(plan.context_message().is_none());
    }

    #[test]
    fn plan_reports_the_context_that_would_be_removed() {
        // ARRANGE
        let disks = vec![disk("sda", vec![partition("sda2", "STATE", "")])];

        // ACT
        let plan = plan(&disks, true, Some("edge"));

        // ASSERT
        assert_eq!(
            plan.context_message().as_deref(),
            Some("Context 'edge' would be removed")
        );
        assert_eq!(plan.rows.len(), 1);
    }

    #[test]
    fn plan_ignores_disks_without_removable_partitions() {
        // ARRANGE
        let disks = vec![
            disk("sda", vec![partition("sda1", "EFI", "/boot")]),
            disk("sdb", vec![partition("sdb1", "", "")]),
        ];

        // ACT
        let plan = plan(&disks, true, None);

        // ASSERT
        assert_eq!(plan.rows.len(), 0);
    }

    #[test]
    fn plan_table_renders_the_header_and_one_line_per_removable_partition() {
        // ARRANGE
        let disks = vec![disk(
            "sda",
            vec![
                partition("sda1", "EFI", "/boot"),
                partition("sda2", "STATE", "/run/muak"),
                partition("sda3", "DATA", ""),
            ],
        )];
        let plan = plan(&disks, true, None);

        // ACT
        let output = render(&plan.table());

        // ASSERT
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("PARTITION"));
        assert!(lines[0].contains("ROLE"));
        assert!(lines[0].contains("SIZE"));
        assert!(lines[0].contains("MOUNT"));
        assert!(lines[1].starts_with("/dev/sda2"));
        assert!(lines[2].starts_with("/dev/sda3"));
    }
}
