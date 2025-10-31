use clap::{Parser, Subcommand};
use std::collections::HashMap;

pub mod process_service {
    tonic::include_proto!("muak.process.v1");
}

pub mod vm_service {
    tonic::include_proto!("muak.vm.v1");
}

use process_service::process_service_client::ProcessServiceClient;
use process_service::{ListProcessesRequest, StartProcessRequest, StopProcessRequest};
use vm_service::vm_service_client::VmServiceClient;
use vm_service::{
    upload_file_request, CreateVmRequest, DeleteVmRequest, DiskConfig, GetVmSerialLogRequest,
    ListVmsRequest, NetConfig, StartVmRequest, StopVmRequest, UploadFileRequest,
};

#[derive(Parser)]
#[command(name = "muak")]
#[command(about = "MUAK process management CLI", long_about = None)]
struct Cli {
    #[arg(long, short, default_value = "localhost:50051")]
    server: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
}

#[derive(Subcommand)]
enum ProcessAction {
    Start {
        command: String,
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    Stop {
        pid: i32,
        #[arg(short, long, default_value = "15")]
        signal: i32,
    },
    List,
}

#[derive(Subcommand)]
enum VmAction {
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        cmdline: Option<String>,
        #[arg(long)]
        kernel: Option<String>,
        #[arg(long)]
        initrd: Option<String>,
        #[arg(long, default_value = "cloud-hypervisor")]
        vmm: String,
        #[arg(long, default_value = "1")]
        cpus: i32,
        #[arg(long, default_value = "512")]
        memory: i64,
        #[arg(long)]
        disk: Vec<String>,
        #[arg(long)]
        net: Vec<String>,
    },
    Start {
        vm_id: String,
    },
    Stop {
        vm_id: String,
        #[arg(long)]
        force: bool,
    },
    Delete {
        vm_id: String,
    },
    Logs {
        vm_id: String,
        #[arg(long, short = 'n', default_value = "0")]
        tail: i64,
    },
    List,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let server_addr = format!("http://{}", cli.server);
    let channel = tonic::transport::Channel::from_shared(server_addr)?
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .connect()
        .await?;

    match cli.command {
        Commands::Process { action } => {
            let mut client = ProcessServiceClient::new(channel);
            handle_process_action(&mut client, action).await?;
        }
        Commands::Vm { action } => {
            let mut client = VmServiceClient::new(channel);
            handle_vm_action(&mut client, action).await?;
        }
    }

    Ok(())
}

async fn upload_file(
    client: &mut VmServiceClient<tonic::transport::Channel>,
    file_path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let (tx, rx) = tokio::sync::mpsc::channel(128);

    tokio::spawn(async move {
        let metadata_msg = UploadFileRequest {
            request: Some(upload_file_request::Request::Metadata(
                vm_service::UploadFileMetadata {
                    filename,
                    size: file_size as i64,
                },
            )),
        };

        if tx.send(metadata_msg).await.is_err() {
            return;
        }

        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = UploadFileRequest {
                        request: Some(upload_file_request::Request::Chunk(buffer[..n].to_vec())),
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let request = tonic::Request::new(stream);
    let response = client.upload_file(request).await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        return Err(resp.error.into());
    }

    Ok(resp.path)
}

async fn handle_process_action(
    client: &mut ProcessServiceClient<tonic::transport::Channel>,
    action: ProcessAction,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ProcessAction::Start { command, args } => {
            let request = tonic::Request::new(StartProcessRequest {
                command: command.clone(),
                args: args.clone(),
                env: HashMap::new(),
            });

            let response = client.start_process(request).await?;
            let resp = response.into_inner();

            if resp.error.is_empty() {
                println!("Started process with PID: {}", resp.pid);
            } else {
                eprintln!("Error starting process: {}", resp.error);
                std::process::exit(1);
            }
        }
        ProcessAction::Stop { pid, signal } => {
            let request = tonic::Request::new(StopProcessRequest { pid, signal });

            let response = client.stop_process(request).await?;
            let resp = response.into_inner();

            if resp.success {
                println!("Sent signal {} to process {}", signal, pid);
            } else {
                eprintln!("Error stopping process: {}", resp.error);
                std::process::exit(1);
            }
        }
        ProcessAction::List => {
            let request = tonic::Request::new(ListProcessesRequest {});

            let response = client.list_processes(request).await?;
            let resp = response.into_inner();

            if resp.processes.is_empty() {
                println!("No processes running");
            } else {
                println!(
                    "{:<8} {:<20} {:<15} {}",
                    "PID", "COMMAND", "STATUS", "STARTED"
                );
                for p in resp.processes {
                    let started = chrono::DateTime::from_timestamp(p.started_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    println!(
                        "{:<8} {:<20} {:<15} {}",
                        p.pid, p.command, p.status, started
                    );
                }
            }
        }
    }
    Ok(())
}

async fn handle_vm_action(
    client: &mut VmServiceClient<tonic::transport::Channel>,
    action: VmAction,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        VmAction::Create {
            name,
            cmdline,
            kernel,
            initrd,
            vmm,
            cpus,
            memory,
            disk,
            net,
        } => {
            // Upload kernel if it exists locally
            let kernel_path = if let Some(ref k) = kernel {
                if std::path::Path::new(k).exists() {
                    println!("Uploading kernel: {}", k);
                    match upload_file(client, k).await {
                        Ok(remote_path) => {
                            println!("Uploaded to: {}", remote_path);
                            Some(remote_path)
                        }
                        Err(e) => {
                            eprintln!("Error uploading kernel {}: {}", k, e);
                            std::process::exit(1);
                        }
                    }
                } else {
                    Some(k.clone())
                }
            } else {
                None
            };

            // Upload initrd if it exists locally
            let initrd_path = if let Some(ref i) = initrd {
                if std::path::Path::new(i).exists() {
                    println!("Uploading initrd: {}", i);
                    match upload_file(client, i).await {
                        Ok(remote_path) => {
                            println!("Uploaded to: {}", remote_path);
                            Some(remote_path)
                        }
                        Err(e) => {
                            eprintln!("Error uploading initrd {}: {}", i, e);
                            std::process::exit(1);
                        }
                    }
                } else {
                    Some(i.clone())
                }
            } else {
                None
            };

            let mut uploaded_disks = Vec::new();

            for disk_path in &disk {
                if std::path::Path::new(disk_path).exists() {
                    println!("Uploading disk: {}", disk_path);
                    match upload_file(client, disk_path).await {
                        Ok(remote_path) => {
                            println!("Uploaded to: {}", remote_path);
                            uploaded_disks.push(remote_path);
                        }
                        Err(e) => {
                            eprintln!("Error uploading disk {}: {}", disk_path, e);
                            std::process::exit(1);
                        }
                    }
                } else {
                    uploaded_disks.push(disk_path.clone());
                }
            }

            let disks: Vec<DiskConfig> = uploaded_disks
                .into_iter()
                .map(|path| {
                    // ISOs should be readonly
                    let readonly = path.to_lowercase().ends_with(".iso");
                    DiskConfig {
                        path,
                        readonly,
                    }
                })
                .collect();

            let networks: Vec<NetConfig> = net
                .into_iter()
                .map(|tap| NetConfig {
                    tap: tap.clone(),
                    mac: String::new(),
                })
                .collect();

            let request = tonic::Request::new(CreateVmRequest {
                name: name.clone(),
                kernel: kernel_path.unwrap_or_default(),
                initrd: initrd_path.unwrap_or_default(),
                cmdline: cmdline.unwrap_or_default(),
                cpus,
                memory_mb: memory,
                disks,
                networks,
                vmm_type: vmm,
            });

            let response = client.create_vm(request).await?;
            let resp = response.into_inner();

            if resp.error.is_empty() {
                let vm_id = resp.vm_id.clone();
                println!("Created VM: {} (ID: {})", name, vm_id);

                let start_request = tonic::Request::new(StartVmRequest {
                    vm_id: vm_id.clone(),
                });
                let start_response = client.start_vm(start_request).await?;
                let start_resp = start_response.into_inner();

                if start_resp.success {
                    println!("Started VM: {}", vm_id);
                } else {
                    eprintln!("Error starting VM: {}", start_resp.error);
                    std::process::exit(1);
                }
            } else {
                eprintln!("Error creating VM: {}", resp.error);
                std::process::exit(1);
            }
        }
        VmAction::Start { vm_id } => {
            let request = tonic::Request::new(StartVmRequest {
                vm_id: vm_id.clone(),
            });

            let response = client.start_vm(request).await?;
            let resp = response.into_inner();

            if resp.success {
                println!("Started VM: {}", vm_id);
            } else {
                eprintln!("Error starting VM: {}", resp.error);
                std::process::exit(1);
            }
        }
        VmAction::Stop { vm_id, force } => {
            let request = tonic::Request::new(StopVmRequest {
                vm_id: vm_id.clone(),
                force,
            });

            let response = client.stop_vm(request).await?;
            let resp = response.into_inner();

            if resp.success {
                println!("Stopped VM: {}", vm_id);
            } else {
                eprintln!("Error stopping VM: {}", resp.error);
                std::process::exit(1);
            }
        }
        VmAction::Delete { vm_id } => {
            let request = tonic::Request::new(DeleteVmRequest {
                vm_id: vm_id.clone(),
            });

            let response = client.delete_vm(request).await?;
            let resp = response.into_inner();

            if resp.success {
                println!("Deleted VM: {}", vm_id);
            } else {
                eprintln!("Error deleting VM: {}", resp.error);
                std::process::exit(1);
            }
        }
        VmAction::Logs { vm_id, tail } => {
            let request = tonic::Request::new(GetVmSerialLogRequest {
                vm_id: vm_id.clone(),
                tail_lines: tail,
            });

            let response = client.get_vm_serial_log(request).await?;
            let resp = response.into_inner();

            if resp.error.is_empty() {
                print!("{}", resp.output);
            } else {
                eprintln!("Error getting VM serial log: {}", resp.error);
                std::process::exit(1);
            }
        }
        VmAction::List => {
            let request = tonic::Request::new(ListVmsRequest {});

            let response = client.list_vms(request).await?;
            let resp = response.into_inner();

            if resp.vms.is_empty() {
                println!("No VMs");
            } else {
                println!(
                    "{:<36} {:<20} {:<12} {:<17} {:<6} {:<10} {:<8} {}",
                    "VM ID", "NAME", "STATE", "VMM", "CPUS", "MEMORY(MB)", "PID", "CREATED"
                );
                for vm in resp.vms {
                    let created = chrono::DateTime::from_timestamp(vm.created_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    let pid_str = if vm.pid == -1 {
                        "-".to_string()
                    } else {
                        vm.pid.to_string()
                    };

                    println!(
                        "{:<36} {:<20} {:<12} {:<17} {:<6} {:<10} {:<8} {}",
                        vm.vm_id, vm.name, vm.state, vm.vmm_type, vm.cpus, vm.memory_mb, pid_str, created
                    );
                }
            }
        }
    }
    Ok(())
}
