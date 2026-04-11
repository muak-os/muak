//! VM daemon for Muak - Manages virtual machines and their life cycle

mod actor;
mod clients;
mod disk;
mod hypervisor;
mod ipc;
mod persistence;

use actor::start_vm_actor;
use anyhow::Result;
use clients::NetworkClient;
use granola::Health;
use ipc::VmServiceImpl;
use rustix::process::{WaitOptions, wait};
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    pub mod vm {
        tonic::include_proto!("muak.vm.v1");
    }
    pub mod network {
        tonic::include_proto!("muak.internal.network");
    }
}

const STATE_DIR: &str = "/run/state/vmd";

/// Entry point for the VM daemon
#[granola::service("vmd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;
    set_child_subreaper()?;

    notifier.status("Initializing VM daemon", Health::Healthy)?;

    let kvm_available = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok();
    if !kvm_available {
        kmsg::warn!(@ "vmd", "/dev/kvm is not available, entering degraded mode");
        notifier.status("KVM not available", Health::Degraded)?;
    }

    tokio::fs::create_dir_all(format!("{}/vms", STATE_DIR)).await?;

    let network_client = NetworkClient::connect("/run/services/networkd.sock").await?;
    println!("Connected to networkd");

    let vm_handle = start_vm_actor(network_client, kvm_available).await;

    let stream = UnixListenerStream::new(granola::socket()?);

    notifier.ready()?;

    let service = VmServiceImpl::new(vm_handle);

    let mut sigchld = signal(SignalKind::child())?;

    let server = Server::builder()
        .add_service(proto::vm::vm_service_server::VmServiceServer::new(service))
        .serve_with_incoming_shutdown(stream, granola::shutdown_signal());

    let sigchld_handle = tokio::spawn(async move {
        loop {
            sigchld.recv().await;
            reap_children();
        }
    });

    let scrub_handle = tokio::spawn(disk::scrub::timer());

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                kmsg::error!("Server error: {}", e);
                return Err(e.into());
            }
        }
    }

    sigchld_handle.abort();
    scrub_handle.abort();

    Ok(())
}

/// Sets this process as a child sub reaper to reap orphaned VM processes.
fn set_child_subreaper() -> Result<()> {
    // SAFETY: prctl with known constants and no pointers, syscall is safe.
    let result = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if result != 0 {
        anyhow::bail!(
            "Failed to set child subreaper: {}",
            std::io::Error::last_os_error()
        );
    }
    println!("Set as child subreaper");
    Ok(())
}

/// Reaps zombie child processes.
fn reap_children() {
    loop {
        match wait(WaitOptions::NOHANG) {
            Ok(Some((pid, status))) if status.exited() => {
                let exit_status = status.exit_status().unwrap_or(0);
                println!(
                    "Child {} exited with status {}",
                    pid.as_raw_nonzero(),
                    exit_status
                );
            }
            Ok(Some((pid, status))) if status.signaled() => {
                let signal = status.terminating_signal().unwrap_or(0);
                println!("Child {} killed by signal {}", pid.as_raw_nonzero(), signal);
            }
            Ok(None) | Err(_) => break,
            Ok(Some(_)) => continue,
        }
    }
}
