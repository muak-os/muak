use anyhow::Result;
use tonic::transport::Channel;

use crate::client::{
    upload::upload,
    vm_service::{
        CreateVmRequest, DeleteVmRequest, DiskConfig, Hypervisor, StartVmRequest, VmConfig,
        vm_service_client::VmServiceClient,
    },
};
use crate::ui;

/// Parameters for creating a new VM.
pub struct VmSpec {
    pub name: String,
    pub cmdline: Option<String>,
    pub kernel: Option<String>,
    pub initrd: Option<String>,
    pub vmm: String,
    pub cpus: u32,
    pub memory: u64,
    pub disk: Vec<String>,
    pub disk_size: u64,
}

/// Creates a new VM with the specified configuration.
pub async fn handle(client: &mut VmServiceClient<Channel>, args: VmSpec) -> Result<()> {
    let kernel = validate_kernel(args.kernel.as_deref())?;
    validate_initrd(args.initrd.as_deref())?;
    validate_disks(&args.disk)?;

    let hypervisor = parse_hypervisor(&args.vmm);
    let disks = build_disk_configs(&args.disk);
    let config = build_vm_config(&args, disks, hypervisor);

    let steps = ui::steps::Steps::new();

    let vm_id = create_vm(client, config, &args.name, &steps).await?;

    if let Err(e) = upload_vm_files(
        client,
        &vm_id,
        &kernel,
        args.initrd.as_deref(),
        &args.disk,
        &steps,
    )
    .await
    {
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
fn validate_kernel(kernel: Option<&str>) -> Result<String> {
    let kernel = kernel.ok_or_else(|| anyhow::anyhow!("--kernel is required"))?;

    if !std::path::Path::new(kernel).exists() {
        return Err(anyhow::anyhow!("kernel file not found: {kernel}"));
    }

    Ok(kernel.to_owned())
}

/// Validates initrd file exists if specified.
fn validate_initrd(initrd: Option<&str>) -> Result<()> {
    if let Some(path) = initrd
        && !std::path::Path::new(path).exists()
    {
        return Err(anyhow::anyhow!("initrd file not found: {path}"));
    }
    Ok(())
}

/// Validates all disk files exist.
fn validate_disks(disks: &[String]) -> Result<()> {
    for disk_path in disks {
        if !std::path::Path::new(disk_path).exists() {
            return Err(anyhow::anyhow!("disk file not found: {disk_path}"));
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
fn build_vm_config(args: &VmSpec, disks: Vec<DiskConfig>, hypervisor: Hypervisor) -> VmConfig {
    VmConfig {
        name: args.name.clone(),
        cpus: args.cpus,
        memory_mb: args.memory,
        kernel: "kernel".to_owned(),
        initrd: if args.initrd.is_some() {
            "initrd".to_owned()
        } else {
            String::new()
        },
        cmdline: args.cmdline.clone().unwrap_or_default(),
        disks,
        hypervisor: hypervisor.into(),
        root_disk_size_mb: args.disk_size,
    }
}

/// Creates VM on the server and returns VM ID.
async fn create_vm(
    client: &mut VmServiceClient<Channel>,
    config: VmConfig,
    name: &str,
    steps: &ui::steps::Steps,
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
    initrd: Option<&str>,
    disks: &[String],
    steps: &ui::steps::Steps,
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
    steps: &ui::steps::Steps,
) -> Result<()> {
    steps.start(format!("Uploading kernel: {kernel}"));
    match upload(client, kernel, Some(vm_id), Some("kernel")).await {
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
    steps: &ui::steps::Steps,
) -> Result<()> {
    steps.start(format!("Uploading initrd: {initrd_path}"));
    match upload(client, initrd_path, Some(vm_id), Some("initrd")).await {
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
    steps: &ui::steps::Steps,
) -> Result<()> {
    for (i, disk_path) in disks.iter().enumerate() {
        let target_name = format!("disk{i}");
        steps.start(format!("Uploading disk: {disk_path}"));
        match upload(client, disk_path, Some(vm_id), Some(&target_name)).await {
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
    steps: &ui::steps::Steps,
) -> Result<()> {
    steps.start(format!("Starting VM {vm_id}..."));

    let start_request = tonic::Request::new(StartVmRequest {
        vm_id: vm_id.to_owned(),
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
        vm_id: vm_id.to_owned(),
    });
    if let Err(e) = client.delete_vm(delete_request).await {
        let msg = format!("Warning: Failed to clean up VM: {e}");
        eprintln!("{}", ui::style::warn(&msg));
    }
}
