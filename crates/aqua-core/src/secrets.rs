//! Secret storage behind a platform adapter.
//!
//! Portable code (providers, radio) calls `get`/`set`/`delete`; the platform
//! layer decides where secrets live by calling `install(...)` once at startup:
//! the Windows Credential Manager on the desktop, and (in the future) the
//! Android Keystore-backed storage on a phone. Without an installed store,
//! secrets are kept in memory for the session and never written to disk.
//!
//! Secrets are device-local: nothing in `crate::sync` carries them.
//! Never log values returned from here. `Redacted` is provided for Debug output.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub mod keys {
    pub const SPOTIFY_REFRESH: &str = "spotify.refresh_token";
    pub const SPOTIFY_WEBAPI_REFRESH: &str = "spotify.webapi_refresh_token";
    pub const YOUTUBE_REFRESH: &str = "youtube.refresh_token";
    pub const YOUTUBE_CLIENT_SECRET: &str = "youtube.client_secret";
    pub const LASTFM_API_KEY: &str = "lastfm.api_key";
}

/// Where secrets are stored on this platform.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, value: &str) -> anyhow::Result<()>;
    fn get(&self, key: &str) -> Option<String>;
    fn delete(&self, key: &str);
    /// Survives a restart (false for the in-memory fallback).
    fn persistent(&self) -> bool;
}

/// Session-only fallback (development hosts, tests).
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
    fn delete(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
    fn persistent(&self) -> bool {
        false
    }
}

static STORE: OnceLock<Box<dyn SecretStore>> = OnceLock::new();

/// Install the platform's secret store. Call once, before any secret is read.
/// Returns false if a store was already installed (the first one stays).
pub fn install(store: Box<dyn SecretStore>) -> bool {
    STORE.set(store).is_ok()
}

fn store() -> &'static dyn SecretStore {
    STORE.get_or_init(|| Box::new(MemoryStore::default())).as_ref()
}

/// Namespace for secrets of this app (used by OS stores as the "service" name).
pub const SERVICE: &str = "AquaGlass";

pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    store().set(key, value)
}

pub fn get(key: &str) -> Option<String> {
    store().get(key)
}

pub fn delete(key: &str) {
    store().delete(key)
}

pub fn persistent() -> bool {
    store().persistent()
}

/// Wrapper that hides a secret in Debug/Display output.
#[derive(Clone, Default)]
pub struct Redacted(pub String);
impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Remove anything that looks like a bearer token from an error string before display/logging.
pub fn scrub(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_inclusive(char::is_whitespace) {
        let w = word.trim();
        // file paths (\\dir\\file, /tmp/…, C:\\…) are not secrets
        let is_path = w.starts_with('/') || w.contains('\\') || w.contains(":\\");
        let looks_secret = !is_path && w.len() >= 40 && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~+/=".contains(c));
        if looks_secret {
            out.push_str("<redacted>");
            if word.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrub_tokens() {
        let s = "error: Bearer BQDxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghij failed";
        let r = scrub(s);
        assert!(!r.contains("BQDxyz"));
        assert!(r.contains("failed"));
        assert_eq!(format!("{:?}", Redacted("x".into())), "<redacted>");
        // paths stay readable
        let p = "saved to /tmp/claude-0/cb/radio-selftest-1794677336.txt";
        assert_eq!(scrub(p), p);
        let w = r"saved to C:\Users\someone\AppData\Roaming\AquaGlass\radio-selftest-1794677336.txt";
        assert_eq!(scrub(w), w);
    }
    #[test]
    fn store_roundtrip() {
        set("test.key", "v").unwrap();
        assert_eq!(get("test.key").as_deref(), Some("v"));
        delete("test.key");
        assert!(get("test.key").is_none());
    }
}
