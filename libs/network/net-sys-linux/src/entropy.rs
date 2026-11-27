//! Operating system entropy through `getrandom`.

/// Operating system entropy source.
#[derive(Debug, Clone, Copy, Default)]
pub struct Entropy;

impl Entropy {
    /// Creates an entropy source.
    #[must_use = "the runtime bundle needs an entropy source"]
    pub const fn new() -> Self {
        Self
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
