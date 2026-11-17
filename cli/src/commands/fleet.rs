//! Optional fleet inspection through the existing operator context.

use anyhow::Result;
use clap::Subcommand;
use fleet_api::fleet::fleet_service_client::FleetServiceClient;
use fleet_api::fleet::{
    ApproveNodeRequest, GetAgentStatusRequest, GetInfoRequest, ListNodesRequest, NodeRecord,
    RevokeNodeRequest, RotateCredentialRequest,
};
use tonic::transport::Channel;

/// Fleet inventory and explicit enrollment actions.
#[derive(Subcommand, Debug, Clone)]
pub enum Action {
    /// Show the manager's domain and public trust anchor.
    Info,
    /// List pending, approved and revoked hosts, including freshness.
    Nodes,
    /// List observed VMs, qualified by node and freshness.
    Vms,
    /// Show the local agent's enrollment and reporting state.
    AgentStatus,
    /// Generate a new key and CSR on this agent while preserving its node identity.
    RenewAgentCredential,
    /// Approve an exact pending node/certificate binding.
    Approve { node: String, fingerprint: String },
    /// Revoke a node and invalidate its reporting session.
    Revoke { node: String },
    /// Replace an approved node's certificate fingerprint.
    RotateCredential { node: String, fingerprint: String },
}

pub(crate) async fn handle(channel: Channel, action: Action) -> Result<()> {
    let mut client = FleetServiceClient::new(channel);
    match action {
        Action::Info => {
            let info = client.get_info(GetInfoRequest {}).await?.into_inner();
            println!(
                "Domain: {}\nServer name: {}\nStale after: {}s\nCA:\n{}",
                info.domain_id, info.server_name, info.stale_after_secs, info.ca_pem
            );
        }
        Action::AgentStatus => {
            let status = client
                .get_agent_status(GetAgentStatusRequest {})
                .await?
                .into_inner();
            println!(
                "Node: {}\nCSR: {}\nCertificate: {}\nPhase: {}\n{}",
                status.node_id,
                status.csr_fingerprint,
                status.certificate_fingerprint,
                status.phase,
                status.detail
            );
        }
        Action::Nodes | Action::Vms => {
            let nodes = client
                .list_nodes(ListNodesRequest {})
                .await?
                .into_inner()
                .nodes;
            let vms = matches!(action, Action::Vms);
            if vms {
                println!("NODE\tVM\tNAME\tSTATE\tFRESHNESS");
            } else {
                println!("NODE\tHOST\tSTATUS\tFRESHNESS\tAGE(s)\tCERTIFICATE");
            }
            for node in nodes {
                print_node(node, vms);
            }
        }
        Action::RenewAgentCredential => {
            let status = client
                .renew_agent_credential(GetAgentStatusRequest {})
                .await?
                .into_inner();
            println!("{} {}", status.node_id, status.phase);
        }
        Action::Approve { node, fingerprint } => {
            let result = client
                .approve_node(ApproveNodeRequest {
                    node_id: node,
                    fingerprint,
                })
                .await?
                .into_inner();
            println!("{} {}", result.node_id, result.status);
        }
        Action::Revoke { node } => {
            let result = client
                .revoke_node(RevokeNodeRequest { node_id: node })
                .await?
                .into_inner();
            println!("{} {}", result.node_id, result.status);
        }
        Action::RotateCredential { node, fingerprint } => {
            let result = client
                .rotate_credential(RotateCredentialRequest {
                    node_id: node,
                    fingerprint,
                })
                .await?
                .into_inner();
            println!("{} {}", result.node_id, result.fingerprint);
        }
    }
    Ok(())
}

fn state(value: i32) -> &'static str {
    match value {
        1 => "created",
        2 => "starting",
        3 => "running",
        4 => "stopping",
        5 => "stopped",
        6 => "failed",
        _ => "unspecified",
    }
}

fn print_node(node: NodeRecord, vms: bool) {
    let freshness = if node.stale { "stale" } else { "fresh" };
    if vms {
        print_vms(node, freshness);
        return;
    }
    let age = node
        .observation_age_secs
        .map_or_else(|| "never".to_owned(), |age| age.to_string());
    println!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        node.node_id,
        node.hostname.escape_default(),
        node.status,
        freshness,
        age,
        node.fingerprint
    );
}

fn print_vms(node: NodeRecord, freshness: &str) {
    let Some(snapshot) = node.snapshot else {
        return;
    };
    for vm in snapshot.vms {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            node.node_id,
            vm.vm_id,
            vm.name.escape_default(),
            state(vm.state),
            freshness
        );
    }
}
