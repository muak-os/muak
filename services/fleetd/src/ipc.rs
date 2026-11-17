//! RPC adapters behind apid and a separately confined local publisher socket.

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;

use fleet_api::fleet::fleet_service_server::FleetService;
use fleet_api::fleet::local_inventory_service_server::LocalInventoryService;
use fleet_api::fleet::{
    ApproveNodeRequest, GetAgentStatusRequest, GetAgentStatusResponse, GetInfoRequest,
    GetInfoResponse, ListNodesRequest, ListNodesResponse, NodeRecord, NodeSnapshot,
    RegisterNodeRequest, RegisterNodeResponse, ReportInventoryRequest, ReportInventoryResponse,
    RevokeNodeRequest, RotateCredentialRequest,
};
use tonic::{Request, Response, Status};

use crate::agent::Shared;
use crate::manager::Manager;

#[derive(Clone)]
pub(crate) struct Service {
    pub(crate) manager: Option<Arc<Mutex<Manager>>>,
    pub(crate) agent: Option<Arc<Shared>>,
}

impl Service {
    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Manager) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<Response<T>, Status> {
        let manager = Arc::clone(
            self.manager
                .as_ref()
                .ok_or_else(|| Status::failed_precondition("not a manager"))?,
        );
        let value = tokio::task::spawn_blocking(move || {
            let mut manager = manager
                .lock()
                .map_err(|_error| anyhow::anyhow!("manager lock poisoned"))?;
            operation(&mut manager)
        })
        .await
        .map_err(|_error| Status::unavailable("manager task failed"))?
        .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(value))
    }
}

#[tonic::async_trait]
impl FleetService for Service {
    async fn get_info(
        &self,
        request: Request<GetInfoRequest>,
    ) -> Result<Response<GetInfoResponse>, Status> {
        actor(&request)?;
        self.call(|manager| manager.info()).await
    }
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let actor = actor(&request)?;
        let request = request.into_inner();
        self.call(move |manager| manager.register(&actor, request))
            .await
    }
    async fn report_inventory(
        &self,
        request: Request<ReportInventoryRequest>,
    ) -> Result<Response<ReportInventoryResponse>, Status> {
        let actor = actor(&request)?;
        let request = request.into_inner();
        self.call(move |manager| {
            manager.report(&actor, request)?;
            Ok(ReportInventoryResponse {})
        })
        .await
    }
    async fn list_nodes(
        &self,
        request: Request<ListNodesRequest>,
    ) -> Result<Response<ListNodesResponse>, Status> {
        actor(&request)?;
        self.call(|manager| {
            Ok(ListNodesResponse {
                nodes: manager.list()?,
            })
        })
        .await
    }
    async fn approve_node(
        &self,
        request: Request<ApproveNodeRequest>,
    ) -> Result<Response<NodeRecord>, Status> {
        let actor = actor(&request)?;
        let request = request.into_inner();
        self.call(move |manager| manager.approve(&actor, &request.node_id, &request.fingerprint))
            .await
    }
    async fn revoke_node(
        &self,
        request: Request<RevokeNodeRequest>,
    ) -> Result<Response<NodeRecord>, Status> {
        let actor = actor(&request)?;
        let request = request.into_inner();
        self.call(move |manager| manager.revoke(&actor, &request.node_id))
            .await
    }
    async fn rotate_credential(
        &self,
        request: Request<RotateCredentialRequest>,
    ) -> Result<Response<NodeRecord>, Status> {
        let actor = actor(&request)?;
        let request = request.into_inner();
        self.call(move |manager| manager.rotate(&actor, &request.node_id, &request.fingerprint))
            .await
    }
    async fn get_agent_status(
        &self,
        request: Request<GetAgentStatusRequest>,
    ) -> Result<Response<GetAgentStatusResponse>, Status> {
        actor(&request)?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("not an agent"))?;
        Ok(Response::new(agent.status.lock().await.clone()))
    }

    async fn renew_agent_credential(
        &self,
        request: Request<GetAgentStatusRequest>,
    ) -> Result<Response<GetAgentStatusResponse>, Status> {
        actor(&request)?;
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("not an agent"))?;
        agent.renewal.store(true, Ordering::Release);
        let mut status = agent.status.lock().await.clone();
        "renewal_requested".clone_into(&mut status.phase);
        Ok(Response::new(status))
    }
}

#[derive(Clone)]
pub(crate) struct Publisher(pub(crate) Arc<Shared>);

#[tonic::async_trait]
impl LocalInventoryService for Publisher {
    async fn publish(
        &self,
        request: Request<NodeSnapshot>,
    ) -> Result<Response<ReportInventoryResponse>, Status> {
        let snapshot = crate::convert::from_wire(request.into_inner())
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        *self.0.observation.lock().await = Some((snapshot, Instant::now()));
        Ok(Response::new(ReportInventoryResponse {}))
    }
}
fn actor<T>(request: &Request<T>) -> Result<String, Status> {
    let value = request
        .metadata()
        .get(fleet_api::PEER_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| Status::unauthenticated("verified gateway identity required"))?;
    crate::model::fingerprint(value)
        .map_err(|_error| Status::unauthenticated("invalid gateway identity"))?;
    Ok(value.to_owned())
}
