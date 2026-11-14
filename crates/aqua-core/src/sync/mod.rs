//! Cross-device synchronisation **data model** (no networking).
//!
//! This module defines what Aqua Glass devices would exchange with an
//! optional self-hosted sync server — listening state, queues, the local
//! library, shareable settings and playback-handoff offers — and how
//! concurrent edits merge. Nothing here opens sockets, starts threads or
//! polls; the desktop app does not sync yet. See
//! `docs/future-mobile-architecture.md` and `docs/sync-server.md`.
//!
//! Design rules:
//! * **Versioned**: every payload travels in an [`Envelope`] carrying
//!   [`SCHEMA_VERSION`]. Additive changes (new optional fields) keep the
//!   version (unknown fields are ignored, missing ones default). Breaking
//!   changes bump it and add a step to [`migrate`].
//! * **Deterministic merges**: every mergeable value carries a hybrid-logical
//!   [`Stamp`] (time, counter, device id) with a total order, so all devices
//!   converge to the same result regardless of the order updates arrive in.
//! * **No secrets**: no payload type has a field for tokens, passwords or API
//!   keys. Music-service credentials stay in each device's secret store.
//! * **Local vs provider data**: Spotify/YouTube-owned data is only
//!   *referenced* (ids, snapshots); sync never writes back to a provider.

pub mod clock;
pub mod handoff;
pub mod outbox;
pub mod state;

pub use clock::{HybridClock, Lww, LwwMap, Stamp};
pub use handoff::{plan_handoff, Capabilities, HandoffAction, HandoffOffer, HandoffPlan};
pub use outbox::{Change, Outbox};
pub use state::*;

use crate::platform::Platform;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Current sync schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// Oldest schema this build can still read (via [`migrate`]).
pub const MIN_SCHEMA_VERSION: u32 = 1;

/// Random, per-installation identifier. Not derived from hardware, account or
/// host name, so it reveals nothing about the user. Revoking a device on the
/// server invalidates its credentials, not this id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn generate() -> DeviceId {
        let n: u128 = rand::random();
        DeviceId(format!("{n:032x}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DeviceInfo {
    pub id: DeviceId,
    /// User-chosen label ("Desk PC", "Phone"). Defaults to the platform name —
    /// never the computer's host name or the user's account name.
    pub name: String,
    pub platform: Platform,
    pub app_version: String,
}

impl DeviceInfo {
    /// Load this installation's identity from `path`, creating it on first use.
    /// Only called when something actually needs a device identity (export,
    /// future sync) — not at startup.
    pub fn load_or_create(path: &std::path::Path, app_version: &str) -> DeviceInfo {
        if let Some(mut d) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<DeviceInfo>(&b).ok()).filter(|d| !d.id.0.is_empty()) {
            d.app_version = app_version.to_string();
            d.platform = Platform::current();
            return d;
        }
        let platform = Platform::current();
        let d = DeviceInfo { id: DeviceId::generate(), name: format!("{platform:?} device"), platform, app_version: app_version.to_string() };
        if let Ok(b) = serde_json::to_vec_pretty(&d) {
            let _ = crate::storage::atomic_write(path, &b);
        }
        d
    }
}

/// What an envelope contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Listening,
    Queue,
    Library,
    Settings,
    Handoff,
    Changes,
    Snapshot,
}

/// Versioned wrapper for every sync payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub schema: u32,
    pub kind: Kind,
    /// Device that produced the payload.
    pub device: DeviceId,
    /// Wall-clock milliseconds when it was produced (informational; merges use stamps).
    pub created_ms: i64,
    pub payload: T,
}

#[derive(Debug, PartialEq)]
pub enum SyncError {
    Malformed(String),
    /// Produced by a newer Aqua Glass; this build must not guess at it.
    NewerSchema { found: u32, supported: u32 },
    TooOld { found: u32, oldest: u32 },
    WrongKind { expected: Kind, found: Kind },
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Malformed(m) => write!(f, "malformed sync data: {m}"),
            SyncError::NewerSchema { found, supported } => write!(f, "sync data uses schema {found}, this version understands up to {supported} — update Aqua Glass"),
            SyncError::TooOld { found, oldest } => write!(f, "sync data uses schema {found}, older than the oldest supported ({oldest})"),
            SyncError::WrongKind { expected, found } => write!(f, "expected {expected:?} data, got {found:?}"),
        }
    }
}
impl std::error::Error for SyncError {}

impl<T: Serialize> Envelope<T> {
    pub fn new(kind: Kind, device: DeviceId, payload: T) -> Envelope<T> {
        Envelope { schema: SCHEMA_VERSION, kind, device, created_ms: crate::storage::now_millis(), payload }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Decode an envelope, upgrading older schemas and refusing newer ones.
pub fn decode<T: DeserializeOwned>(json: &str, expected: Kind) -> Result<Envelope<T>, SyncError> {
    let mut v: serde_json::Value = serde_json::from_str(json).map_err(|e| SyncError::Malformed(e.to_string()))?;
    let found = v["schema"].as_u64().ok_or_else(|| SyncError::Malformed("missing schema".into()))? as u32;
    if found > SCHEMA_VERSION {
        return Err(SyncError::NewerSchema { found, supported: SCHEMA_VERSION });
    }
    if found < MIN_SCHEMA_VERSION {
        return Err(SyncError::TooOld { found, oldest: MIN_SCHEMA_VERSION });
    }
    migrate(&mut v, found);
    let env: Envelope<T> = serde_json::from_value(v).map_err(|e| SyncError::Malformed(e.to_string()))?;
    if env.kind != expected {
        return Err(SyncError::WrongKind { expected, found: env.kind });
    }
    Ok(env)
}

/// Upgrade a raw envelope from schema `from` to [`SCHEMA_VERSION`] in place.
/// Each breaking schema change adds one `from == N` step here.
pub fn migrate(v: &mut serde_json::Value, from: u32) {
    let _ = from; // schema 1 is the first version: nothing to upgrade yet
    v["schema"] = serde_json::json!(SCHEMA_VERSION);
}

#[cfg(test)]
mod tests;
