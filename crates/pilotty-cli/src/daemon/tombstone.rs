//! Bounded, short-lived evidence for finalized sessions.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use pilotty_core::snapshot::ScreenState;

use crate::daemon::retention::RetentionSnapshot;
use crate::daemon::session::SessionId;

pub(crate) const TOMBSTONE_CAPACITY: usize = 100;
pub(crate) const TOMBSTONE_TTL: Duration = Duration::from_secs(10 * 60);
pub(crate) const TOMBSTONE_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExitMetadata {
    pub(crate) code: Option<u32>,
    pub(crate) signal: Option<String>,
    pub(crate) success: bool,
    pub(crate) killed_by_client: bool,
}

impl ExitMetadata {
    pub(crate) fn description(&self) -> String {
        if self.killed_by_client {
            return "killed by client".to_string();
        }
        if let Some(signal) = &self.signal {
            return format!("signal {signal}");
        }
        self.code
            .map(|code| format!("exit code {code}"))
            .unwrap_or_else(|| "exit status unavailable".to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tombstone {
    pub(crate) id: SessionId,
    pub(crate) name: Option<String>,
    pub(crate) command: Vec<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) ended_at: DateTime<Utc>,
    pub(crate) ended_at_monotonic: Instant,
    pub(crate) exit: ExitMetadata,
    pub(crate) output_complete: bool,
    pub(crate) final_screen: ScreenState,
    pub(crate) output: RetentionSnapshot,
}

impl Tombstone {
    pub(crate) fn ended_at_monotonic(&self) -> Instant {
        self.ended_at_monotonic
    }
}

pub(crate) struct TombstoneStore {
    entries: HashMap<SessionId, Tombstone>,
    insertion_order: VecDeque<SessionId>,
    capacity: usize,
    ttl: Duration,
}

impl TombstoneStore {
    pub(crate) fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            capacity,
            ttl,
        }
    }

    pub(crate) fn insert(&mut self, tombstone: Tombstone, now: Instant) {
        self.purge_expired(now);
        if self.entries.remove(&tombstone.id).is_some() {
            self.insertion_order.retain(|id| id != &tombstone.id);
        }
        while self.entries.len() >= self.capacity && self.capacity > 0 {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        if self.capacity == 0 {
            return;
        }
        self.insertion_order.push_back(tombstone.id.clone());
        self.entries.insert(tombstone.id.clone(), tombstone);
    }

    pub(crate) fn get(&mut self, id: &SessionId, now: Instant) -> Option<Tombstone> {
        self.purge_expired(now);
        self.entries.get(id).cloned()
    }

    pub(crate) fn newest_by_name(&mut self, name: &str, now: Instant) -> Option<Tombstone> {
        self.purge_expired(now);
        self.insertion_order.iter().rev().find_map(|id| {
            self.entries
                .get(id)
                .filter(|item| item.name.as_deref() == Some(name))
                .cloned()
        })
    }

    pub(crate) fn purge_expired(&mut self, now: Instant) {
        self.insertion_order.retain(|id| {
            let retain = self.entries.get(id).is_some_and(|item| {
                now.saturating_duration_since(item.ended_at_monotonic()) < self.ttl
            });
            if !retain {
                self.entries.remove(id);
            }
            retain
        });
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::Utc;
    use pilotty_core::snapshot::ScreenState;

    use crate::daemon::retention::RetentionSnapshot;
    use crate::daemon::session::SessionId;
    use crate::daemon::tombstone::{ExitMetadata, Tombstone, TombstoneStore};

    fn tombstone(id: &str, name: &str, ended_at: Instant) -> Tombstone {
        Tombstone {
            id: SessionId::from(id),
            name: Some(name.to_string()),
            command: vec!["sh".to_string()],
            cwd: None,
            created_at: Utc::now(),
            ended_at: Utc::now(),
            ended_at_monotonic: ended_at,
            exit: ExitMetadata {
                code: Some(0),
                signal: None,
                success: true,
                killed_by_client: false,
            },
            output_complete: true,
            final_screen: ScreenState::empty(80, 24),
            output: RetentionSnapshot {
                bytes: vec![],
                total_bytes: 0,
                retained_bytes: 0,
                dropped_bytes: 0,
                truncated: false,
            },
        }
    }

    #[test]
    fn capacity_evicts_the_oldest_tombstone() {
        let start = Instant::now();
        let mut store = TombstoneStore::new(2, Duration::from_secs(60));
        store.insert(tombstone("one", "same", start), start);
        store.insert(tombstone("two", "same", start), start);
        store.insert(tombstone("three", "same", start), start);

        assert!(store.get(&SessionId::from("one"), start).is_none());
        assert!(store.get(&SessionId::from("two"), start).is_some());
        assert_eq!(
            store.newest_by_name("same", start).map(|item| item.id),
            Some(SessionId::from("three"))
        );
    }

    #[test]
    fn ttl_expires_tombstones() {
        let start = Instant::now();
        let mut store = TombstoneStore::new(100, Duration::from_secs(10));
        store.insert(tombstone("one", "expired", start), start);

        assert!(store
            .get(&SessionId::from("one"), start + Duration::from_secs(9))
            .is_some());
        assert!(store
            .get(&SessionId::from("one"), start + Duration::from_secs(10))
            .is_none());
        assert_eq!(store.len(), 0);
    }
}
