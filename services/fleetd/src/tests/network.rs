//! Real mTLS, proxy, RBAC and agent loop; CSR service is a controlled fixture.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use apid::{handler, proxy, rbac, tls};
use config::auth::{State, User};
use config::permission::Permission;
use config::system::fleet::Config;
use der::EncodePem as _;
use der::pem::LineEnding;
use fleet_api::auth::auth_service_server::{AuthService, AuthServiceServer};
use fleet_api::auth::{
    AckEnrollmentRequest, AckEnrollmentResponse, ApproveCsrRequest, ApproveCsrResponse,
    GetCsrStatusRequest, GetCsrStatusResponse, ListPendingCsrsRequest, ListPendingCsrsResponse,
    ListUsersRequest, ListUsersResponse, RevokeCertRequest, RevokeCertResponse, SubmitCsrRequest,
    SubmitCsrResponse,
};
use fleet_api::fleet::fleet_service_client::FleetServiceClient;
use fleet_api::fleet::fleet_service_server::FleetServiceServer;
use fleet_api::fleet::local_inventory_service_client::LocalInventoryServiceClient;
use fleet_api::fleet::local_inventory_service_server::LocalInventoryServiceServer;
use fleet_api::fleet::{
    ApproveNodeRequest, GetInfoRequest, ListNodesRequest, NodeSnapshot, RegisterNodeRequest,
    VmSummary,
};
use fleet_api::unix::Connector;
use hyper::Request as HttpRequest;
use hyper::body::Incoming;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pki::{cert, csr, pem};
use tempfile::{TempDir, tempdir};
use tokio::net::{TcpListener, UnixListener};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity, Server};
use tonic::{Request, Response, Status};
use uuid::Uuid;
use x509_cert::Certificate as X509Certificate;

use crate::agent::Agent;
use crate::ipc::{Publisher, Service};
use crate::manager::Manager;

#[derive(Clone, Default)]
struct EnrollmentFixture(Arc<Mutex<BTreeMap<String, (String, String)>>>);

#[tonic::async_trait]
impl AuthService for EnrollmentFixture {
    async fn submit_csr(
        &self,
        request: Request<SubmitCsrRequest>,
    ) -> Result<Response<SubmitCsrResponse>, Status> {
        let csr = request.into_inner().csr_pem;
        let fingerprint = csr::compute_fingerprint(&csr).unwrap();
        self.0
            .lock()
            .unwrap()
            .insert(fingerprint.clone(), (csr, String::new()));
        Ok(Response::new(SubmitCsrResponse { fingerprint }))
    }
    async fn get_csr_status(
        &self,
        request: Request<GetCsrStatusRequest>,
    ) -> Result<Response<GetCsrStatusResponse>, Status> {
        let records = self.0.lock().unwrap();
        let record = records.get(&request.into_inner().fingerprint);
        let (status, cert_pem) = record.map_or_else(
            || (3_i32, String::new()),
            |entry| (i32::from(!entry.1.is_empty()), entry.1.clone()),
        );
        Ok(Response::new(GetCsrStatusResponse {
            status,
            cert_pem,
            ca_pem: String::new(),
            server_name: String::new(),
        }))
    }
    async fn ack_enrollment(
        &self,
        request: Request<AckEnrollmentRequest>,
    ) -> Result<Response<AckEnrollmentResponse>, Status> {
        self.0
            .lock()
            .unwrap()
            .remove(&request.into_inner().fingerprint);
        Ok(Response::new(AckEnrollmentResponse {}))
    }
    async fn approve_csr(
        &self,
        _request: Request<ApproveCsrRequest>,
    ) -> Result<Response<ApproveCsrResponse>, Status> {
        Err(Status::unimplemented("fixture uses explicit test approval"))
    }
    async fn revoke_cert(
        &self,
        _request: Request<RevokeCertRequest>,
    ) -> Result<Response<RevokeCertResponse>, Status> {
        Err(Status::unimplemented("fixture"))
    }
    async fn list_users(
        &self,
        _request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        Err(Status::unimplemented("fixture"))
    }
    async fn list_pending_csrs(
        &self,
        _request: Request<ListPendingCsrsRequest>,
    ) -> Result<Response<ListPendingCsrsResponse>, Status> {
        Err(Status::unimplemented("fixture"))
    }
}

struct Harness {
    _dir: TempDir,
    tasks: Vec<JoinHandle<()>>,
    endpoint: String,
    ca_pem: String,
    ca_key: String,
    ca: X509Certificate,
    enrollment: EnrollmentFixture,
    auth: Arc<RwLock<State>>,
    manager: Arc<Mutex<Manager>>,
    admin: Channel,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Harness {
    async fn start() -> Self {
        let dir = tempdir().unwrap();
        let (ca_key, ca) = cert::generate_ca("fleet-test").unwrap();
        let (server_key, server) = cert::generate_server("muak-server", &ca_key, &ca).unwrap();
        let ca_pem = ca.to_pem(LineEnding::LF).unwrap();
        let ca_key = pem::encode_pkcs8(ca_key.pkcs8_der()).unwrap();
        fs::write(dir.path().join("ca"), &ca_pem).unwrap();
        fs::write(
            dir.path().join("server"),
            server.to_pem(LineEnding::LF).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("key"),
            pem::encode_pkcs8(server_key.pkcs8_der()).unwrap(),
        )
        .unwrap();
        let acceptor = tls::load_tls_config_with_paths(
            dir.path().join("ca").to_str().unwrap(),
            dir.path().join("server").to_str().unwrap(),
            dir.path().join("key").to_str().unwrap(),
        )
        .unwrap();
        let manager = Arc::new(Mutex::new(
            Manager::open(&dir.path().join("state"), 16, 60, ca_pem.clone()).unwrap(),
        ));
        let service = Service {
            manager: Some(Arc::clone(&manager)),
            agent: None,
        };
        let socket = dir.path().join("backend.sock");
        let uds = UnixListener::bind(&socket).unwrap();
        let pool = Arc::new(proxy::BackendPool::from_socket(socket.to_str().unwrap()));
        let enrollment = EnrollmentFixture::default();
        let backend = Server::builder()
            .add_service(FleetServiceServer::new(service))
            .add_service(AuthServiceServer::new(enrollment.clone()))
            .serve_with_incoming(UnixListenerStream::new(uds));
        let backend_task = tokio::spawn(async move {
            backend.await.unwrap();
        });
        let auth = Arc::new(RwLock::new(State::default()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("https://{}", listener.local_addr().unwrap());
        let gateway = tokio::spawn(gateway(listener, acceptor, pool, Arc::clone(&auth), socket));
        let (admin_key, admin_csr) = csr::generate("operator").unwrap();
        let (admin_cert, fingerprint) = csr::sign(&admin_csr, &ca_key, &ca).unwrap();
        auth.write().unwrap().users.push(User {
            fingerprint,
            permissions: vec![Permission::Admin],
        });
        let admin = connect(
            &endpoint,
            &ca_pem,
            Some((admin_cert.to_pem(LineEnding::LF).unwrap(), admin_key)),
        )
        .await;
        Self {
            _dir: dir,
            tasks: vec![backend_task, gateway],
            endpoint,
            ca_pem,
            ca_key,
            ca,
            enrollment,
            auth,
            manager,
            admin,
        }
    }

    async fn enrolled_agent(&self, path: &Path) -> Agent {
        let config = Config::Agent {
            manager: self.endpoint.clone(),
            domain: self.manager.lock().unwrap().info().unwrap().domain_id,
            ca_pem: self.ca_pem.clone(),
            server_name: "muak-server".to_owned(),
            report_interval_secs: 1,
        };
        let mut agent = Agent::open(path, &config, "worker".to_owned()).unwrap();
        agent.tick().await.unwrap();
        assert_eq!(
            agent.shared.status.lock().await.phase,
            "pending_certificate"
        );
        let fingerprint = agent.shared.status.lock().await.csr_fingerprint.clone();
        self.approve_certificate(&fingerprint);
        agent.tick().await.unwrap();
        assert_eq!(agent.shared.status.lock().await.phase, "certificate_ready");
        agent.tick().await.unwrap();
        assert_eq!(agent.shared.status.lock().await.phase, "pending_node");
        agent
    }
    fn approve_certificate(&self, fingerprint: &str) {
        let fingerprint = {
            let mut records = self.enrollment.0.lock().unwrap();
            let entry = records.get_mut(fingerprint).unwrap();
            let (certificate, fingerprint) = csr::sign(&entry.0, &self.ca_key, &self.ca).unwrap();
            entry.1 = certificate.to_pem(LineEnding::LF).unwrap();
            fingerprint
        };
        self.auth.write().unwrap().users.push(User {
            fingerprint,
            permissions: vec![Permission::FleetReport],
        });
    }

    fn agent_config(&self) -> Config {
        Config::Agent {
            manager: self.endpoint.clone(),
            domain: self.manager.lock().unwrap().info().unwrap().domain_id,
            ca_pem: self.ca_pem.clone(),
            server_name: "muak-server".to_owned(),
            report_interval_secs: 1,
        }
    }
}

async fn connect(endpoint: &str, ca: &str, identity: Option<(String, String)>) -> Channel {
    let mut tls = ClientTlsConfig::new()
        .domain_name("muak-server")
        .ca_certificate(Certificate::from_pem(ca));
    if let Some((certificate, key)) = identity {
        tls = tls.identity(Identity::from_pem(certificate, key));
    }
    Endpoint::from_shared(endpoint.to_owned())
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn two_agents_enroll_publish_and_reconnect_over_mtls() {
    // ARRANGE: real apid TLS, RBAC and proxy; signing uses Muak PKI.
    let mut harness = Harness::start().await;
    let workers = tempdir().unwrap();
    let mut first = harness
        .enrolled_agent(&workers.path().join("one/state"))
        .await;
    let mut second = harness
        .enrolled_agent(&workers.path().join("two/state"))
        .await;
    let mut operator = FleetServiceClient::new(harness.admin.clone());
    let nodes = operator
        .list_nodes(ListNodesRequest {})
        .await
        .unwrap()
        .into_inner()
        .nodes;
    assert_eq!(nodes.len(), 2);
    // ACT: two explicit approvals, then local publication through the isolated RPC.
    for node in nodes {
        operator
            .approve_node(ApproveNodeRequest {
                node_id: node.node_id,
                fingerprint: node.fingerprint,
            })
            .await
            .unwrap();
    }
    let socket = workers.path().join("publisher.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let publisher = Server::builder()
        .add_service(LocalInventoryServiceServer::new(Publisher(Arc::clone(
            &first.shared,
        ))))
        .serve_with_incoming(UnixListenerStream::new(listener));
    harness.tasks.push(tokio::spawn(async move {
        publisher.await.unwrap();
    }));
    let channel = Endpoint::from_static("http://localhost")
        .connect_with_connector(Connector(PathBuf::from(&socket)))
        .await
        .unwrap();
    LocalInventoryServiceClient::new(channel)
        .publish(NodeSnapshot {
            hostname: "worker".to_owned(),
            vms: vec![VmSummary {
                vm_id: Uuid::new_v4().to_string(),
                name: "guest".to_owned(),
                state: 3,
            }],
        })
        .await
        .unwrap();
    first.tick().await.unwrap();
    second.tick().await.unwrap(); // No local observation must remain stale.
    // ASSERT: actual VM report, no fabricated empty inventory, stable identity.
    let nodes = operator
        .list_nodes(ListNodesRequest {})
        .await
        .unwrap()
        .into_inner()
        .nodes;
    assert_eq!(nodes.iter().filter(|node| !node.stale).count(), 1);
    assert_eq!(
        nodes
            .iter()
            .filter_map(|node| node.snapshot.as_ref())
            .flat_map(|snapshot| &snapshot.vms)
            .count(),
        1
    );
    let old_id = first.shared.status.lock().await.node_id.clone();
    drop(first);
    let config = Config::Agent {
        manager: harness.endpoint.clone(),
        domain: harness.manager.lock().unwrap().info().unwrap().domain_id,
        ca_pem: harness.ca_pem.clone(),
        server_name: "muak-server".to_owned(),
        report_interval_secs: 1,
    };
    let mut first = Agent::open(
        &workers.path().join("one/state"),
        &config,
        "worker".to_owned(),
    )
    .unwrap();
    assert_eq!(first.shared.status.lock().await.node_id, old_id);
    first.tick().await.unwrap();
    assert_eq!(first.shared.status.lock().await.phase, "waiting_inventory");
    assert_eq!(
        operator
            .list_nodes(ListNodesRequest {})
            .await
            .unwrap()
            .into_inner()
            .nodes
            .len(),
        2
    );
    assert!(harness.enrollment.0.lock().unwrap().is_empty()); // Durable enrollment acknowledged.
}

#[tokio::test]
async fn forged_identity_and_wrong_permissions_are_rejected() {
    // ARRANGE
    let harness = Harness::start().await;
    let workers = tempdir().unwrap();
    let agent = harness.enrolled_agent(&workers.path().join("state")).await;
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(workers.path().join("state")).unwrap()).unwrap();
    let material = (
        state["certificate"].as_str().unwrap().to_owned(),
        state["key"].as_str().unwrap().to_owned(),
    );
    let channel = connect(&harness.endpoint, &harness.ca_pem, Some(material)).await;
    let mut worker = FleetServiceClient::new(channel);
    // ACT: a report-only peer cannot approve or list, even with forged metadata.
    assert_eq!(
        worker
            .list_nodes(ListNodesRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let domain_id = harness.manager.lock().unwrap().info().unwrap().domain_id;
    let mut forged = Request::new(RegisterNodeRequest {
        domain_id,
        node_id: agent.shared.status.lock().await.node_id.clone(),
        hostname: "worker".to_owned(),
    });
    forged
        .metadata_mut()
        .insert(fleet_api::PEER_HEADER, "ff".repeat(32).parse().unwrap());
    assert!(
        worker
            .register_node(forged)
            .await
            .unwrap()
            .into_inner()
            .pending
    );
    // ASSERT: the pending binding still belongs to the TLS certificate.
    let nodes = harness.manager.lock().unwrap().list().unwrap();
    assert_ne!(nodes.first().unwrap().fingerprint, "ff".repeat(32));
    let anonymous = connect(&harness.endpoint, &harness.ca_pem, None).await;
    assert_eq!(
        FleetServiceClient::new(anonymous)
            .get_info(GetInfoRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
}

async fn gateway(
    listener: TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
    pool: Arc<proxy::BackendPool>,
    auth: Arc<RwLock<State>>,
    socket: PathBuf,
) {
    while let Ok((stream, _peer)) = listener.accept().await {
        tokio::spawn(connection(
            acceptor.clone(),
            Arc::clone(&pool),
            Arc::clone(&auth),
            socket.clone(),
            stream,
        ));
    }
}

async fn connection(
    acceptor: tokio_rustls::TlsAcceptor,
    pool: Arc<proxy::BackendPool>,
    auth: Arc<RwLock<State>>,
    socket: PathBuf,
    stream: tokio::net::TcpStream,
) {
    let Ok(tls) = acceptor.accept(stream).await else {
        return;
    };
    let fingerprint = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|cert| cert::compute_fingerprint_der(cert));
    let service = service_fn(move |request: HttpRequest<Incoming>| {
        forward_fixture(
            Arc::clone(&pool),
            Arc::clone(&auth),
            socket.clone(),
            fingerprint.clone(),
            request,
        )
    });
    let _connection = http2::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(tls), service)
        .await;
}

async fn forward_fixture(
    pool: Arc<proxy::BackendPool>,
    auth: Arc<RwLock<State>>,
    socket: PathBuf,
    fingerprint: Option<String>,
    mut request: HttpRequest<Incoming>,
) -> anyhow::Result<hyper::Response<handler::Body>> {
    let access = rbac::check_access_with_state(
        request.uri().path(),
        fingerprint.as_deref(),
        &auth.read().unwrap(),
    );
    if let Err(error) = access {
        return handler::grpc_error(error.grpc_status_code(), &error.grpc_message());
    }
    handler::set_fleet_identity(&mut request, fingerprint.as_deref())?;
    proxy::forward(&pool, request, socket.to_str().unwrap())
        .await
        .map(|response| response.map(http_body_util::Either::Right))
}

#[tokio::test]
async fn credential_renewal_preserves_identity_and_requires_new_binding() {
    // ARRANGE
    use fleet_api::fleet::fleet_service_server::FleetService as _;
    use fleet_api::fleet::{GetAgentStatusRequest, RotateCredentialRequest};
    let harness = Harness::start().await;
    let dir = tempdir().unwrap();
    let mut agent = harness.enrolled_agent(&dir.path().join("state")).await;
    let old = agent.shared.status.lock().await.clone();
    let mut operator = FleetServiceClient::new(harness.admin.clone());
    operator
        .approve_node(ApproveNodeRequest {
            node_id: old.node_id.clone(),
            fingerprint: old.certificate_fingerprint.clone(),
        })
        .await
        .unwrap();
    let local = Service {
        manager: None,
        agent: Some(Arc::clone(&agent.shared)),
    };
    let mut renewal = Request::new(GetAgentStatusRequest {});
    renewal
        .metadata_mut()
        .insert(fleet_api::PEER_HEADER, "ff".repeat(32).parse().unwrap());
    // ACT
    local.renew_agent_credential(renewal).await.unwrap();
    agent.tick().await.unwrap();
    let pending = agent.shared.status.lock().await.clone();
    assert_eq!(pending.node_id, old.node_id);
    assert_ne!(pending.csr_fingerprint, old.csr_fingerprint);
    harness.approve_certificate(&pending.csr_fingerprint);
    agent.tick().await.unwrap();
    let renewed = agent.shared.status.lock().await.clone();
    agent.tick().await.unwrap_err(); // New credential cannot take over implicitly.
    operator
        .rotate_credential(RotateCredentialRequest {
            node_id: old.node_id,
            fingerprint: renewed.certificate_fingerprint.clone(),
        })
        .await
        .unwrap();
    agent.tick().await.unwrap();
    // ASSERT
    assert_ne!(renewed.certificate_fingerprint, old.certificate_fingerprint);
    assert_eq!(
        operator
            .list_nodes(ListNodesRequest {})
            .await
            .unwrap()
            .into_inner()
            .nodes
            .len(),
        1
    );
}

#[tokio::test]
async fn foreign_ca_and_revoked_certificate_fail_closed() {
    // ARRANGE
    let harness = Harness::start().await;
    let dir = tempdir().unwrap();
    let mut agent = harness.enrolled_agent(&dir.path().join("trusted")).await;
    let mut config = harness.agent_config();
    let (_, foreign) = cert::generate_ca("foreign").unwrap();
    if let Config::Agent { ref mut ca_pem, .. } = config {
        *ca_pem = foreign.to_pem(LineEnding::LF).unwrap();
    }
    let mut foreign =
        Agent::open(&dir.path().join("foreign"), &config, "worker".to_owned()).unwrap();
    // ACT
    foreign.tick().await.unwrap_err();
    let status = agent.shared.status.lock().await.clone();
    harness
        .auth
        .write()
        .unwrap()
        .revoked
        .push(status.certificate_fingerprint);
    agent.tick().await.unwrap_err();
    // ASSERT
    assert!(harness.enrollment.0.lock().unwrap().is_empty());
    assert_eq!(harness.manager.lock().unwrap().list().unwrap().len(), 1);
}
