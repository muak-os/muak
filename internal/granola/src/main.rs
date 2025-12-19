mod config;
mod disk;
mod grpc;
mod ipc;
mod network;
mod process;
mod provisioning;
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
    kmsg::init("granola")?;
    kmsg::info!("PID 1 init started");

    match provisioning::status() {
        provisioning::InstallationStatus::Live => {
            kmsg::info!("CURRENTLY IN MAINTENANCE MODE");
            kmsg::info!("   Run 'muakctl install --target <disk>' to install");
        }
        provisioning::InstallationStatus::Installed => {
            kmsg::info!("Running from INSTALLED DISK");

            if let Err(e) = disk::mount_partitions() {
                kmsg::warn!("Failed to mount partitions: {}", e);
                // TODO: set maintenance mode here to recover
            } else if let Err(e) = provisioning::check_and_handle_pending_validation() {
                kmsg::warn!("Update validation handling failed: {}", e);
            }
        }
    }

    std::fs::create_dir_all(config::MUAK_DISKS_DIR)?;
    kmsg::info!("Created {} directory", config::MUAK_DISKS_DIR);

    let mut signal_handler = SignalHandler::new()?;
    kmsg::info!("Signal handlers installed");

    let network_actor = network::start_network_actor()
        .await
        .expect("Failed to start network actor");

    let actor_clone = network_actor.clone();
    tokio::spawn(async move {
        if let Err(e) = actor_clone.initialize_with_retry().await {
            kmsg::error!(@ "network", "Fatal: Network initialization failed: {}", e);
        }
    });

    let mut snap_rx = network_actor.subscribe();
    tokio::spawn(async move {
        let mut last_state: Option<network::model::NetworkStateKind> = None;
        while snap_rx.changed().await.is_ok() {
            let snap = snap_rx.borrow().clone();
            let should_log = match &last_state {
                None => true,
                Some(prev) => {
                    *prev != snap.state || snap.state != network::model::NetworkStateKind::Ready
                }
            };
            if should_log {
                kmsg::info!(
                    @ "network",
                    "Snapshot state={:?} primary={:?} interfaces={}",
                    snap.state,
                    snap.primary,
                    snap.interfaces.len()
                );
            }
            last_state = Some(snap.state);
        }
    });

    let process_manager = ProcessManager::new();
    let network_arc = Arc::new(network_actor);
    let vm_manager = VmManager::new(process_manager.clone(), network_arc.clone());

    let ipc_server = Arc::new(IpcServer::new()?);
    kmsg::info!("IPC server listening on {}", config::GRANOLA_SOCKET_PATH);

    tokio::task::spawn_blocking({
        let pm = process_manager.clone();
        move || match pm.spawn_service(
            "grpc-server",
            vec![config::GRPC_SERVER_ADDR.to_string()],
            grpc::main,
        ) {
            Ok(pid) => {
                kmsg::info!("Spawned grpc-server (PID {})", pid);
            }
            Err(e) => {
                kmsg::error!("Failed to spawn grpc-server: {}", e);
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
                    kmsg::error!("Failed to send IPC response: {}", e);
                }
            }
        });
    }
}
