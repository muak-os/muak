//! Byte aggregation for pull progress.

#[derive(Default)]
pub(super) struct ByteState {
    pub(super) sizes: Vec<u64>,
    pub(super) downloaded: Vec<u64>,
    pub(super) total: u64,
    pub(super) current: u64,
}

impl ByteState {
    pub(super) fn ensure(&mut self, index: usize) {
        while self.sizes.len() < index {
            self.sizes.push(0);
            self.downloaded.push(0);
        }
    }

    pub(super) fn set_size(&mut self, index: usize, size: u64) {
        self.ensure(index);
        let Some(slot) = self.sizes.get_mut(index.saturating_sub(1)) else {
            return;
        };
        let previous = *slot;
        *slot = size;
        self.total = self.total.saturating_sub(previous).saturating_add(size);
    }

    pub(super) fn sample(&mut self, index: usize, downloaded: u64, total: u64) -> u64 {
        self.set_size(index, total);
        let Some(slot) = self.downloaded.get_mut(index.saturating_sub(1)) else {
            return self.current;
        };
        let previous = *slot;
        *slot = downloaded;
        self.current = self
            .current
            .saturating_sub(previous)
            .saturating_add(downloaded);

        self.current
    }

    pub(super) fn finish(&mut self, index: usize) -> u64 {
        let size = self.size(index);
        self.sample(index, size, size)
    }

    pub(super) fn size(&self, index: usize) -> u64 {
        self.sizes
            .get(index.saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }
}
