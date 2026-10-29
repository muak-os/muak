use anyhow::Result;
use tonic::transport::Channel;
use tui::style::Role;
use tui::table::Table;
use tui::units;

use crate::client::vm_service::vm_service_client::VmServiceClient;
use crate::client::vm_service::{ListVmsRequest, VmInfo};
use crate::console::Console;
use crate::format::display::{hypervisor_to_string, vm_state_to_string};
use crate::format::time::format_timestamp;

/// Lists all VMs.
pub async fn handle(client: &mut VmServiceClient<Channel>, console: &Console) -> Result<()> {
    let request = tonic::Request::new(ListVmsRequest {});

    let response = client.list_vms(request).await?;
    let resp = response.into_inner();

    if resp.vms.is_empty() {
        console.line(Role::Warn, "No VMs");
        return Ok(());
    }

    let mut table = Table::new().header(&[
        "VM ID",
        "NAME",
        "STATE",
        "VMM",
        "CPUS",
        "MEMORY(MB)",
        "DISK",
        "PID",
        "CREATED",
    ]);

    for vm in resp.vms {
        table = vm_rows(table, &vm);
    }

    console.table(&table)?;

    Ok(())
}

/// Appends a row describing a single VM to the table.
fn vm_rows(table: Table, vm: &VmInfo) -> Table {
    let created = format_timestamp(vm.created_at);

    let pid_str = if vm.pid == -1_i32 {
        "-".to_owned()
    } else {
        vm.pid.to_string()
    };

    let state_str = vm_state_to_string(vm.state);
    let config = vm.config.as_ref();
    let cpus = config.map_or(0, |cfg| cfg.cpus);
    let memory_mb = config.map_or(0, |cfg| cfg.memory_mb);
    let hypervisor = config.map_or(0_i32, |cfg| cfg.hypervisor);
    let vmm_str = hypervisor_to_string(hypervisor);

    let disk_str = if let Some(usage) = vm.disk_usage.as_ref() {
        format!(
            "{}/{}",
            units::bytes(usage.used_bytes),
            units::bytes(usage.quota_bytes)
        )
    } else {
        "-".to_owned()
    };

    let cpus_str = cpus.to_string();
    let mem_str = memory_mb.to_string();

    table.row(&[
        &vm.vm_id, &vm.name, state_str, vmm_str, &cpus_str, &mem_str, &disk_str, &pid_str, &created,
    ])
}
