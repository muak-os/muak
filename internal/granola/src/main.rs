mod config;
mod disk;
mod provisioning;
mod supervisor;

// NOTE: These modules are temporarily disabled pending extraction to separate services.
// - grpc, vm, vmm -> will become grpcd and vmd in Phase 2 and 3
// - ipc, process, signal -> will be removed once grpcd/vmd are extracted
// TODO: Re-enable when grpcd/vmd services are ready
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
        // TODO: Phase 2 - Add grpcd when extracted
        // ServiceDef {
        //     name: "grpcd".to_string(),
        //     binary: "/usr/bin/grpcd".to_string(),
        //     args: vec![
        //         "--listen".to_string(),
        //         config::GRPC_SERVER_ADDR.to_string(),
        //     ],
        //     depends_on: vec!["networkd".to_string()],
        // },
        // TODO: Phase 3 - Add vmd when extracted
        // ServiceDef {
        //     name: "vmd".to_string(),
        //     binary: "/usr/bin/vmd".to_string(),
        //     args: vec![],
        //     depends_on: vec!["networkd".to_string()],
        // },
    ];

    let mut supervisor = Supervisor::new(services)?;
    supervisor.run().await?;

    Ok(())
}
