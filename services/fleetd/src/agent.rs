//! Outbound, explicitly trusted enrollment and read-only inventory reporting.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use config::system::fleet::Config;
use coordination::inventory::Snapshot;
use fleet_api::auth::auth_service_client::AuthServiceClient;
use fleet_api::auth::get_csr_status_response::Status as CsrStatus;
use fleet_api::auth::{AckEnrollmentRequest, GetCsrStatusRequest, SubmitCsrRequest};
use fleet_api::fleet::fleet_service_client::FleetServiceClient;
use fleet_api::fleet::{
    GetAgentStatusResponse, NodeSnapshot, RegisterNodeRequest, ReportInventoryRequest,
};
use granola::runtime::signal::shutdown;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

use crate::convert;
use crate::model::identity;
use crate::storage::Store;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    version: u32,
    node: String,
    domain: String,
    ca_pem: String,
    key: String,
    csr: String,
    certificate: Option<String>,
}

pub(crate) struct Shared {
    pub(crate) observation: Mutex<Option<(Snapshot, Instant)>>,
    pub(crate) status: Mutex<GetAgentStatusResponse>,
    pub(crate) renewal: AtomicBool,
}

pub(crate) struct Agent {
    store: Store<Credentials>,
    endpoint: String,
    server_name: String,
    hostname: String,
    interval: u64,
    channel: Option<Channel>,
    session: Option<String>,
    sequence: u64,
    pub(crate) shared: Arc<Shared>,
}

impl Agent {
    pub(crate) fn open(path: &Path, config: &Config, hostname: String) -> Result<Self> {
        let Config::Agent {
            ref manager,
            ref domain,
            ref ca_pem,
            ref server_name,
            ref report_interval_secs,
        } = *config
        else {
            bail!("agent configuration required");
        };
        let domain = identity(domain)?.to_string();
        let store = Store::open(path, || {
            let node = Uuid::new_v4().to_string();
            let (key, csr) = pki::csr::generate(&format!("muak-fleet-{node}"))?;
            Ok(Credentials {
                version: 1,
                node,
                domain: domain.clone(),
                ca_pem: ca_pem.clone(),
                key,
                csr,
                certificate: None,
            })
        })?;
        let credentials = store.data()?;
        if credentials.version != 1 || credentials.domain != domain || &credentials.ca_pem != ca_pem
        {
            bail!("agent identity is bound to different trust; explicit replacement is required");
        }
        identity(&credentials.node)?;
        let status = GetAgentStatusResponse {
            node_id: credentials.node.clone(),
            csr_fingerprint: pki::csr::compute_fingerprint(&credentials.csr)?,
            phase: "starting".to_owned(),
            detail: String::new(),
            certificate_fingerprint: credentials
                .certificate
                .as_deref()
                .map(certificate_fingerprint)
                .transpose()?
                .unwrap_or_default(),
        };
        Ok(Self {
            store,
            endpoint: manager.clone(),
            server_name: server_name.clone(),
            hostname,
            interval: *report_interval_secs,
            channel: None,
            session: None,
            sequence: 0,
            shared: Arc::new(Shared {
                observation: Mutex::new(None),
                status: Mutex::new(status),
                renewal: AtomicBool::new(false),
            }),
        })
    }

    pub(crate) async fn run(mut self) {
        loop {
            self.report_once().await;
            let jitter = u64::from(
                Uuid::new_v4()
                    .as_bytes()
                    .first()
                    .copied()
                    .unwrap_or(0)
                    .checked_rem(3)
                    .unwrap_or(0),
            );
            tokio::select! {
                () = shutdown() => break,
                () = tokio::time::sleep(Duration::from_secs(self.interval.saturating_add(jitter))) => {},
            }
        }
    }

    async fn report_once(&mut self) {
        if let Err(error) = self.tick().await {
            self.channel = None;
            self.session = None;
            self.status("unavailable", &error.to_string()).await;
            kmsg::warn!("Fleet reporting unavailable: {error}");
        }
    }

    pub(crate) async fn tick(&mut self) -> Result<()> {
        if self.shared.renewal.swap(false, Ordering::AcqRel) {
            self.renew().await?;
        }
        if self.store.data()?.certificate.is_none() {
            return self.enroll().await;
        }
        let mut client = FleetServiceClient::new(self.channel().await?);
        if self.session.is_none() && !self.register(&mut client).await? {
            return Ok(());
        }
        let snapshot = self.snapshot().await;
        let Some(snapshot) = snapshot else {
            self.status(
                "waiting_inventory",
                "No recent local workload observation; no empty replacement is sent",
            )
            .await;
            return Ok(());
        };
        client
            .report_inventory(ReportInventoryRequest {
                session_id: self.session.clone().context("missing session")?,
                sequence: self.sequence,
                snapshot: Some(snapshot),
            })
            .await?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("report sequence exhausted")?;
        self.status("reporting", "Latest workload observation accepted")
            .await;
        Ok(())
    }

    async fn snapshot(&self) -> Option<NodeSnapshot> {
        let observation = self.shared.observation.lock().await;
        observation
            .as_ref()
            .filter(|entry| {
                entry.1.elapsed() < Duration::from_secs(self.interval.saturating_mul(3))
            })
            .map(|entry| convert::to_wire(&entry.0))
    }

    async fn register(&mut self, client: &mut FleetServiceClient<Channel>) -> Result<bool> {
        let credentials = self.store.data()?;
        let response = client
            .register_node(RegisterNodeRequest {
                domain_id: credentials.domain.clone(),
                node_id: credentials.node.clone(),
                hostname: self.hostname.clone(),
            })
            .await?
            .into_inner();
        if self.interval.saturating_mul(3).saturating_add(2) >= response.stale_after_secs {
            bail!(
                "report interval must leave room for three reports and jitter before manager staleness"
            );
        }
        if response.pending {
            self.status(
                "pending_node",
                "Approve the exact node and certificate fingerprint on the manager",
            )
            .await;
            return Ok(false);
        }
        if response.session_id.is_empty() {
            bail!("manager returned no session");
        }
        self.session = Some(response.session_id);
        self.sequence = 0;
        Ok(true)
    }

    async fn channel(&mut self) -> Result<Channel> {
        if let Some(channel) = self.channel.as_ref() {
            return Ok(channel.clone());
        }
        let credentials = self.store.data()?;
        let mut tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(&credentials.ca_pem))
            .domain_name(&self.server_name);
        if let Some(certificate) = credentials.certificate.as_ref() {
            tls = tls.identity(Identity::from_pem(certificate, &credentials.key));
        }
        let channel = Endpoint::from_shared(self.endpoint.clone())?
            .tls_config(tls)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .connect()
            .await?;
        self.channel = Some(channel.clone());
        Ok(channel)
    }

    async fn enroll(&mut self) -> Result<()> {
        let mut client = AuthServiceClient::new(self.channel().await?);
        let credentials = self.store.data()?;
        let fingerprint = pki::csr::compute_fingerprint(&credentials.csr)?;
        let response = client
            .get_csr_status(GetCsrStatusRequest {
                fingerprint: fingerprint.clone(),
            })
            .await?
            .into_inner();
        match CsrStatus::try_from(response.status)? {
            CsrStatus::NotFound => {
                client
                    .submit_csr(SubmitCsrRequest {
                        csr_pem: credentials.csr.clone(),
                    })
                    .await?;
                self.status(
                    "pending_certificate",
                    "Approve the CSR with fleet:report permission on the manager",
                )
                .await;
            }
            CsrStatus::Pending => {
                self.status(
                    "pending_certificate",
                    "Waiting for certificate approval on the manager",
                )
                .await;
            }
            CsrStatus::Rejected => {
                bail!("certificate request rejected; explicit administrator action required")
            }
            CsrStatus::Approved => {
                self.accept_certificate(response.cert_pem)?;
                let certificate = self
                    .store
                    .data()?
                    .certificate
                    .as_ref()
                    .context("missing enrolled certificate")?;
                self.shared.status.lock().await.certificate_fingerprint =
                    certificate_fingerprint(certificate)?;
                // Acknowledgement is after the durable key/certificate association.
                client
                    .ack_enrollment(AckEnrollmentRequest { fingerprint })
                    .await?;
                self.channel = None;
                self.status(
                    "certificate_ready",
                    "Connecting with the approved credential",
                )
                .await;
            }
        }
        Ok(())
    }

    fn accept_certificate(&mut self, certificate: String) -> Result<()> {
        if certificate.is_empty() || certificate.len() > 0x4000 {
            bail!("invalid enrollment certificate");
        }
        let mut next = self.store.data()?.clone();
        next.certificate = Some(certificate);
        self.store.commit(next)
    }

    async fn renew(&mut self) -> Result<()> {
        let mut next = self.store.data()?.clone();
        let (key, csr) = pki::csr::generate(&format!("muak-fleet-{}", next.node))?;
        let fingerprint = pki::csr::compute_fingerprint(&csr)?;
        next.key = key;
        next.csr = csr;
        next.certificate = None;
        self.store.commit(next)?;
        self.channel = None;
        self.session = None;
        let mut status = self.shared.status.lock().await;
        status.csr_fingerprint = fingerprint;
        status.certificate_fingerprint.clear();
        Ok(())
    }

    async fn status(&self, phase: &str, detail: &str) {
        let mut status = self.shared.status.lock().await;
        phase.clone_into(&mut status.phase);
        status.detail = detail.chars().take(1024).collect();
    }
}

fn certificate_fingerprint(value: &str) -> Result<String> {
    Ok(pki::cert::compute_fingerprint(&pki::pem::decode_cert(
        value,
    )?)?)
}
