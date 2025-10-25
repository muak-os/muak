mod grpc;
mod ipc;
mod log;
mod network;
mod process;
mod signal;
mod vm;

use ipc::IpcServer;
use process::ProcessManager;
use signal::SignalHandler;
use vm::VmManager;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log!("granola", "PID 1 init started");

    std::fs::create_dir_all("/tmp/muak/disks")?;
    log!("granola", "Created /tmp/muak/disks directory");

    let process_manager = ProcessManager::new();
    let vm_manager = VmManager::new(process_manager.clone());

    let mut signal_handler = SignalHandler::new()?;
    log!("granola", "Signal handlers installed");

    let ipc_server = Arc::new(IpcServer::new()?);
    log!("granola", "IPC server listening on /run/granola.sock");

    tokio::task::spawn_blocking({
        let pm = process_manager.clone();
        let vm = vm_manager.clone();
        move || {
            let pid = pm
                .spawn_service("network-manager", vec![], network::main)
                .unwrap();
            log!("granola", "Spawned network-manager (PID {})", pid);

            let pid = pm
                .spawn_service("grpc-server", vec!["0.0.0.0:50051".to_string()], || grpc::main(vm))
                .unwrap();
            log!("granola", "Spawned grpc-server (PID {})", pid);
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
        let ipc = ipc_server.clone();
        tokio::spawn(async move {
            if let Ok(message) = ipc.read_message(&mut stream).await {
                let response = ipc.handle_message(message, &pm);
                let _ = ipc.send_response(&mut stream, &response).await;
            }
        });
    }
}
