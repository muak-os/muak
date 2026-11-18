//! One durable manager; observations remain a reconstructible projection.

use alloc::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use coordination::inventory::Limits;
use coordination::registry::{AuthenticatedPeer, Registry, Session};
use fleet_api::fleet::{
    GetInfoResponse, NodeRecord, RegisterNodeRequest, RegisterNodeResponse, ReportInventoryRequest,
};
use prost::Message as _;
use uuid::Uuid;

use crate::convert;
use crate::model::{Approval, Enrollment, Policy, fingerprint, identity};
use crate::storage::Store;

pub(crate) struct Manager {
    store: Store<Policy>,
    registry: Registry,
    sessions: BTreeMap<Uuid, (Uuid, Session, [u8; 32])>,
    observation_bytes: BTreeMap<Uuid, usize>,
    max_nodes: usize,
    stale_after_secs: u64,
    ca_pem: String,
}

impl Manager {
    pub(crate) fn open(
        path: &Path,
        max_nodes: usize,
        stale_after_secs: u64,
        ca_pem: String,
    ) -> Result<Self> {
        let store = Store::open(path, || Ok(Policy::fresh()))?;
        store.data()?.validate(max_nodes)?;
        let mut registry = Registry::new(
            identity(&store.data()?.domain)?,
            Limits::new(max_nodes, 1024, 255)?,
        )?;
        for record in &store.data()?.nodes {
            restore(&mut registry, record)?;
        }
        Ok(Self {
            store,
            registry,
            sessions: BTreeMap::new(),
            observation_bytes: BTreeMap::new(),
            max_nodes,
            stale_after_secs,
            ca_pem,
        })
    }

    pub(crate) fn info(&self) -> Result<GetInfoResponse> {
        Ok(GetInfoResponse {
            domain_id: self.store.data()?.domain.clone(),
            ca_pem: self.ca_pem.clone(),
            server_name: "muak-server".to_owned(),
            stale_after_secs: self.stale_after_secs,
        })
    }

    pub(crate) fn register(
        &mut self,
        actor: &str,
        request: RegisterNodeRequest,
    ) -> Result<RegisterNodeResponse> {
        let policy = self.store.data()?;
        if request.domain_id != policy.domain {
            bail!("management domain mismatch");
        }
        let node = identity(&request.node_id)?;
        let credential = fingerprint(actor)?;
        if request.hostname.is_empty()
            || request.hostname.len() > 255
            || request.hostname.chars().any(char::is_control)
        {
            bail!("invalid hostname");
        }
        let existing = policy
            .nodes
            .iter()
            .find(|record| record.node == request.node_id);
        match existing {
            Some(record) if pending(record, actor)? => {
                return Ok(RegisterNodeResponse {
                    pending: true,
                    stale_after_secs: self.stale_after_secs,
                    session_id: String::new(),
                });
            }
            Some(_) => {}
            None => {
                let mut next = policy.clone();
                next.nodes.push(Enrollment {
                    node: request.node_id.clone(),
                    fingerprint: actor.to_owned(),
                    hostname: request.hostname,
                    approval: Approval::Pending,
                });
                next.record(actor, "request", &request.node_id);
                self.commit(next)?;
                return Ok(RegisterNodeResponse {
                    pending: true,
                    stale_after_secs: self.stale_after_secs,
                    session_id: String::new(),
                });
            }
        }
        let session = self.registry.connect(AuthenticatedPeer {
            domain: identity(&self.store.data()?.domain)?,
            node,
            fingerprint: credential,
        })?;
        let token = Uuid::new_v4();
        self.sessions.insert(node, (token, session, credential));
        Ok(RegisterNodeResponse {
            pending: false,
            stale_after_secs: self.stale_after_secs,
            session_id: token.to_string(),
        })
    }

    pub(crate) fn report(&mut self, actor: &str, request: ReportInventoryRequest) -> Result<()> {
        self.store.data()?;
        let token = identity(&request.session_id)?;
        let credential = fingerprint(actor)?;
        let (node, (_, session, _)) = self
            .sessions
            .iter()
            .find(|entry| entry.1.0 == token && entry.1.2 == credential)
            .map(|entry| (*entry.0, *entry.1))
            .context("obsolete or foreign session")?;
        let snapshot = request.snapshot.context("full snapshot required")?;
        let size = snapshot.encoded_len();
        let used = self
            .observation_bytes
            .values()
            .copied()
            .fold(0_usize, usize::saturating_add)
            .saturating_sub(self.observation_bytes.get(&node).copied().unwrap_or(0));
        if used.saturating_add(size) > 2 * 1024 * 1024 {
            bail!("aggregate fleet inventory byte limit exceeded");
        }
        self.registry
            .report(session, request.sequence, convert::from_wire(snapshot)?)?;
        self.observation_bytes.insert(node, size);
        Ok(())
    }

    pub(crate) fn list(&self) -> Result<Vec<NodeRecord>> {
        self.store
            .data()?
            .nodes
            .iter()
            .map(|record| self.view(record))
            .collect()
    }

    pub(crate) fn approve(
        &mut self,
        actor: &str,
        node: &str,
        expected: &str,
    ) -> Result<NodeRecord> {
        let mut next = self.store.data()?.clone();
        let record = next
            .nodes
            .iter_mut()
            .find(|record| record.node == node)
            .context("node is not pending")?;
        if record.approval != Approval::Pending || record.fingerprint != expected {
            bail!("approval requires the exact pending fingerprint");
        }
        record.approval = Approval::Approved;
        next.record(actor, "approve", node);
        self.commit(next)?;
        self.registry
            .approve(identity(node)?, fingerprint(expected)?)?;
        self.find(node)
    }

    pub(crate) fn revoke(&mut self, actor: &str, node: &str) -> Result<NodeRecord> {
        let mut next = self.store.data()?.clone();
        let record = next
            .nodes
            .iter_mut()
            .find(|record| record.node == node)
            .context("unknown node")?;
        let approved = record.approval == Approval::Approved;
        record.approval = Approval::Revoked;
        next.record(actor, "revoke", node);
        self.commit(next)?;
        let id = identity(node)?;
        if approved {
            self.registry.revoke(id)?;
        }
        self.registry.discard_revoked_observations();
        self.observation_bytes.remove(&id);
        self.sessions.remove(&id);
        self.find(node)
    }

    pub(crate) fn rotate(
        &mut self,
        actor: &str,
        node: &str,
        credential: &str,
    ) -> Result<NodeRecord> {
        let bytes = fingerprint(credential)?;
        let mut next = self.store.data()?.clone();
        let record = next
            .nodes
            .iter_mut()
            .find(|record| record.node == node)
            .context("unknown node")?;
        if record.approval != Approval::Approved {
            bail!("only an approved registration can rotate credentials");
        }
        credential.clone_into(&mut record.fingerprint);
        next.record(actor, "rotate", node);
        self.commit(next)?;
        let id = identity(node)?;
        self.registry.rotate_credential(id, bytes)?;
        self.sessions.remove(&id);
        self.find(node)
    }

    fn find(&self, node: &str) -> Result<NodeRecord> {
        let record = self
            .store
            .data()?
            .nodes
            .iter()
            .find(|record| record.node == node)
            .context("unknown node")?;
        self.view(record)
    }

    fn view(&self, record: &Enrollment) -> Result<NodeRecord> {
        let entry = self.registry.node(identity(&record.node)?);
        let observation = entry.and_then(coordination::registry::Node::observation);
        let now = Instant::now();
        Ok(NodeRecord {
            node_id: record.node.clone(),
            fingerprint: record.fingerprint.clone(),
            hostname: observation.map_or_else(
                || record.hostname.clone(),
                |observation| observation.snapshot().hostname.clone(),
            ),
            status: match record.approval {
                Approval::Pending => "pending",
                Approval::Approved => "approved",
                Approval::Revoked => "revoked",
            }
            .to_owned(),
            stale: entry.is_none_or(|node| {
                node.is_stale(now, core::time::Duration::from_secs(self.stale_after_secs))
            }),
            observation_age_secs: observation.and_then(|observation| {
                now.checked_duration_since(observation.received_at())
                    .map(|age| age.as_secs())
            }),
            snapshot: observation.map(|observation| convert::to_wire(observation.snapshot())),
        })
    }

    fn commit(&mut self, policy: Policy) -> Result<()> {
        policy.validate(self.max_nodes)?;
        let event = policy.audit.last().cloned();
        self.store.commit(policy)?;
        if let Some(event) = event {
            kmsg::info!(
                "Fleet decision: actor={} operation={} node={}",
                event.actor,
                event.operation,
                event.node
            );
        }
        Ok(())
    }
}

fn restore(registry: &mut Registry, record: &Enrollment) -> Result<()> {
    if record.approval == Approval::Pending {
        return Ok(());
    }
    let node = identity(&record.node)?;
    registry.approve(node, fingerprint(&record.fingerprint)?)?;
    if record.approval == Approval::Revoked {
        registry.revoke(node)?;
    }
    Ok(())
}

fn pending(record: &Enrollment, actor: &str) -> Result<bool> {
    if record.fingerprint != actor || record.approval == Approval::Revoked {
        bail!("node credential mismatch or revoked registration");
    }
    Ok(record.approval == Approval::Pending)
}
