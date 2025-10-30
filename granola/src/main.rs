mod config;
mod grpc;
mod ipc;
mod log;
mod network;
mod process;
mod signal;
mod vm;
mod vmm;

use ipc::IpcServer;
use process::ProcessManager;
use signal::SignalHandler;
use std::sync::Arc;
use vm::VmManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log!("granola", "PID 1 init started");

    std::fs::create_dir_all(config::MUAK_DISKS_DIR)?;
    log!("granola", "Created {} directory", config::MUAK_DISKS_DIR);

    let network_manager = Arc::new(network::NetworkManager::new().await?);

    network_manager.initialize_host().await?;
    network_manager.setup_bridge().await?;
    network_manager.start_dhcp_server().await?;

    log!("granola", "Network initialized");

    let process_manager = ProcessManager::new();
    let vm_manager = VmManager::new(process_manager.clone(), network_manager.clone());

    let mut signal_handler = SignalHandler::new()?;
    log!("granola", "Signal handlers installed");

    let ipc_server = Arc::new(IpcServer::new()?);
    log!(
        "granola",
        "IPC server listening on {}",
        config::GRANOLA_SOCKET_PATH
    );

    tokio::task::spawn_blocking({
        let pm = process_manager.clone();
        move || match pm.spawn_service(
            "grpc-server",
            vec![config::GRPC_SERVER_ADDR.to_string()],
            grpc::main,
        ) {
            Ok(pid) => {
                log!("granola", "Spawned grpc-server (PID {})", pid);
            }
            Err(e) => {
                log!("granola", "ERROR: Failed to spawn grpc-server: {}", e);
            }
        }
    })
    .await?;

    let pm_clone = process_manager.clone();
    tokio::spawn(async move { signal_handler.handle_signals(&pm_clone).await });

    loop {
        let mut stream = match ipc_server.accept_connection().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let pm = process_manager.clone();
        let vm = vm_manager.clone();
        let ipc = ipc_server.clone();
        tokio::spawn(async move {
            if let Ok(message) = ipc.read_message(&mut stream).await {
                let response = ipc.handle_message(message, &pm, &vm).await;
                if let Err(e) = ipc.send_response(&mut stream, &response).await {
                    log!("granola", "Failed to send IPC response: {}", e);
                }
            }
        });
    }
}
