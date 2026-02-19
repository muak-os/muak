//! VM daemon for Muak - Manages virtual machines and their life cycle

mod actor;
mod clients;
mod disk;
mod grpc;
mod hypervisor;
mod persistence;

use std::path::Path;

use actor::start_vm_actor;
use anyhow::Result;
use clients::NetworkClient;
use grpc::VmServiceImpl;
use notify::{Health, NotifyClient};
use rustix::process::{Pid, WaitOptions, waitpid};
use tokio::net::UnixListener;
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

const SOCKET_PATH: &str = "/run/vmd.sock";
const NETWORKD_SOCKET: &str = "/run/networkd.sock";
const STATE_DIR: &str = "/run/state/vmd";

/// Entry point for the VM daemon
#[tokio::main]
async fn main() -> Result<()> {
    kmsg::info!(@ "vmd", "Starting vmd");

    sysconfig::init()?;

    set_child_subreaper()?;

    let notifier = NotifyClient::new("vmd")?;
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

    let network_client = NetworkClient::connect(NETWORKD_SOCKET).await?;
    kmsg::info!(@ "vmd", "Connected to networkd");

    let vm_handle = start_vm_actor(network_client, kvm_available).await;

    if Path::new(SOCKET_PATH).exists() {
        std::fs::remove_file(SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(SOCKET_PATH)?;
    let stream = UnixListenerStream::new(listener);

    kmsg::info!(@ "vmd", "Listening on {}", SOCKET_PATH);

    notifier.ready(SOCKET_PATH)?;

    let service = VmServiceImpl::new(vm_handle);

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigchld = signal(SignalKind::child())?;

    let server = Server::builder()
        .add_service(proto::vm::vm_service_server::VmServiceServer::new(service))
        .serve_with_incoming_shutdown(stream, async {
            tokio::select! {
                _ = sigterm.recv() => {
                    kmsg::info!(@ "vmd", "Received SIGTERM, shutting down");
                }
                _ = sigint.recv() => {
                    kmsg::info!(@ "vmd", "Received SIGINT, shutting down");
                }
            }
        });

    let sigchld_handle = tokio::spawn(async move {
        loop {
            sigchld.recv().await;
            reap_children();
        }
    });

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                kmsg::error!(@ "vmd", "Server error: {}", e);
                return Err(e.into());
            }
        }
    }

    sigchld_handle.abort();
    notifier.stopping("Graceful shutdown")?;
    kmsg::info!(@ "vmd", "Shutdown complete");

    Ok(())
}

/// Sets this process as a child sub reaper to reap orphaned VM processes
fn set_child_subreaper() -> Result<()> {
    // SAFETY: prctl with known constants and no pointers, syscall is safe
    let result = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if result != 0 {
        anyhow::bail!(
            "Failed to set child subreaper: {}",
            std::io::Error::last_os_error()
        );
    }
    kmsg::info!(@ "vmd", "Set as child subreaper");
    Ok(())
}

/// Reaps zombie child processes
fn reap_children() {
    loop {
        match waitpid(
            Some(Pid::from_raw(-1).expect("Failed to get pid")),
            WaitOptions::NOHANG,
        ) {
            Ok(Some((pid, status))) if status.exited() => {
                let exit_status = status.exit_status().unwrap_or(0);
                kmsg::info!(@ "vmd", "Child {} exited with status {}", pid.as_raw_nonzero(), exit_status);
            }
            Ok(Some((pid, status))) if status.signaled() => {
                let signal = status.terminating_signal().unwrap_or(0);
                kmsg::info!(@ "vmd", "Child {} killed by signal {}", pid.as_raw_nonzero(), signal);
            }
            Ok(None) | Err(_) => break,
            Ok(Some(_)) => continue,
        }
    }
}
