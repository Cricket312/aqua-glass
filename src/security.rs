//! Secret storage. On Windows, secrets (OAuth refresh tokens, the Google client
//! secret) go to the Windows Credential Manager. On other platforms (development
//! only) they are kept in memory for the session and never written to disk.
//!
//! Never log values returned from here. `Redacted` is provided for Debug output.

use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "AquaGlass";

pub mod keys {
    pub const SPOTIFY_REFRESH: &str = "spotify.refresh_token";
    pub const SPOTIFY_WEBAPI_REFRESH: &str = "spotify.webapi_refresh_token";
    pub const YOUTUBE_REFRESH: &str = "youtube.refresh_token";
    pub const YOUTUBE_CLIENT_SECRET: &str = "youtube.client_secret";
    pub const LASTFM_API_KEY: &str = "lastfm.api_key";
}

#[allow(dead_code)]
static MEM: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

#[cfg(windows)]
pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    keyring::Entry::new(SERVICE, key)?.set_password(value)?;
    Ok(())
}

#[cfg(windows)]
pub fn get(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
}

#[cfg(windows)]
pub fn delete(key: &str) {
    if let Ok(e) = keyring::Entry::new(SERVICE, key) {
        let _ = e.delete_credential();
    }
}

#[cfg(not(windows))]
pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    let _ = SERVICE;
    MEM.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.into(), value.into());
    Ok(())
}

#[cfg(not(windows))]
pub fn get(key: &str) -> Option<String> {
    MEM.lock().unwrap().as_ref()?.get(key).cloned()
}

#[cfg(not(windows))]
pub fn delete(key: &str) {
    if let Some(m) = MEM.lock().unwrap().as_mut() {
        m.remove(key);
    }
}

pub fn persistent() -> bool {
    cfg!(windows)
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
