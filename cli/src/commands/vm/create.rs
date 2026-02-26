use anyhow::Result;
use tonic::transport::Channel;

use crate::client::{
    CreateVmRequest, DeleteVmRequest, DiskConfig, Hypervisor, StartVmRequest, VmConfig,
    VmServiceClient, upload_file,
};
use crate::ui;

/// Creates a new VM with the specified configuration.
#[allow(clippy::too_many_arguments)]
pub async fn handle(
    client: &mut VmServiceClient<Channel>,
    name: String,
    cmdline: Option<String>,
    kernel: Option<String>,
    initrd: Option<String>,
    vmm: String,
    cpus: u32,
    memory: u64,
    disk: Vec<String>,
    disk_size: u64,
) -> Result<()> {
    let kernel = validate_kernel(kernel)?;
    validate_initrd(&initrd)?;
    validate_disks(&disk)?;

    let hypervisor = parse_hypervisor(&vmm);
    let disks = build_disk_configs(&disk);
    let config = build_vm_config(
        &name, cpus, memory, &cmdline, &initrd, disks, hypervisor, disk_size,
    );

    let steps = ui::Steps::new();

    let vm_id = create_vm(client, config, &name, &steps).await?;

    if let Err(e) = upload_vm_files(client, &vm_id, &kernel, &initrd, &disk, &steps).await {
        steps.fail(format!("Upload failed: {e}"));
        steps.finish().await;
        cleanup_vm(client, &vm_id).await;
        return Err(e);
    }

    if let Err(e) = start_vm(client, &vm_id, &steps).await {
        steps.fail(format!("Start failed: {e}"));
        steps.finish().await;
        cleanup_vm(client, &vm_id).await;
        return Err(e);
    }

    steps.finish().await;
    Ok(())
}

/// Validates kernel file exists.
fn validate_kernel(kernel: Option<String>) -> Result<String> {
    let kernel = kernel.ok_or_else(|| {
        eprintln!("{}", ui::style::error_text("Error: --kernel is required"));
        std::process::exit(1);
    })?;

    if !std::path::Path::new(&kernel).exists() {
        let msg = format!("Error: kernel file not found: {kernel}");
        eprintln!("{}", ui::style::error_text(&msg));
        std::process::exit(1);
    }

    Ok(kernel)
}

/// Validates initrd file exists if specified.
fn validate_initrd(initrd: &Option<String>) -> Result<()> {
    if let Some(path) = initrd
        && !std::path::Path::new(path).exists()
    {
        let msg = format!("Error: initrd file not found: {path}");
        eprintln!("{}", ui::style::error_text(&msg));
        std::process::exit(1);
    }
    Ok(())
}

/// Validates all disk files exist.
fn validate_disks(disks: &[String]) -> Result<()> {
    for disk_path in disks {
        if !std::path::Path::new(disk_path).exists() {
            let msg = format!("Error: disk file not found: {disk_path}");
            eprintln!("{}", ui::style::error_text(&msg));
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Parses hypervisor string to enum variant.
fn parse_hypervisor(vmm: &str) -> Hypervisor {
    match vmm.to_lowercase().as_str() {
        "firecracker" | "fc" => Hypervisor::Firecracker,
        "cloud-hypervisor" | "cloud_hypervisor" | "ch" => Hypervisor::CloudHypervisor,
        "qemu" | "kvm" => Hypervisor::Qemu,
        other => {
            let msg = format!("Warning: Unknown hypervisor '{other}', defaulting to QEMU");
            eprintln!("{}", ui::style::warn(&msg));
            Hypervisor::Qemu
        }
    }
}

/// Builds disk configuration from paths.
fn build_disk_configs(disks: &[String]) -> Vec<DiskConfig> {
    disks
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let readonly = path.to_lowercase().ends_with(".iso");
            DiskConfig {
                path: format!("disk{i}"),
                readonly,
            }
        })
        .collect()
}

/// Builds VM configuration from parameters.
#[allow(clippy::too_many_arguments)]
fn build_vm_config(
    name: &str,
    cpus: u32,
    memory: u64,
    cmdline: &Option<String>,
    initrd: &Option<String>,
    disks: Vec<DiskConfig>,
    hypervisor: Hypervisor,
    disk_size: u64,
) -> VmConfig {
    VmConfig {
        name: name.to_string(),
        cpus,
        memory_mb: memory,
        kernel: "kernel".to_string(),
        initrd: if initrd.is_some() {
            "initrd".to_string()
        } else {
            String::new()
        },
        cmdline: cmdline.clone().unwrap_or_default(),
        disks,
        hypervisor: hypervisor.into(),
        root_disk_size_mb: disk_size,
    }
}

/// Creates VM on the server and returns VM ID.
async fn create_vm(
    client: &mut VmServiceClient<Channel>,
    config: VmConfig,
    name: &str,
    steps: &ui::Steps,
) -> Result<String> {
    steps.start(format!("Creating VM '{name}'..."));

    let request = tonic::Request::new(CreateVmRequest {
        config: Some(config),
    });

    let response = client.create_vm(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        let msg = format!("Error creating VM: {}", resp.error);
        steps.fail(&msg);
        return Err(anyhow::anyhow!("{msg}"));
    }

    let vm_id = resp.vm_id.clone();
    let msg = format!("Created VM: {name} (ID: {vm_id})");
    steps.complete(&msg);

    Ok(vm_id)
}

/// Uploads kernel, initrd, and disk files to the VM.
async fn upload_vm_files(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    kernel: &str,
    initrd: &Option<String>,
    disks: &[String],
    steps: &ui::Steps,
) -> Result<()> {
    upload_kernel(client, vm_id, kernel, steps).await?;

    if let Some(initrd_path) = initrd {
        upload_initrd(client, vm_id, initrd_path, steps).await?;
    }

    upload_disks(client, vm_id, disks, steps).await?;

    Ok(())
}

/// Uploads kernel file to the VM.
async fn upload_kernel(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    kernel: &str,
    steps: &ui::Steps,
) -> Result<()> {
    steps.start(format!("Uploading kernel: {kernel}"));
    match upload_file(client, kernel, Some(vm_id), Some("kernel")).await {
        Ok(remote_path) => {
            steps.complete(format!("Uploaded kernel to: {remote_path}"));
            Ok(())
        }
        Err(e) => {
            steps.fail(format!("Error uploading kernel: {e}"));
            Err(e)
        }
    }
}

/// Uploads initrd file to the VM.
async fn upload_initrd(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    initrd_path: &str,
    steps: &ui::Steps,
) -> Result<()> {
    steps.start(format!("Uploading initrd: {initrd_path}"));
    match upload_file(client, initrd_path, Some(vm_id), Some("initrd")).await {
        Ok(remote_path) => {
            steps.complete(format!("Uploaded initrd to: {remote_path}"));
            Ok(())
        }
        Err(e) => {
            steps.fail(format!("Error uploading initrd: {e}"));
            Err(e)
        }
    }
}

/// Uploads all disk files to the VM.
async fn upload_disks(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    disks: &[String],
    steps: &ui::Steps,
) -> Result<()> {
    for (i, disk_path) in disks.iter().enumerate() {
        let target_name = format!("disk{i}");
        steps.start(format!("Uploading disk: {disk_path}"));
        match upload_file(client, disk_path, Some(vm_id), Some(&target_name)).await {
            Ok(remote_path) => {
                steps.complete(format!("Uploaded disk to: {remote_path}"));
            }
            Err(e) => {
                steps.fail(format!("Error uploading disk: {e}"));
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Starts the VM after files are uploaded.
async fn start_vm(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    steps: &ui::Steps,
) -> Result<()> {
    steps.start(format!("Starting VM {vm_id}..."));

    let start_request = tonic::Request::new(StartVmRequest {
        vm_id: vm_id.to_string(),
    });
    let start_response = client.start_vm(start_request).await?;
    let start_resp = start_response.into_inner();

    if start_resp.success {
        steps.complete(format!("Started VM: {vm_id}"));
        Ok(())
    } else {
        let msg = format!("Error starting VM: {}", start_resp.error);
        steps.fail(&msg);
        Err(anyhow::anyhow!("Failed to start VM: {}", start_resp.error))
    }
}

/// Cleans up VM on failure.
async fn cleanup_vm(client: &mut VmServiceClient<Channel>, vm_id: &str) {
    let delete_request = tonic::Request::new(DeleteVmRequest {
        vm_id: vm_id.to_string(),
    });
    if let Err(e) = client.delete_vm(delete_request).await {
        let msg = format!("Warning: Failed to clean up VM: {e}");
        eprintln!("{}", ui::style::warn(&msg));
    }
}
