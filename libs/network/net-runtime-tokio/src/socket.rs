//! Socket identifier allocator and table shared by the data plane ports.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

use net_runtime::SocketId;
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::error::{Result, TokioError};

/// Identifier allocator shared by every socket table of one runtime.
pub(crate) struct Ids {
    next: AtomicU32,
}

impl Ids {
    pub(crate) const fn new() -> Self {
        Self {
            next: AtomicU32::new(0),
        }
    }

    pub(crate) fn next(&self) -> SocketId {
        SocketId::new(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

/// Open sockets keyed by the identifier handed to the driver.
pub(crate) struct Table<T> {
    entries: RwLock<HashMap<SocketId, T>>,
}

impl<T> Table<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: RwLock::const_new(HashMap::new()),
        }
    }

    pub(crate) async fn insert(&self, ids: &Ids, socket: T) -> SocketId {
        let id = ids.next();
        self.entries.write().await.insert(id, socket);

        id
    }

    pub(crate) async fn close(&self, id: SocketId) -> Result<()> {
        if self.entries.write().await.remove(&id).is_some() {
            return Ok(());
        }

        Err(not_found())
    }

    /// Borrows the table while a caller performs one socket operation.
    pub(crate) async fn read(&self) -> RwLockReadGuard<'_, HashMap<SocketId, T>> {
        self.entries.read().await
    }
}

pub(crate) fn not_found() -> TokioError {
    TokioError::Io(io::Error::from(io::ErrorKind::NotFound))
}

pub(crate) fn invalid_input(reason: &'static str) -> TokioError {
    TokioError::Io(io::Error::new(io::ErrorKind::InvalidInput, reason))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[tokio::test]
    async fn insert_issues_unique_ids_and_close_removes_once() {
        // ARRANGE
        let ids = Ids::new();
        let table = Table::new();

        // ACT
        let first = table.insert(&ids, 1_u8).await;
        let second = table.insert(&ids, 2_u8).await;
        table.close(first).await.expect("first socket closes");
        let closed = table.close(first).await;

        // ASSERT
        assert_ne!(first, second);
        assert_eq!(table.read().await.get(&second), Some(&2));
        assert!(matches!(
            closed,
            Err(TokioError::Io(error)) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    #[tokio::test]
    async fn tables_sharing_one_allocator_issue_distinct_ids() {
        // ARRANGE
        let ids = Ids::new();
        let packets = Table::new();
        let udp = Table::new();

        // ACT
        let packet = packets.insert(&ids, 1_u8).await;
        let datagram = udp.insert(&ids, 2_u8).await;

        // ASSERT
        assert_ne!(packet, datagram);
        assert_eq!(packets.read().await.get(&packet), Some(&1));
        assert_eq!(udp.read().await.get(&datagram), Some(&2));
    }
}
