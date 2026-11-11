//! Fake firmware variable store shared by the sbolt integration tests.

use core::cell::RefCell;

use firmware::error::Result as FwResult;
use firmware::nvram::{Id, Update, VariableStore};
use firmware::secureboot::{PK, SETUP_MODE};

/// Fake firmware variable store with configurable persistence behaviour.
pub(crate) struct Firmware {
    booted: bool,
    available: bool,
    ready: bool,
    honors_deletes: bool,
    stores_writes: bool,
    ignored_write: Option<Id>,
    failing_write: Option<Id>,
    variables: RefCell<Vec<(Id, Vec<u8>)>>,
    writes: RefCell<Vec<(Id, Vec<u8>)>>,
}

impl Firmware {
    /// Builds a booted, available, ready store that persists every write.
    #[must_use]
    pub(crate) fn ready() -> Self {
        Self {
            booted: true,
            available: true,
            ready: true,
            honors_deletes: true,
            stores_writes: true,
            ignored_write: None,
            failing_write: None,
            variables: RefCell::new(Vec::new()),
            writes: RefCell::new(Vec::new()),
        }
    }

    /// Seeds a variable as if firmware had already committed it.
    #[must_use]
    pub(crate) fn with_variable(self, id: Id, payload: Vec<u8>) -> Self {
        self.variables.borrow_mut().push((id, payload));
        self
    }

    /// Reports that preparing variable storage is not ready.
    #[must_use]
    pub(crate) fn not_ready(self) -> Self {
        Self {
            ready: false,
            ..self
        }
    }

    /// Accepts writes without committing them to the variable store.
    #[must_use]
    pub(crate) fn dropping_writes(self) -> Self {
        Self {
            stores_writes: false,
            ..self
        }
    }

    /// Accepts writes to `id` without committing them to the variable store.
    #[must_use]
    pub(crate) fn ignoring_writes_on(self, id: Id) -> Self {
        Self {
            ignored_write: Some(id),
            ..self
        }
    }

    /// Rejects writes to `id` without persisting them.
    #[must_use]
    pub(crate) fn failing_on(self, id: Id) -> Self {
        Self {
            failing_write: Some(id),
            ..self
        }
    }

    /// Reports deleted variables as still present.
    #[must_use]
    pub(crate) fn keeping_deletes(self) -> Self {
        Self {
            honors_deletes: false,
            ..self
        }
    }

    /// Every write attempted through the store, in order.
    #[must_use]
    pub(crate) fn writes(&self) -> Vec<(Id, Vec<u8>)> {
        self.writes.borrow().clone()
    }
}

impl VariableStore for Firmware {
    fn is_boot(&self) -> bool {
        self.booted
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn prepare(&self) -> FwResult<bool> {
        Ok(self.ready)
    }

    fn exists(&self, id: &Id) -> bool {
        self.variables.borrow().iter().any(|entry| entry.0 == *id)
    }

    fn read(&self, id: &Id) -> FwResult<Option<Vec<u8>>> {
        Ok(self
            .variables
            .borrow()
            .iter()
            .find(|entry| entry.0 == *id)
            .map(|entry| entry.1.clone()))
    }

    fn write(&self, update: Update<'_>) -> FwResult<()> {
        let id = *update.id();
        self.writes.borrow_mut().push((id, update.data().to_vec()));

        if self.ignored_write == Some(id) {
            return Ok(());
        }

        if self.failing_write == Some(id) {
            return Err(firmware::error::FirmwareError::Variable(format!(
                "write to {} failed",
                id.name()
            )));
        }

        persist_variable(
            &self.variables,
            id,
            update.data(),
            self.honors_deletes,
            self.stores_writes,
        );

        Ok(())
    }
}

fn persist_variable(
    variables: &RefCell<Vec<(Id, Vec<u8>)>>,
    id: Id,
    payload: &[u8],
    honors_deletes: bool,
    stores_writes: bool,
) {
    if !stores_writes {
        return;
    }

    let content = signed_content(payload).to_vec();
    if content.is_empty() && !honors_deletes {
        return;
    }

    let mut variables = variables.borrow_mut();
    variables.retain(|entry| entry.0 != id);
    if !content.is_empty() {
        variables.push((id, content));
        if id == PK {
            // Real firmware leaves Setup Mode as soon as a PK is enrolled.
            variables.retain(|entry| entry.0 != SETUP_MODE);
            variables.push((SETUP_MODE, vec![0]));
        }
    }
}

fn signed_content(payload: &[u8]) -> &[u8] {
    let certificate_size = payload
        .get(16..20)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|size| usize::try_from(size).ok())
        .unwrap_or(0);
    let content_offset = 16_usize.saturating_add(certificate_size);

    payload.get(content_offset..).unwrap_or_default()
}
