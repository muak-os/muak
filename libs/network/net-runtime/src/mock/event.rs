//! Link event port implementation for the deterministic mock runtime.

use std::io::Result;

use super::Mock;
use crate::Events;

impl Events for Mock {
    fn next(&self) -> impl Future<Output = Result<netmodel::LinkEvent>> + Send {
        std::future::ready(
            self.write(|state| state.events.pop_front().ok_or_else(super::would_block)),
        )
    }
}
