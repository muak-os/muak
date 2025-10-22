use clap::{Parser, Subcommand};
use std::collections::HashMap;

pub mod process_service {
    tonic::include_proto!("muak.process.v1");
}

use process_service::process_service_client::ProcessServiceClient;
use process_service::{ListProcessesRequest, StartProcessRequest, StopProcessRequest};

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let server_addr = format!("http://{}", cli.server);
    let channel = tonic::transport::Channel::from_shared(server_addr)?
        .connect()
        .await?;

    let mut client = ProcessServiceClient::new(channel);

    match cli.command {
        Commands::Process { action } => match action {
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
        },
    }

    Ok(())
}
