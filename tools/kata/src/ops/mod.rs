//! Registry-backed catalog operations.

pub mod add;
pub mod compose;
pub mod publish;
pub mod remove;
pub mod verify;

/// Fully qualified `repository:tag` reference behind the registry prefix.
#[must_use]
pub(crate) fn reference(prefix: &str, repository: &str, tag: &str) -> String {
    format!("{prefix}/{repository}:{tag}")
}
