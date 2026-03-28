use std::pin::Pin;

use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tonic::{Request, Response, Status};

use crate::actor::NetworkActorHandle;
use crate::model::NetworkStateKind;
use crate::netutil::{format_mac_address, generate_mac_address};
use crate::proto::network_service_server::NetworkService;
use crate::proto::*;

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
        NetworkStateKind::Operational | NetworkStateKind::Ready => State::Ready,
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

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use super::*;
    use crate::model::{
        InterfaceSnapshot, IpConfig, LinkStateKind, NetworkSnapshot, NetworkStateKind,
    };

    fn empty_snapshot() -> NetworkSnapshot {
        NetworkSnapshot::empty()
    }

    fn make_iface(name: &str, ip: Option<IpConfig>) -> Arc<InterfaceSnapshot> {
        Arc::new(InterfaceSnapshot {
            name: name.to_string(),
            index: 1,
            mac: [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
            link: LinkStateKind::Up,
            ip,
            lease: None,
            ipv6: None,
        })
    }

    #[test]
    fn uninitialized_maps_to_initializing() {
        // ARRANGE
        let snap = empty_snapshot();

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.state, i32::from(State::Initializing));
    }

    #[test]
    fn initializing_maps_to_initializing() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.state = NetworkStateKind::Initializing;

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.state, i32::from(State::Initializing));
    }

    #[test]
    fn operational_maps_to_ready() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.state = NetworkStateKind::Operational;

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.state, i32::from(State::Ready));
    }

    #[test]
    fn ready_maps_to_ready() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.state = NetworkStateKind::Ready;

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.state, i32::from(State::Ready));
    }

    #[test]
    fn degraded_maps_to_degraded() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.state = NetworkStateKind::Degraded;

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.state, i32::from(State::Degraded));
    }

    #[test]
    fn primary_interface_from_snapshot() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.primary = Some("eth0".to_string());

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.primary_interface, "eth0");
    }

    #[test]
    fn no_primary_defaults_to_empty() {
        // ARRANGE
        let snap = empty_snapshot();

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.primary_interface, "");
    }

    #[test]
    fn interface_with_ip_has_address_and_gateway() {
        // ARRANGE
        let mut snap = empty_snapshot();
        let ip = IpConfig {
            address: Ipv4Addr::new(10, 0, 0, 5),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
            dns: vec![],
        };
        snap.interfaces.push(make_iface("eth0", Some(ip)));

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.interfaces.len(), 1);
        assert_eq!(status.interfaces[0].name, "eth0");
        assert_eq!(status.interfaces[0].addresses, vec!["10.0.0.5/24"]);
        assert!(status.interfaces[0].has_gateway);
    }

    #[test]
    fn interface_without_ip_has_empty_addresses() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.interfaces.push(make_iface("eth0", None));

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert!(status.interfaces[0].addresses.is_empty());
        assert!(!status.interfaces[0].has_gateway);
    }

    #[test]
    fn interface_mac_formatted() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.interfaces.push(make_iface("eth0", None));

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.interfaces[0].mac, "02:00:00:00:00:01");
    }

    #[test]
    fn multiple_interfaces() {
        // ARRANGE
        let mut snap = empty_snapshot();
        snap.interfaces.push(make_iface("eth0", None));
        snap.interfaces.push(make_iface("eth1", None));

        // ACT
        let status = snapshot_to_status(&snap);

        // ASSERT
        assert_eq!(status.interfaces.len(), 2);
    }
}
