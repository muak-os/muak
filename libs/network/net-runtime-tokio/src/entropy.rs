//! Operating system entropy backed by the Linux primitives.

impl net_runtime::Entropy for super::Tokio {
    fn fill(&self, bytes: &mut [u8]) {
        self.entropy.fill(bytes);
    }
}
