use tokio::sync::watch;
use tonic::{Request, Response, Status};

use super::proto::service::service_state_service_server::{
    ServiceStateService, ServiceStateServiceServer,
};
use super::proto::service::{
    ListServiceStatesRequest, ListServiceStatesResponse, ServiceState as ProtoState,
};
use crate::supervisor::status::{Snapshot, View};

pub struct ServiceImpl {
    rx: watch::Receiver<Snapshot>,
}

#[tonic::async_trait]
impl ServiceStateService for ServiceImpl {
    async fn list_service_states(
        &self,
        _request: Request<ListServiceStatesRequest>,
    ) -> Result<Response<ListServiceStatesResponse>, Status> {
        let snapshot = self.rx.borrow().clone();
        let states = snapshot.into_iter().map(to_proto_state).collect();

        Ok(Response::new(ListServiceStatesResponse { states }))
    }
}

pub fn service(rx: watch::Receiver<Snapshot>) -> ServiceStateServiceServer<ServiceImpl> {
    ServiceStateServiceServer::new(ServiceImpl { rx })
}

fn to_proto_state(view: View) -> ProtoState {
    ProtoState {
        name: view.name,
        status: view.status.to_string(),
        restarts: view.restarts,
    }
}

#[cfg(test)]
mod tests {
    use tonic::Request;

    use super::*;
    use crate::supervisor::service::ServiceStatus;
    use crate::supervisor::status;

    #[tokio::test]
    async fn list_service_states_reports_snapshot() {
        // ARRANGE
        let (tx, rx) = status::channel();
        tx.send_replace(vec![
            View {
                name: "alpha".to_owned(),
                status: ServiceStatus::Ready,
                restarts: 2,
            },
            View {
                name: "beta".to_owned(),
                status: ServiceStatus::Failed,
                restarts: 0,
            },
        ]);
        let service = ServiceImpl { rx };

        // ACT
        let response = service
            .list_service_states(Request::new(ListServiceStatesRequest {}))
            .await
            .expect("list_service_states failed");

        // ASSERT
        let states = response.into_inner().states;
        assert_eq!(states.len(), 2);
        assert_eq!(
            states.first().map(|state| state.name.as_str()),
            Some("alpha")
        );
        assert_eq!(
            states.first().map(|state| state.status.as_str()),
            Some("ready")
        );
        assert_eq!(states.first().map(|state| state.restarts), Some(2));
        assert_eq!(states.get(1).map(|state| state.name.as_str()), Some("beta"));
        assert_eq!(
            states.get(1).map(|state| state.status.as_str()),
            Some("failed")
        );
        assert_eq!(states.get(1).map(|state| state.restarts), Some(0));
    }

    #[tokio::test]
    async fn list_service_states_returns_empty_for_empty_snapshot() {
        // ARRANGE
        let (_tx, rx) = status::channel();
        let service = ServiceImpl { rx };

        // ACT
        let response = service
            .list_service_states(Request::new(ListServiceStatesRequest {}))
            .await
            .expect("list_service_states failed");

        // ASSERT
        assert_eq!(response.into_inner().states, Vec::<ProtoState>::new());
    }
}
