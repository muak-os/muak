//! Registry-backed catalog operations.

pub mod add;
pub mod compose;
pub mod publish;
pub mod remove;
pub mod verify;

/// Default registry prefix when `REGISTRY` is unset or empty.
const DEFAULT_REGISTRY: &str = "ghcr.io/muak-os";

/// Registry prefix for digest resolution: `REGISTRY`, then the default.
#[must_use]
pub(crate) fn registry() -> String {
    std::env::var("REGISTRY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_owned())
}

/// Fully qualified `repository:tag` reference behind the registry prefix.
#[must_use]
pub(crate) fn reference(repository: &str, tag: &str) -> String {
    format!("{}/{repository}:{tag}", registry())
}
