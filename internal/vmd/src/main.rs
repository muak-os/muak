mod actor;
mod clients;
mod grpc;
mod hypervisor;

use anyhow::Result;
use notify::{Health, NotifyClient};
use std::path::Path;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

use actor::start_vm_actor;
use clients::NetworkClient;
use grpc::VmServiceImpl;

pub mod proto {
    pub mod vm {
        tonic::include_proto!("muak.internal.vm");
    }
    pub mod network {
        tonic::include_proto!("muak.internal.network");
    }
}

const SOCKET_PATH: &str = "/run/vmd.sock";
const NETWORKD_SOCKET: &str = "/run/networkd.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::info!(@ "vmd", "Starting vmd");

    set_child_subreaper()?;

    let notifier = NotifyClient::new("vmd")?;
    notifier.status("Initializing VM daemon", Health::Healthy)?;

    let network_client = NetworkClient::connect(NETWORKD_SOCKET).await?;
    kmsg::info!(@ "vmd", "Connected to networkd");

    let vm_handle = start_vm_actor(network_client).await;

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

    let notifier_clone = NotifyClient::new("vmd")?;

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                kmsg::error!(@ "vmd", "Server error: {}", e);
                return Err(e.into());
            }
        }
    }

    sigchld_handle.abort();
    notifier_clone.stopping("Graceful shutdown")?;
    kmsg::info!(@ "vmd", "Shutdown complete");

    Ok(())
}

fn set_child_subreaper() -> Result<()> {
    use nix::libc;
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

fn reap_children() {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::Pid;

    loop {
        match waitpid(
            Pid::from_raw(-1),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
        ) {
            Ok(WaitStatus::Exited(pid, status)) => {
                kmsg::info!(@ "vmd", "Child {} exited with status {}", pid, status);
            }
            Ok(WaitStatus::Signaled(pid, signal, _)) => {
                kmsg::info!(@ "vmd", "Child {} killed by signal {:?}", pid, signal);
            }
            Ok(WaitStatus::StillAlive) | Err(_) => break,
            Ok(_) => continue,
        }
    }
}
