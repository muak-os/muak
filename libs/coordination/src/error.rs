//! Failures of enrollment, session handling and snapshot acceptance.

use thiserror::Error;

/// A rejected registry operation. Rejected reports never update inventory age.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Failure {
    /// Registry limits must be nonzero.
    #[error("registry limits must be nonzero")]
    InvalidLimits,
    /// Persistent domain, node and VM identities cannot be nil UUIDs.
    #[error("identity must not be a nil UUID")]
    InvalidIdentity,
    /// The configured number of registered nodes has been reached.
    #[error("registered node limit reached")]
    NodeLimit,
    /// The node is already registered, including revoked registrations.
    #[error("node already registered")]
    AlreadyRegistered,
    /// The credential is already bound to another registration.
    #[error("credential already bound to another node")]
    CredentialInUse,
    /// The peer belongs to a different management domain.
    #[error("management domain mismatch")]
    DomainMismatch,
    /// An administrator has not registered this node.
    #[error("node not registered")]
    UnknownNode,
    /// The node registration has been revoked.
    #[error("node registration revoked")]
    RevokedNode,
    /// The verified peer credential does not match its registration.
    #[error("credential does not match registered node")]
    CredentialMismatch,
    /// The session is disconnected, replaced, revoked or from another registry.
    #[error("session is no longer current")]
    StaleSession,
    /// The report repeats or precedes an accepted report in this session.
    #[error("report sequence is not newer than the accepted sequence")]
    StaleReport,
    /// No more session generations can be issued in this registry instance.
    #[error("session generation exhausted")]
    SessionExhausted,
    /// The snapshot contains more VMs than configured.
    #[error("VM inventory limit exceeded")]
    SnapshotTooLarge,
    /// The hostname or a VM name is empty or exceeds its byte limit.
    #[error("inventory name is empty or exceeds its byte limit")]
    InvalidName,
    /// A full snapshot lists the same VM identity more than once.
    #[error("duplicate VM identity in snapshot")]
    DuplicateVm,
}
