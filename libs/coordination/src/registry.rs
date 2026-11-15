//! In-memory enrollment bindings and monotonic session/report handling.

use alloc::collections::BTreeMap;
use core::time::Duration;
use std::time::Instant;

use uuid::Uuid;

use crate::error::Failure;
use crate::inventory::{Limits, Observation, Snapshot};

/// Identity derived by an adapter from an authenticated connection.
///
/// The adapter must verify the certificate and derive its fingerprint itself;
/// values copied from an untrusted report are not authenticated identity.
#[derive(Debug, Clone, Copy)]
pub struct AuthenticatedPeer {
    /// Persisted management-domain identity established during enrollment.
    pub domain: Uuid,
    /// Persisted node identity, independent of its credential or tunnel key.
    pub node: Uuid,
    /// Fingerprint of the verified client certificate.
    pub fingerprint: [u8; 32],
}

/// An opaque session issued by this registry. Never persist sessions across restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    instance: Uuid,
    node: Uuid,
    generation: u64,
}

#[derive(Debug)]
struct ActiveSession {
    token: Session,
    last_sequence: Option<u64>,
}

/// A registered node and its last retained observation.
#[derive(Debug)]
pub struct Node {
    fingerprint: [u8; 32],
    revoked: bool,
    active: Option<ActiveSession>,
    observation: Option<Observation>,
}

impl Node {
    /// Returns the last observation, including after disconnect or reconnect.
    #[must_use = "callers need the retained observation"]
    pub const fn observation(&self) -> Option<&Observation> {
        self.observation.as_ref()
    }

    /// Returns whether an administrator revoked this registration.
    #[must_use = "callers must inspect registration revocation"]
    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }

    /// Reports freshness without treating lack of contact as host failure.
    ///
    /// Disconnected, revoked and newly reconnected nodes are stale until a valid
    /// report arrives on the current session. Expiry uses the manager's clock;
    /// an evaluation time preceding receipt is conservatively stale.
    #[must_use = "callers must distinguish stale inventory from current reports"]
    pub fn is_stale(&self, now: Instant, max_age: Duration) -> bool {
        self.active
            .as_ref()
            .is_none_or(|session| session.last_sequence.is_none())
            || self
                .observation
                .as_ref()
                .is_none_or(|observation| observation.is_stale(now, max_age))
    }
}

/// A bounded, single-owner registry with no storage or transport side effects.
///
/// Approval and rotation methods are trusted administrative entry points. The
/// caller must persist and restore enrollment policy before accepting connections.
/// Creating a registry generates a fresh session scope; old tokens remain invalid
/// even if the same registrations are later restored. No VM operations are exposed.
#[derive(Debug)]
pub struct Registry {
    domain: Uuid,
    instance: Uuid,
    generation: u64,
    limits: Limits,
    nodes: BTreeMap<Uuid, Node>,
}

impl Registry {
    /// Creates an empty registry for an existing management-domain identity.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::InvalidIdentity`] if the domain identity is nil.
    pub fn new(domain: Uuid, limits: Limits) -> Result<Self, Failure> {
        if domain.is_nil() {
            return Err(Failure::InvalidIdentity);
        }
        Ok(Self {
            domain,
            instance: Uuid::new_v4(),
            generation: 0,
            limits,
            nodes: BTreeMap::new(),
        })
    }

    /// Registers a persistent node identity and its approved credential.
    ///
    /// # Errors
    ///
    /// Rejects nil identities, existing registrations, credential reuse and a
    /// full registry. Revocation cannot be undone by repeating enrollment.
    pub fn approve(&mut self, node: Uuid, fingerprint: [u8; 32]) -> Result<(), Failure> {
        if node.is_nil() {
            return Err(Failure::InvalidIdentity);
        }
        if self.nodes.contains_key(&node) {
            return Err(Failure::AlreadyRegistered);
        }
        self.check_credential(node, fingerprint)?;
        if self.nodes.len() >= self.limits.nodes {
            return Err(Failure::NodeLimit);
        }
        self.nodes.insert(
            node,
            Node {
                fingerprint,
                revoked: false,
                active: None,
                observation: None,
            },
        );
        Ok(())
    }

    /// Replaces an approved credential and invalidates the current session.
    /// Retains the node identity and its last observation, now marked stale.
    ///
    /// # Errors
    ///
    /// Rejects unknown/revoked nodes and a credential bound to another node.
    pub fn rotate_credential(&mut self, node: Uuid, fingerprint: [u8; 32]) -> Result<(), Failure> {
        self.check_credential(node, fingerprint)?;
        let entry = self.nodes.get_mut(&node).ok_or(Failure::UnknownNode)?;
        if entry.revoked {
            return Err(Failure::RevokedNode);
        }
        entry.fingerprint = fingerprint;
        entry.active = None;
        Ok(())
    }

    /// Revokes a registration and invalidates any current session.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::UnknownNode`] for an unregistered identity.
    pub fn revoke(&mut self, node: Uuid) -> Result<(), Failure> {
        let entry = self.nodes.get_mut(&node).ok_or(Failure::UnknownNode)?;
        entry.revoked = true;
        entry.active = None;
        Ok(())
    }

    /// Opens a session for a verified peer, replacing any earlier session.
    /// A new session must supply its own full snapshot before becoming fresh.
    ///
    /// # Errors
    ///
    /// Rejects foreign domains, unknown/revoked nodes, mismatched credentials and
    /// exhaustion of the session counter. Rejection preserves the current session.
    pub fn connect(&mut self, peer: AuthenticatedPeer) -> Result<Session, Failure> {
        if peer.domain != self.domain {
            return Err(Failure::DomainMismatch);
        }
        let entry = self.nodes.get_mut(&peer.node).ok_or(Failure::UnknownNode)?;
        if entry.revoked {
            return Err(Failure::RevokedNode);
        }
        if entry.fingerprint != peer.fingerprint {
            return Err(Failure::CredentialMismatch);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(Failure::SessionExhausted)?;
        let token = Session {
            instance: self.instance,
            node: peer.node,
            generation,
        };
        entry.active = Some(ActiveSession {
            token,
            last_sequence: None,
        });
        self.generation = generation;
        Ok(token)
    }

    /// Accepts a newer complete snapshot on the current session.
    /// Sequences may begin at zero and reset only when a new session is issued.
    ///
    /// # Errors
    ///
    /// Rejects obsolete sessions, repeated/reordered reports, invalid inventory
    /// and configured bounds. Rejection preserves the previous observation.
    pub fn report(
        &mut self,
        session: Session,
        sequence: u64,
        snapshot: Snapshot,
    ) -> Result<(), Failure> {
        let entry = self
            .nodes
            .get_mut(&session.node)
            .ok_or(Failure::StaleSession)?;
        let active = entry.active.as_mut().ok_or(Failure::StaleSession)?;
        if session.instance != self.instance || active.token != session {
            return Err(Failure::StaleSession);
        }
        if active.last_sequence.is_some_and(|last| sequence <= last) {
            return Err(Failure::StaleReport);
        }
        snapshot.validate(self.limits)?;
        entry.observation = Some(Observation {
            snapshot,
            sequence,
            received_at: Instant::now(),
        });
        active.last_sequence = Some(sequence);
        Ok(())
    }

    /// Marks the current session disconnected while retaining its observation.
    /// An obsolete connection's close callback cannot disconnect a newer session.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::StaleSession`] if this token is no longer current.
    pub fn disconnect(&mut self, session: Session) -> Result<(), Failure> {
        let entry = self
            .nodes
            .get_mut(&session.node)
            .ok_or(Failure::StaleSession)?;
        if session.instance != self.instance
            || entry.active.as_ref().map(|active| active.token) != Some(session)
        {
            return Err(Failure::StaleSession);
        }
        entry.active = None;
        Ok(())
    }

    /// Looks up a registration, including revoked and disconnected nodes.
    #[must_use = "callers need the registered node"]
    pub fn node(&self, node: Uuid) -> Option<&Node> {
        self.nodes.get(&node)
    }

    /// Iterates over registered identities and their retained observations.
    pub fn nodes(&self) -> impl Iterator<Item = (&Uuid, &Node)> {
        self.nodes.iter()
    }

    fn check_credential(&self, node: Uuid, fingerprint: [u8; 32]) -> Result<(), Failure> {
        if self
            .nodes
            .iter()
            .any(|(id, entry)| *id != node && entry.fingerprint == fingerprint)
        {
            return Err(Failure::CredentialInUse);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_generation_does_not_replace_current_session() {
        // ARRANGE
        let domain = Uuid::new_v4();
        let node = Uuid::new_v4();
        let mut registry = Registry::new(domain, Limits::new(1, 1, 32).unwrap()).unwrap();
        registry.approve(node, [1; 32]).unwrap();
        let peer = AuthenticatedPeer {
            domain,
            node,
            fingerprint: [1; 32],
        };
        let session = registry.connect(peer).unwrap();
        registry.generation = u64::MAX;

        // ACT
        assert_eq!(registry.connect(peer), Err(Failure::SessionExhausted));

        // ASSERT
        registry.disconnect(session).unwrap();
    }
}
