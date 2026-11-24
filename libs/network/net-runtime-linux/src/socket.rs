//! Socket identifier table shared by the data plane ports.

// A pending `recv` holds the table read guard while it awaits readiness. The
// driver must drop the receive future before closing that socket, otherwise
// `close` waits for the read guard to be released.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

use net_runtime::SocketId;
use tokio::sync::{RwLock, RwLockReadGuard};

/// Open sockets keyed by the identifier handed to the driver.
pub(crate) struct Table<T> {
    entries: RwLock<HashMap<SocketId, T>>,
    next: AtomicU32,
}

impl<T> Table<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: RwLock::const_new(HashMap::new()),
            next: AtomicU32::new(0),
        }
    }

    pub(crate) async fn insert(&self, socket: T) -> SocketId {
        let id = SocketId::new(self.next.fetch_add(1, Ordering::Relaxed));
        self.entries.write().await.insert(id, socket);
        id
    }

    pub(crate) async fn close(&self, id: SocketId) -> io::Result<()> {
        if self.entries.write().await.remove(&id).is_some() {
            return Ok(());
        }

        Err(not_found())
    }

    /// Borrows the table while a caller performs one socket operation.
    ///
    /// The returned guard is held across the socket await, so closing that
    /// socket requires the receive or send future to be dropped first.
    pub(crate) async fn read(&self) -> RwLockReadGuard<'_, HashMap<SocketId, T>> {
        self.entries.read().await
    }
}

pub(crate) fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

pub(crate) fn invalid_input(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_issues_unique_ids_and_close_removes_once() {
        // ARRANGE
        let table = Table::new();

        // ACT
        let first = table.insert(1_u8).await;
        let second = table.insert(2_u8).await;
        table.close(first).await.expect("first socket closes");
        let closed = table.close(first).await;

        // ASSERT
        assert_ne!(first, second);
        assert_eq!(table.read().await.get(&second), Some(&2));
        assert!(matches!(
            closed,
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
    }
}
