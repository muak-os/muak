use std::pin::Pin;

use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tonic::{Request, Response, Status};

use crate::actor::NetworkActorHandle;
use crate::model::{ConnectivityStatus, NetworkStateKind};
use crate::proto::network_service_server::NetworkService;
use crate::proto::*;
use crate::services::tap::{format_mac_address, generate_mac_address};

pub struct NetworkServiceImpl {
    handle: NetworkActorHandle,
}

impl NetworkServiceImpl {
    pub fn new(handle: NetworkActorHandle) -> Self {
        Self { handle }
    }
}

#[tonic::async_trait]
impl NetworkService for NetworkServiceImpl {
    async fn initialize(
        &self,
        _request: Request<InitializeRequest>,
    ) -> Result<Response<InitializeResponse>, Status> {
        self.handle
            .initialize_with_retry()
            .await
            .map_err(|e| Status::internal(format!("Failed to initialize network: {}", e)))?;

        Ok(Response::new(InitializeResponse {}))
    }

    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<NetworkStatus>, Status> {
        let snapshot = self.handle.snapshot().await;
        let status = snapshot_to_status(&snapshot);
        Ok(Response::new(status))
    }

    async fn create_tap(
        &self,
        request: Request<CreateTapRequest>,
    ) -> Result<Response<CreateTapResponse>, Status> {
        let req = request.into_inner();
        let tap_name = if req.name.is_empty() {
            format!("tap-{}", &req.vm_id[..8.min(req.vm_id.len())])
        } else {
            req.name
        };

        let iface = self
            .handle
            .add_tap(tap_name.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to create TAP device: {}", e)))?;

        let mac = generate_mac_address(&req.vm_id);

        Ok(Response::new(CreateTapResponse {
            interface_name: iface.name,
            mac_address: format_mac_address(&mac),
            interface_index: iface.index,
        }))
    }

    async fn delete_tap(
        &self,
        request: Request<DeleteTapRequest>,
    ) -> Result<Response<DeleteTapResponse>, Status> {
        let req = request.into_inner();

        self.handle
            .delete_tap(req.name.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to delete TAP device: {}", e)))?;

        Ok(Response::new(DeleteTapResponse {}))
    }

    async fn setup_bridge(
        &self,
        _request: Request<SetupBridgeRequest>,
    ) -> Result<Response<SetupBridgeResponse>, Status> {
        self.handle
            .setup_bridge()
            .await
            .map_err(|e| Status::internal(format!("Failed to setup bridge: {}", e)))?;

        Ok(Response::new(SetupBridgeResponse {}))
    }

    async fn check_connectivity(
        &self,
        _request: Request<CheckConnectivityRequest>,
    ) -> Result<Response<ConnectivityResult>, Status> {
        let result = self.handle.check_connectivity().await;

        let snapshot = self.handle.snapshot().await;
        let (primary_ip, gateway) = snapshot
            .interfaces
            .iter()
            .find(|i| Some(&i.name) == snapshot.primary.as_ref())
            .and_then(|i| i.ip.as_ref())
            .map(|ip| {
                (
                    ip.address.to_string(),
                    ip.gateway.map(|g| g.to_string()).unwrap_or_default(),
                )
            })
            .unwrap_or_default();

        Ok(Response::new(ConnectivityResult {
            dns_works: result.dns_ok,
            internet_reachable: result.https_ok,
            primary_ip,
            gateway,
        }))
    }

    type SubscribeStatusStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<NetworkStatus, Status>> + Send>>;

    async fn subscribe_status(
        &self,
        _request: Request<SubscribeStatusRequest>,
    ) -> Result<Response<Self::SubscribeStatusStream>, Status> {
        let watch_rx = self.handle.subscribe();

        let stream =
            WatchStream::from_changes(watch_rx).map(|snapshot| Ok(snapshot_to_status(&snapshot)));

        Ok(Response::new(Box::pin(stream)))
    }
}

fn snapshot_to_status(snapshot: &crate::model::NetworkSnapshot) -> NetworkStatus {
    let state = match snapshot.state {
        NetworkStateKind::Uninitialized | NetworkStateKind::Initializing => State::Initializing,
        NetworkStateKind::Operational | NetworkStateKind::Ready => {
            if snapshot.connectivity.status == ConnectivityStatus::Connected {
                State::Ready
            } else {
                State::Degraded
            }
        }
        NetworkStateKind::Degraded => State::Degraded,
    };

    let interfaces = snapshot
        .interfaces
        .iter()
        .map(|iface| {
            let addresses = iface
                .ip
                .as_ref()
                .map(|ip| vec![format!("{}/{}", ip.address, ip.prefix_len)])
                .unwrap_or_default();

            InterfaceInfo {
                name: iface.name.clone(),
                mac: format_mac_address(&iface.mac),
                addresses,
                has_gateway: iface
                    .ip
                    .as_ref()
                    .map(|ip| ip.gateway.is_some())
                    .unwrap_or(false),
            }
        })
        .collect();

    NetworkStatus {
        state: state.into(),
        primary_interface: snapshot.primary.clone().unwrap_or_default(),
        interfaces,
    }
}
