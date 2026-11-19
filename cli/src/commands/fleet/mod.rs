//! Optional fleet inspection through the existing operator context.

mod inventory;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use fleet_api::fleet::fleet_service_client::FleetServiceClient;
use fleet_api::fleet::{
    ApproveNodeRequest, GetAgentStatusRequest, GetInfoRequest, ListNodesRequest, RevokeNodeRequest,
    RotateCredentialRequest,
};
use tonic::transport::Channel;

/// Fleet inventory and explicit enrollment actions.
#[derive(Subcommand, Debug, Clone)]
pub enum Action {
    /// Show the manager's domain and public trust anchor.
    Info,
    /// List pending, approved and revoked hosts, including freshness.
    Nodes {
        #[command(flatten)]
        options: Options,
        /// Filter by enrollment status.
        #[arg(long, value_parser = ["pending", "approved", "revoked"])]
        status: Option<String>,
    },
    /// List observed VMs, qualified by node and freshness.
    Vms {
        #[command(flatten)]
        options: Options,
        /// Filter by observed VM state.
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(inventory::VM_STATES))]
        state: Option<String>,
    },
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

/// Filters and output format for fleet lists.
#[derive(Args, Clone, Debug)]
pub struct Options {
    /// Select the exact node UUID from `fleet nodes`.
    #[arg(long, value_name = "UUID")]
    node: Option<String>,
    /// Filter by observation freshness.
    #[arg(long, value_parser = ["fresh", "stale"])]
    freshness: Option<String>,
    /// Choose table output for operators or JSON for scripts.
    #[arg(short, long, value_enum, default_value_t = Format::Table)]
    output: Format,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Table,
    Json,
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
        Action::Nodes { options, status } => {
            list(
                &mut client,
                &options,
                inventory::View::Nodes(status.as_deref()),
            )
            .await?;
        }
        Action::Vms { options, state } => {
            list(
                &mut client,
                &options,
                inventory::View::Vms(state.as_deref()),
            )
            .await?;
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

async fn list(
    client: &mut FleetServiceClient<Channel>,
    options: &Options,
    view: inventory::View<'_>,
) -> Result<()> {
    let response = client.list_nodes(ListNodesRequest {}).await?.into_inner();
    inventory::write(
        &mut std::io::stdout().lock(),
        &response.nodes,
        options,
        view,
    )
}
