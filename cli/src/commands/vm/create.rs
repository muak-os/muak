use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{
    CreateVmRequest, DeleteVmRequest, DiskConfig, Hypervisor, StartVmRequest, VmConfig,
    VmServiceClient, upload_file,
};

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

    let vm_id = create_vm(client, config, &name).await?;

    if let Err(e) = upload_vm_files(client, &vm_id, &kernel, &initrd, &disk).await {
        cleanup_vm(client, &vm_id).await;
        return Err(e);
    }

    if let Err(e) = start_vm(client, &vm_id).await {
        cleanup_vm(client, &vm_id).await;
        return Err(e);
    }

    Ok(())
}

fn validate_kernel(kernel: Option<String>) -> Result<String> {
    let kernel = kernel.ok_or_else(|| {
        eprintln!("{}", "Error: --kernel is required".red());
        std::process::exit(1);
    })?;

    if !std::path::Path::new(&kernel).exists() {
        eprintln!(
            "{}",
            format!("Error: kernel file not found: {kernel}").red()
        );
        std::process::exit(1);
    }

    Ok(kernel)
}

fn validate_initrd(initrd: &Option<String>) -> Result<()> {
    if let Some(path) = initrd
        && !std::path::Path::new(path).exists()
    {
        eprintln!("{}", format!("Error: initrd file not found: {path}").red());
        std::process::exit(1);
    }
    Ok(())
}

fn validate_disks(disks: &[String]) -> Result<()> {
    for disk_path in disks {
        if !std::path::Path::new(disk_path).exists() {
            eprintln!(
                "{}",
                format!("Error: disk file not found: {disk_path}").red()
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

fn parse_hypervisor(vmm: &str) -> Hypervisor {
    match vmm.to_lowercase().as_str() {
        "firecracker" | "fc" => Hypervisor::Firecracker,
        "cloud-hypervisor" | "cloud_hypervisor" | "ch" => Hypervisor::CloudHypervisor,
        "qemu" | "kvm" => Hypervisor::Qemu,
        other => {
            eprintln!(
                "{}",
                format!("Warning: Unknown hypervisor '{other}', defaulting to QEMU").yellow()
            );
            Hypervisor::Qemu
        }
    }
}

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

async fn create_vm(
    client: &mut VmServiceClient<Channel>,
    config: VmConfig,
    name: &str,
) -> Result<String> {
    let request = tonic::Request::new(CreateVmRequest {
        config: Some(config),
    });

    let response = client.create_vm(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        eprintln!("{}", format!("Error creating VM: {}", resp.error).red());
        std::process::exit(1);
    }

    let vm_id = resp.vm_id.clone();
    println!("{}", format!("Created VM: {name} (ID: {vm_id})").green());

    Ok(vm_id)
}

async fn upload_vm_files(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    kernel: &str,
    initrd: &Option<String>,
    disks: &[String],
) -> Result<()> {
    upload_kernel(client, vm_id, kernel).await?;

    if let Some(initrd_path) = initrd {
        upload_initrd(client, vm_id, initrd_path).await?;
    }

    upload_disks(client, vm_id, disks).await?;

    Ok(())
}

async fn upload_kernel(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    kernel: &str,
) -> Result<()> {
    println!("{}", format!("Uploading kernel: {kernel}").blue());
    match upload_file(client, kernel, Some(vm_id), Some("kernel")).await {
        Ok(remote_path) => {
            println!("{}", format!("Uploaded to: {remote_path}").green());
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", format!("Error uploading kernel: {e}").red());
            Err(e)
        }
    }
}

async fn upload_initrd(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    initrd_path: &str,
) -> Result<()> {
    println!("{}", format!("Uploading initrd: {initrd_path}").blue());
    match upload_file(client, initrd_path, Some(vm_id), Some("initrd")).await {
        Ok(remote_path) => {
            println!("{}", format!("Uploaded to: {remote_path}").green());
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", format!("Error uploading initrd: {e}").red());
            Err(e)
        }
    }
}

async fn upload_disks(
    client: &mut VmServiceClient<Channel>,
    vm_id: &str,
    disks: &[String],
) -> Result<()> {
    for (i, disk_path) in disks.iter().enumerate() {
        let target_name = format!("disk{i}");
        println!("{}", format!("Uploading disk: {disk_path}").blue());
        match upload_file(client, disk_path, Some(vm_id), Some(&target_name)).await {
            Ok(remote_path) => {
                println!("{}", format!("Uploaded to: {remote_path}").green());
            }
            Err(e) => {
                eprintln!("{}", format!("Error uploading disk: {e}").red());
                return Err(e);
            }
        }
    }
    Ok(())
}

async fn start_vm(client: &mut VmServiceClient<Channel>, vm_id: &str) -> Result<()> {
    let start_request = tonic::Request::new(StartVmRequest {
        vm_id: vm_id.to_string(),
    });
    let start_response = client.start_vm(start_request).await?;
    let start_resp = start_response.into_inner();

    if start_resp.success {
        println!("{}", format!("Started VM: {vm_id}").green());
        Ok(())
    } else {
        eprintln!(
            "{}",
            format!("Error starting VM: {}", start_resp.error).red()
        );
        Err(anyhow::anyhow!("Failed to start VM: {}", start_resp.error))
    }
}

async fn cleanup_vm(client: &mut VmServiceClient<Channel>, vm_id: &str) {
    let delete_request = tonic::Request::new(DeleteVmRequest {
        vm_id: vm_id.to_string(),
    });
    if let Err(e) = client.delete_vm(delete_request).await {
        eprintln!(
            "{}",
            format!("Warning: Failed to clean up VM: {e}").yellow()
        );
    }
}
