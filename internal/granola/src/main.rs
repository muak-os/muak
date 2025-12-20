mod config;
mod disk;
mod provisioning;
mod supervisor;

// NOTE: These modules are temporarily disabled pending extraction to separate services.
// - grpc, vm, vmm -> apid is done, vmd will be Phase 3
// - ipc, process, signal -> will be removed once vmd is extracted
// TODO: Re-enable when vmd service is ready
// mod grpc;
// mod ipc;
// mod process;
// mod signal;
// mod vm;
// mod vmm;

use supervisor::{ServiceDef, Supervisor};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

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

    let services = vec![
        ServiceDef {
            name: "networkd".to_string(),
            binary: "/sbin/networkd".to_string(),
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "apid".to_string(),
            binary: "/sbin/apid".to_string(),
            args: vec![
                "--listen".to_string(),
                config::GRPC_SERVER_ADDR.to_string(),
            ],
            depends_on: vec!["networkd".to_string()],
        },
        // TODO: Phase 3 - Add vmd when extracted
        // ServiceDef {
        //     name: "vmd".to_string(),
        //     binary: "/sbin/vmd".to_string(),
        //     args: vec![],
        //     depends_on: vec!["networkd".to_string()],
        // },
    ];

    let mut supervisor = Supervisor::new(services)?;
    supervisor.run().await?;

    Ok(())
}
