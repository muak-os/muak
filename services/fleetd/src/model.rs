//! Durable administrative policy; live inventory and sessions are not restored.

use alloc::collections::BTreeSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Approval {
    Pending,
    Approved,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Enrollment {
    pub(crate) node: String,
    pub(crate) fingerprint: String,
    pub(crate) hostname: String,
    pub(crate) approval: Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Audit {
    pub(crate) actor: String,
    pub(crate) operation: String,
    pub(crate) node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    pub(crate) version: u32,
    pub(crate) domain: String,
    pub(crate) nodes: Vec<Enrollment>,
    pub(crate) audit: Vec<Audit>,
}

impl Policy {
    pub(crate) fn fresh() -> Self {
        Self {
            version: 1,
            domain: Uuid::new_v4().to_string(),
            nodes: Vec::new(),
            audit: Vec::new(),
        }
    }

    pub(crate) fn validate(&self, max_nodes: usize) -> Result<()> {
        if self.version != 1 || self.nodes.len() > max_nodes || self.audit.len() > 256 {
            bail!("unsupported or oversized fleet policy");
        }
        identity(&self.domain)?;
        let mut nodes = BTreeSet::new();
        let mut fingerprints = BTreeSet::new();
        for node in &self.nodes {
            validate_node(node, &mut nodes, &mut fingerprints)?;
        }
        Ok(())
    }

    pub(crate) fn record(&mut self, actor: &str, operation: &str, node: &str) {
        if self.audit.len() == 256 {
            self.audit.remove(0);
        }
        self.audit.push(Audit {
            actor: actor.to_owned(),
            operation: operation.to_owned(),
            node: node.to_owned(),
        });
    }
}

fn validate_node(
    node: &Enrollment,
    nodes: &mut BTreeSet<Uuid>,
    fingerprints: &mut BTreeSet<[u8; 32]>,
) -> Result<()> {
    if !nodes.insert(identity(&node.node)?) || !fingerprints.insert(fingerprint(&node.fingerprint)?)
    {
        bail!("duplicate enrollment identity or credential");
    }
    if node.hostname.is_empty()
        || node.hostname.len() > 255
        || node.hostname.chars().any(char::is_control)
    {
        bail!("invalid node hostname");
    }
    Ok(())
}

pub(crate) fn identity(value: &str) -> Result<Uuid> {
    let id = Uuid::parse_str(value)?;
    if id.is_nil() || id.to_string() != value {
        bail!("a canonical non-nil UUID is required");
    }
    Ok(id)
}

pub(crate) fn fingerprint(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        bail!("a full lower-case SHA256 certificate fingerprint is required");
    }
    let mut bytes = [0; 32];
    base16ct::lower::decode(value, &mut bytes)
        .map_err(|e| anyhow::anyhow!("invalid certificate fingerprint: {e}"))?;
    Ok(bytes)
}
