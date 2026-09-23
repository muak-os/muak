//! Fixed-capacity ring buffer with scroll offset for viewing kernel log entries.

extern crate alloc;

use alloc::collections::VecDeque;

const RING_CAP: usize = 10_000;

pub struct Buffer {
    ring: VecDeque<String>,
    scroll_offset: usize,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            ring: VecDeque::with_capacity(RING_CAP),
            scroll_offset: 0,
        }
    }

    pub fn push(&mut self, line: String) {
        if self.ring.len() == RING_CAP {
            self.ring.pop_front();
        }
        self.ring.push_back(line);
    }

    pub fn is_live(&self) -> bool {
        self.scroll_offset == 0
    }

    pub fn scroll_up(&mut self, n: usize, log_rows: usize) {
        let max = self.ring.len().saturating_sub(log_rows);
        self.scroll_offset = self.scroll_offset.saturating_add(n).min(max);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn snap_to_live(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn visible_window(&self, log_rows: usize) -> (&[String], &[String]) {
        let total = self.ring.len();
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(log_rows);

        let (front, back) = self.ring.as_slices();
        let front_len = front.len();
        let empty: &[String] = &[];

        if end <= front_len {
            (front.get(start..end).unwrap_or_default(), empty)
        } else if start >= front_len {
            let offset = |index: usize| index.saturating_sub(front_len);
            (
                empty,
                back.get(offset(start)..offset(end)).unwrap_or_default(),
            )
        } else {
            (
                front.get(start..front_len).unwrap_or_default(),
                back.get(..end.saturating_sub(front_len))
                    .unwrap_or_default(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(buf: &Buffer, log_rows: usize) -> Vec<&str> {
        let (head, tail) = buf.visible_window(log_rows);

        head.iter().chain(tail.iter()).map(String::as_str).collect()
    }

    #[test]
    fn push_and_visible_window_shows_tail() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..5 {
            buf.push(format!("line {i}"));
        }

        // ACT / ASSERT
        assert_eq!(window(&buf, 3), ["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn scroll_up_shifts_window_back() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..10 {
            buf.push(format!("line {i}"));
        }

        // ACT
        buf.scroll_up(2, 3);

        // ASSERT
        assert_eq!(window(&buf, 3), ["line 5", "line 6", "line 7"]);
    }

    #[test]
    fn scroll_down_moves_toward_live() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..10 {
            buf.push(format!("line {i}"));
        }
        buf.scroll_up(4, 3);

        // ACT
        buf.scroll_down(2);

        // ASSERT
        assert_eq!(window(&buf, 3), ["line 5", "line 6", "line 7"]);
    }

    #[test]
    fn snap_to_live_resets_offset() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..5 {
            buf.push(format!("line {i}"));
        }
        buf.scroll_up(3, 2);

        // ACT
        buf.snap_to_live();

        // ASSERT
        assert!(buf.is_live());
        assert_eq!(window(&buf, 2), ["line 3", "line 4"]);
    }

    #[test]
    fn capacity_evicts_oldest() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..RING_CAP + 5 {
            buf.push(format!("line {i}"));
        }

        // ACT / ASSERT
        assert_eq!(
            window(&buf, 2),
            [
                format!("line {}", RING_CAP + 3).as_str(),
                format!("line {}", RING_CAP + 4).as_str(),
            ]
        );
    }

    #[test]
    fn visible_window_spans_the_ring_seam() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..100 {
            buf.push(format!("line {i}"));
        }
        buf.scroll_up(50, 10);

        // ACT
        let (head, tail) = buf.visible_window(10);

        // ASSERT
        let combined: Vec<usize> = head
            .iter()
            .chain(tail.iter())
            .filter_map(|line| line.strip_prefix("line "))
            .filter_map(|index| index.parse().ok())
            .collect();
        let mut sorted = combined.clone();
        sorted.sort_unstable();
        assert_eq!(combined, sorted, "segments must be in logical order");
        assert_eq!(combined.len(), 10);
    }

    #[test]
    fn scroll_up_clamps_to_max() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..5 {
            buf.push(format!("line {i}"));
        }

        // ACT
        buf.scroll_up(100, 3);

        // ASSERT
        assert_eq!(window(&buf, 3), ["line 0", "line 1", "line 2"]);
    }

    #[test]
    fn scroll_down_clamps_to_zero() {
        // ARRANGE
        let mut buf = Buffer::new();
        buf.push("only".to_owned());

        // ACT
        buf.scroll_down(100);

        // ASSERT
        assert!(buf.is_live());
    }

    #[test]
    fn empty_buffer_visible_window_is_empty() {
        // ARRANGE
        let buf = Buffer::new();

        // ACT / ASSERT
        assert_eq!(window(&buf, 10), Vec::<&str>::new());
    }

    #[test]
    fn fewer_lines_than_window_shows_all() {
        // ARRANGE
        let mut buf = Buffer::new();
        buf.push("a".to_owned());
        buf.push("b".to_owned());

        // ACT / ASSERT
        assert_eq!(window(&buf, 10), ["a", "b"]);
    }

    #[test]
    fn is_live_reflects_scroll_state() {
        // ARRANGE
        let mut buf = Buffer::new();
        for i in 0..5 {
            buf.push(format!("line {i}"));
        }

        // ACT / ASSERT
        assert!(buf.is_live());
        buf.scroll_up(1, 3);
        assert!(!buf.is_live());
        buf.snap_to_live();
        assert!(buf.is_live());
    }
}
