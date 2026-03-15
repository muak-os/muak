//! VM daemon for Muak - Manages virtual machines and their life cycle

mod actor;
mod clients;
mod disk;
mod hypervisor;
mod ipc;
mod persistence;

use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;

use actor::start_vm_actor;
use anyhow::Result;
use clients::NetworkClient;
use ipc::VmServiceImpl;
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

const STATE_DIR: &str = "/run/state/vmd";

/// Entry point for the VM daemon
#[tokio::main]
async fn main() -> Result<()> {
    kmsg::info!(@ "vmd", "Starting vmd");

    config::init()?;

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

    let network_client = NetworkClient::connect("/run/services/networkd.sock").await?;
    println!("Connected to networkd");

    let vm_handle = start_vm_actor(network_client, kvm_available).await;

    // SAFETY: granola pre-binds the socket and passes it as FD 3 before exec.
    let std_listener = unsafe { StdUnixListener::from_raw_fd(3) };
    std_listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(std_listener)?;
    let stream = UnixListenerStream::new(listener);

    notifier.ready()?;

    let service = VmServiceImpl::new(vm_handle);

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigchld = signal(SignalKind::child())?;

    let server = Server::builder()
        .add_service(proto::vm::vm_service_server::VmServiceServer::new(service))
        .serve_with_incoming_shutdown(stream, async {
            tokio::select! {
                _ = sigterm.recv() => {
                    println!("Received SIGTERM, shutting down");
                }
                _ = sigint.recv() => {
                    println!("Received SIGINT, shutting down");
                }
            }
        });

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
                kmsg::error!(@ "vmd", "Server error: {}", e);
                return Err(e.into());
            }
        }
    }

    sigchld_handle.abort();
    scrub_handle.abort();
    notifier.stopping("Graceful shutdown")?;
    println!("Shutdown complete");

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
    println!("Set as child subreaper");
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
