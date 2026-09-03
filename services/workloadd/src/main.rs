//! Workload daemon for Muak - Manages virtual machines and their life cycle.

extern crate alloc;

mod actor;
mod disk;
mod hypervisor;
mod ipc;
mod persistence;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use actor::start_vm_actor;
use anyhow::{Context as _, Result};
use config::InterfaceKind;
use granola::runtime::{notify::Health, signal::shutdown, socket::socket};
use ipc::VmServiceImpl;
use rustix::process::{WaitOptions, wait};
use tokio::fs::create_dir_all;
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[expect(
    clippy::absolute_paths,
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    clippy::as_conversions,
    clippy::clone_on_ref_ptr,
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::doc_paragraphs_missing_punctuation,
    clippy::empty_structs_with_brackets,
    clippy::excessive_nesting,
    clippy::impl_trait_in_params,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::pattern_type_mismatch,
    clippy::std_instead_of_core,
    clippy::too_many_lines,
    reason = "generated protobuf code"
)]
pub mod proto {
    pub mod vm {
        tonic::include_proto!("muak.vm.v1");
    }
}

const STATE_DIR: &str = "/run/state/workloadd";

/// Entry point for the VM daemon.
#[granola::service("workloadd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;
    set_child_subreaper()?;

    notifier.status("Initializing workload daemon", Health::Healthy)?;

    let kvm_available = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok();
    if !kvm_available {
        kmsg::warn!(@ "workloadd", "/dev/kvm is not available, entering degraded mode");
        notifier.status("KVM not available", Health::Degraded)?;
    }

    create_dir_all(format!("{STATE_DIR}/vms")).await?;

    let (connection, netlink_handle, _) = rtnetlink::new_connection()?;
    tokio::spawn(connection);

    let bridge_name = config::network()
        .interfaces
        .iter()
        .find(|i| i.kind == InterfaceKind::Bridge)
        .map(|i| i.name.clone())
        .context("no bridge interface found in network config")?;

    let vm_handle = start_vm_actor(netlink_handle, bridge_name, kvm_available).await;

    let stream = UnixListenerStream::new(socket()?);

    notifier.ready()?;

    let service = VmServiceImpl::new(vm_handle);

    let mut sigchld = signal(SignalKind::child())?;

    let server = Server::builder()
        .add_service(proto::vm::vm_service_server::VmServiceServer::new(service))
        .serve_with_incoming_shutdown(stream, shutdown());

    let sigchld_handle = tokio::spawn(async move {
        while sigchld.recv().await.is_some() {
            reap_children();
        }
    });

    let scrub_shutdown = Arc::new(AtomicBool::new(false));
    let scrub_handle = tokio::spawn(disk::scrub::timer(Arc::clone(&scrub_shutdown)));

    if let Err(e) = server.await {
        kmsg::error!("Server error: {e}");
        return Err(e.into());
    }

    scrub_shutdown.store(true, Ordering::Relaxed);
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
            Ok(Some(_)) => {}
        }
    }
}
