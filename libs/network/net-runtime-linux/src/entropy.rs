//! Operating system entropy through `getrandom`.

pub(crate) struct Entropy;

impl Entropy {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl net_runtime::Entropy for super::Tokio {
    fn fill(&self, bytes: &mut [u8]) {
        self.entropy.fill(bytes);
    }
}

/// Fills the buffer from the operating system entropy source.
///
/// # Panics
///
/// Panics when the operating system entropy source is unavailable.
impl net_runtime::Entropy for Entropy {
    fn fill(&self, bytes: &mut [u8]) {
        assert!(
            matches!(getrandom::fill(bytes), Ok(())),
            "operating system entropy source is unavailable"
        );
    }
}
