//! Operating system entropy through `getrandom`.

pub(crate) struct Entropy;

impl Entropy {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl net_runtime::Entropy for super::Tokio {
    /// Fills the buffer from the operating system entropy source.
    ///
    /// # Panics
    ///
    /// Panics when the operating system entropy source is unavailable. The
    /// runtime never substitutes zeroes for entropy.
    fn fill(&self, bytes: &mut [u8]) {
        self.entropy.fill(bytes);
    }
}

impl net_runtime::Entropy for Entropy {
    fn fill(&self, bytes: &mut [u8]) {
        assert!(
            matches!(getrandom::fill(bytes), Ok(())),
            "operating system entropy source is unavailable"
        );
    }
}
