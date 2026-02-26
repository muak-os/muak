use anyhow::Result;
use tonic::transport::Channel;

use crate::client::{ListVmsRequest, VmServiceClient};
use crate::format::{
    format_size, format_timestamp, hypervisor_to_string, time::TimeSeparator, vm_state_to_string,
};
use crate::ui;

/// Lists all VMs.
pub async fn handle(client: &mut VmServiceClient<Channel>) -> Result<()> {
    let request = tonic::Request::new(ListVmsRequest {});

    let response = client.list_vms(request).await?;
    let resp = response.into_inner();

    if resp.vms.is_empty() {
        println!("{}", ui::style::warn("No VMs"));
    } else {
        let mut table = ui::Table::new().header(&[
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
            let created = format_timestamp(vm.created_at, TimeSeparator::Display);

            let pid_str = if vm.pid == -1 {
                "-".to_string()
            } else {
                vm.pid.to_string()
            };

            let state_str = vm_state_to_string(vm.state);
            let config = vm.config.as_ref();
            let cpus = config.map(|c| c.cpus).unwrap_or(0);
            let memory_mb = config.map(|c| c.memory_mb).unwrap_or(0);
            let hypervisor = config.map(|c| c.hypervisor).unwrap_or(0);
            let vmm_str = hypervisor_to_string(hypervisor);

            let disk_str = if let Some(usage) = &vm.disk_usage {
                format!(
                    "{}/{}",
                    format_size(usage.used_bytes),
                    format_size(usage.quota_bytes)
                )
            } else {
                "-".to_string()
            };

            let cpus_str = cpus.to_string();
            let mem_str = memory_mb.to_string();

            table = table.row(&[
                &vm.vm_id, &vm.name, state_str, vmm_str, &cpus_str, &mem_str, &disk_str, &pid_str,
                &created,
            ]);
        }

        table.print();
    }

    Ok(())
}
