//! Music service integrations.
//!
//! * `spotify` — Spotify Web API (library, search, playlists) + OAuth PKCE.
//!   Audio playback is in `audio::spotify` (librespot, Premium only).
//! * `youtube` — YouTube Data API v3 (account playlists, liked music, search,
//!   playlist creation). Playback stays on YouTube/YouTube Music itself,
//!   because Google does not offer a third-party audio playback API.
//! * `lyrics` — LRCLIB.

pub mod lyrics;
pub mod oauth;
pub mod spotify;
pub mod youtube;

use std::sync::OnceLock;
use std::time::Duration;

static HTTP: OnceLock<reqwest::blocking::Client> = OnceLock::new();

pub fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Shared blocking HTTP client (one connection pool for the whole app).
pub fn http() -> &'static reqwest::blocking::Client {
    HTTP.get_or_init(|| {
        install_crypto();
        reqwest::blocking::Client::builder()
            .user_agent(concat!("AquaGlass/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .build()
            .expect("http client")
    })
}

#[derive(Debug)]
pub enum ApiError {
    /// 401 after refresh — user must sign in again.
    Unauthorized,
    /// 403 — not permitted (scope, Premium, Development-Mode restrictions...).
    Forbidden(String),
    NotFound,
    RateLimited(u64),
    QuotaExceeded,
    Network(String),
    Other(u16, String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "Session expired — please sign in again."),
            ApiError::Forbidden(m) => write!(f, "Not permitted by the service: {m}"),
            ApiError::NotFound => write!(f, "Not found (it may have been deleted or made private)."),
            ApiError::RateLimited(s) => write!(f, "Rate limited by the service — retry in {s}s."),
            ApiError::QuotaExceeded => write!(f, "Daily API quota exhausted — try again tomorrow (YouTube Data API quota resets at midnight Pacific Time)."),
            ApiError::Network(m) => write!(f, "Network problem: {m}"),
            ApiError::Other(c, m) => write!(f, "Service error {c}: {m}"),
        }
    }
}
impl std::error::Error for ApiError {}

/// Extract a human readable error message from a JSON error body.
pub fn error_message(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let m = v["error"]["message"]
        .as_str()
        .or_else(|| v["error_description"].as_str())
        .or_else(|| v["error"].as_str())
        .unwrap_or("");
    let m = if m.is_empty() { body.chars().take(200).collect::<String>() } else { m.to_string() };
    crate::security::scrub(&m)
}

pub fn classify(status: u16, retry_after: Option<u64>, body: &str) -> ApiError {
    match status {
        401 => ApiError::Unauthorized,
        403 => {
            let msg = error_message(body);
            if msg.contains("quota") {
                ApiError::QuotaExceeded
            } else {
                ApiError::Forbidden(msg)
            }
        }
        404 => ApiError::NotFound,
        429 => ApiError::RateLimited(retry_after.unwrap_or(5)),
        _ => ApiError::Other(status, error_message(body)),
    }
}

/// Generic JSON request with rate-limit retry. `auth` returns a fresh bearer token,
/// `force_refresh` is called once on 401.
pub fn request_json(
    method: reqwest::Method,
    url: &str,
    body: Option<&serde_json::Value>,
    auth: &dyn Fn(bool) -> Result<String, ApiError>,
) -> Result<serde_json::Value, ApiError> {
    let mut refreshed = false;
    let mut attempts = 0;
    loop {
        attempts += 1;
        let token = auth(false)?;
        let mut rb = http().request(method.clone(), url).bearer_auth(&token);
        if let Some(b) = body {
            rb = rb.json(b);
        } else if method == reqwest::Method::POST || method == reqwest::Method::PUT {
            rb = rb.header("Content-Length", "0");
        }
        let resp = rb.send().map_err(|e| ApiError::Network(crate::security::scrub(&e.to_string())))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = resp.text().unwrap_or_default();
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(serde_json::Value::Null);
            }
            return serde_json::from_str(&text).map_err(|e| ApiError::Other(status, format!("invalid JSON: {e}")));
        }
        match status {
            401 if !refreshed => {
                refreshed = true;
                auth(true)?;
                continue;
            }
            429 if attempts <= 3 => {
                let wait = retry_after.unwrap_or(2).min(30);
                std::thread::sleep(Duration::from_secs(wait));
                continue;
            }
            500 | 502 | 503 | 504 if attempts <= 2 => {
                std::thread::sleep(Duration::from_millis(800));
                continue;
            }
            _ => return Err(classify(status, retry_after, &text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classify_errors() {
        assert!(matches!(classify(401, None, ""), ApiError::Unauthorized));
        assert!(matches!(classify(429, Some(7), ""), ApiError::RateLimited(7)));
        let yt = r#"{"error":{"code":403,"message":"The request cannot be completed because you have exceeded your quota."}}"#;
        assert!(matches!(classify(403, None, yt), ApiError::QuotaExceeded));
        let sp = r#"{"error":{"status":403,"message":"Insufficient client scope"}}"#;
        match classify(403, None, sp) {
            ApiError::Forbidden(m) => assert_eq!(m, "Insufficient client scope"),
            e => panic!("{e:?}"),
        }
    }
}
