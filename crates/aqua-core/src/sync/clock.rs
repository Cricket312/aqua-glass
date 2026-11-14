//! Ordering and conflict-free merge primitives.

use super::DeviceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Hybrid logical timestamp. Ordered by (wall ms, counter, device), so any two
/// stamps compare the same way on every device — merges are deterministic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Stamp {
    pub ms: i64,
    pub ctr: u32,
    pub device: DeviceId,
}

/// Remote stamps further than this in the future are treated as clock errors.
pub const MAX_CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;

impl Stamp {
    /// A stamp from a device whose clock runs far ahead would win every
    /// last-writer-wins merge; receivers (and the server) reject those.
    pub fn plausible(&self, now_ms: i64) -> bool {
        self.ms <= now_ms + MAX_CLOCK_SKEW_MS
    }
}

/// Produces monotonically increasing stamps for one device.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct HybridClock {
    pub device: DeviceId,
    last_ms: i64,
    ctr: u32,
}

impl HybridClock {
    pub fn new(device: DeviceId) -> HybridClock {
        HybridClock { device, last_ms: 0, ctr: 0 }
    }

    /// Stamp for a local change at wall time `now_ms`.
    pub fn tick(&mut self, now_ms: i64) -> Stamp {
        if now_ms > self.last_ms {
            self.last_ms = now_ms;
            self.ctr = 0;
        } else {
            self.ctr += 1;
        }
        Stamp { ms: self.last_ms, ctr: self.ctr, device: self.device.clone() }
    }

    /// Account for a stamp received from another device, so later local
    /// changes order after it even if this device's clock is behind.
    pub fn observe(&mut self, remote: &Stamp, now_ms: i64) {
        if !remote.plausible(now_ms) {
            return;
        }
        if remote.ms > self.last_ms {
            self.last_ms = remote.ms;
            self.ctr = remote.ctr;
        } else if remote.ms == self.last_ms {
            self.ctr = self.ctr.max(remote.ctr);
        }
    }
}

/// Last-writer-wins register.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Lww<T> {
    pub value: T,
    pub stamp: Stamp,
}

impl<T: Clone + Serialize> Lww<T> {
    pub fn new(value: T, stamp: Stamp) -> Lww<T> {
        Lww { value, stamp }
    }

    /// Local write (ignored if `stamp` is older than the current one).
    pub fn set(&mut self, value: T, stamp: Stamp) {
        if stamp > self.stamp {
            self.value = value;
            self.stamp = stamp;
        }
    }

    /// Merge another replica. Commutative, associative and idempotent.
    pub fn merge(&mut self, other: &Lww<T>) {
        let take = match other.stamp.cmp(&self.stamp) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            // Identical stamps only happen for the same write; if a buggy
            // client reused a stamp, pick by content so everyone agrees.
            std::cmp::Ordering::Equal => serde_json::to_string(&other.value).ok() > serde_json::to_string(&self.value).ok(),
        };
        if take {
            *self = other.clone();
        }
    }
}

/// Map of last-writer-wins entries where `None` is a tombstone (removed).
/// Used for favourites, saved albums and saved recommendations: "liked on one
/// device, unliked on another" resolves to whichever action happened last,
/// and the removal is remembered so it isn't resurrected by an old replica.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LwwMap<V> {
    pub entries: BTreeMap<String, Lww<Option<V>>>,
}

impl<V> Default for LwwMap<V> {
    fn default() -> Self {
        LwwMap { entries: BTreeMap::new() }
    }
}

impl<V: Clone + Serialize> LwwMap<V> {
    pub fn insert(&mut self, key: String, value: V, stamp: Stamp) {
        self.write(key, Some(value), stamp);
    }

    pub fn remove(&mut self, key: &str, stamp: Stamp) {
        self.write(key.to_string(), None, stamp);
    }

    fn write(&mut self, key: String, value: Option<V>, stamp: Stamp) {
        match self.entries.get_mut(&key) {
            Some(e) => e.set(value, stamp),
            None => {
                self.entries.insert(key, Lww::new(value, stamp));
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key).and_then(|e| e.value.as_ref())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Live (non-removed) entries, in key order.
    pub fn live(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.iter().filter_map(|(k, e)| e.value.as_ref().map(|v| (k, v)))
    }

    pub fn len(&self) -> usize {
        self.live().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn merge(&mut self, other: &LwwMap<V>) {
        for (k, e) in &other.entries {
            match self.entries.get_mut(k) {
                Some(mine) => mine.merge(e),
                None => {
                    self.entries.insert(k.clone(), e.clone());
                }
            }
        }
    }

    /// Drop tombstones older than `before_ms` (after every device has seen them;
    /// the server decides when that is). Keeps the payload from growing forever.
    pub fn compact(&mut self, before_ms: i64) {
        self.entries.retain(|_, e| e.value.is_some() || e.stamp.ms >= before_ms);
    }
}
