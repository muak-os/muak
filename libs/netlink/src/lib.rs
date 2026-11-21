//! Abstraction over netlink operations.

#![warn(missing_docs)]

extern crate alloc;

pub mod address;
pub mod bridge;
pub mod link;
pub mod retry;
pub mod route;

use core::result::Result as CoreResult;

use thiserror::Error;

/// Connection setup failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to create netlink connection.
    #[error("failed to create netlink connection: {0}")]
    Connection(#[source] std::io::Error),
}

/// Connection result type.
pub type Result<T> = CoreResult<T, Failure>;

/// Production implementation backed by `rtnetlink::Handle`.
#[derive(Clone)]
pub struct Rtnl {
    pub(crate) handle: rtnetlink::Handle,
}

impl Rtnl {
    /// Builds a provider around an existing rtnetlink handle.
    #[must_use = "dropping it discards the handle"]
    pub const fn new(handle: rtnetlink::Handle) -> Self {
        Self { handle }
    }

    /// Connects to the kernel netlink socket and starts its driver task.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Connection`] when the netlink socket cannot be opened.
    pub fn connect() -> Result<Self> {
        let (connection, handle, _) = rtnetlink::new_connection().map_err(Failure::Connection)?;
        tokio::spawn(connection);
        Ok(Self { handle })
    }
}
