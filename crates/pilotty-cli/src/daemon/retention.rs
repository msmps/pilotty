//! Bounded retention of raw PTY output.

use std::collections::VecDeque;

/// Default number of raw output bytes retained for each session.
pub(crate) const DEFAULT_RETAIN_BYTES: usize = 2 * 1024 * 1024;

/// A point-in-time copy of a session's retained raw output and accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionSnapshot {
    pub(crate) bytes: Vec<u8>,
    pub(crate) total_bytes: u64,
    pub(crate) retained_bytes: u64,
    pub(crate) dropped_bytes: u64,
    pub(crate) truncated: bool,
}

/// Bounded tail of raw PTY output.
pub(crate) struct RetentionRing {
    bytes: VecDeque<u8>,
    capacity: usize,
    total_bytes: u64,
}

impl RetentionRing {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            capacity,
            total_bytes: 0,
        }
    }

    pub(crate) fn append(&mut self, output: &[u8]) {
        let output_len = u64::try_from(output.len()).unwrap_or(u64::MAX);
        self.total_bytes = self.total_bytes.saturating_add(output_len);

        if self.capacity == 0 {
            self.bytes.clear();
            return;
        }

        if output.len() >= self.capacity {
            self.bytes.clear();
            self.bytes
                .extend(output[output.len() - self.capacity..].iter().copied());
            return;
        }

        let excess = self
            .bytes
            .len()
            .saturating_add(output.len())
            .saturating_sub(self.capacity);
        if excess > 0 {
            self.bytes.drain(..excess);
        }
        self.bytes.extend(output.iter().copied());
    }

    pub(crate) fn snapshot(&self) -> RetentionSnapshot {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        let retained_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let dropped_bytes = self.total_bytes.saturating_sub(retained_bytes);
        RetentionSnapshot {
            bytes,
            total_bytes: self.total_bytes,
            retained_bytes,
            dropped_bytes,
            truncated: dropped_bytes > 0,
        }
    }
}

impl RetentionSnapshot {
    pub(crate) fn into_tail(self, capacity: usize) -> Self {
        let start = self.bytes.len().saturating_sub(capacity);
        let bytes = self.bytes[start..].to_vec();
        let retained_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let dropped_bytes = self.total_bytes.saturating_sub(retained_bytes);
        Self {
            bytes,
            total_bytes: self.total_bytes,
            retained_bytes,
            dropped_bytes,
            truncated: dropped_bytes > 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::daemon::retention::RetentionRing;

    #[test]
    fn retains_only_the_newest_bytes_with_exact_accounting() {
        let mut retention = RetentionRing::new(5);

        retention.append(b"abc");
        retention.append(b"defg");

        let snapshot = retention.snapshot();
        assert_eq!(snapshot.bytes, b"cdefg");
        assert_eq!(snapshot.total_bytes, 7);
        assert_eq!(snapshot.retained_bytes, 5);
        assert_eq!(snapshot.dropped_bytes, 2);
        assert!(snapshot.truncated);
    }

    #[test]
    fn oversized_append_keeps_only_its_tail() {
        let mut retention = RetentionRing::new(4);

        retention.append(b"old");
        retention.append(b"123456");

        let snapshot = retention.snapshot();
        assert_eq!(snapshot.bytes, b"3456");
        assert_eq!(snapshot.total_bytes, 9);
        assert_eq!(snapshot.dropped_bytes, 5);
    }

    #[test]
    fn zero_capacity_counts_every_byte_as_dropped() {
        let mut retention = RetentionRing::new(0);

        retention.append(b"evidence");

        let snapshot = retention.snapshot();
        assert!(snapshot.bytes.is_empty());
        assert_eq!(snapshot.total_bytes, 8);
        assert_eq!(snapshot.retained_bytes, 0);
        assert_eq!(snapshot.dropped_bytes, 8);
        assert!(snapshot.truncated);
    }

    #[test]
    fn tombstone_tail_preserves_total_accounting() {
        let mut retention = RetentionRing::new(10);
        retention.append(b"0123456789");

        let tail = retention.snapshot().into_tail(4);

        assert_eq!(tail.bytes, b"6789");
        assert_eq!(tail.total_bytes, 10);
        assert_eq!(tail.retained_bytes, 4);
        assert_eq!(tail.dropped_bytes, 6);
        assert!(tail.truncated);
    }
}
