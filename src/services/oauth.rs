//! OAuth 2.0 authorization-code flow with PKCE for native apps (RFC 8252):
//! open the system browser, receive the code on a loopback redirect
//! (`http://127.0.0.1:<port>/<path>`), exchange it for tokens.
//! No embedded browser, no cookie copying.

use base64::Engine;
use rand::Rng;
use sha2::Digest;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct OAuthSpec {
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    /// Only for providers that require it for installed apps (Google). Not confidential.
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    /// Port 0 = pick a free port.
    pub port: u16,
    pub path: String,
    pub extra_auth_params: Vec<(String, String)>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone)]
pub struct Token {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Instant,
}

impl Token {
    pub fn from_response(r: TokenResponse, prev_refresh: Option<String>) -> Token {
        Token {
            access: r.access_token,
            refresh: r.refresh_token.or(prev_refresh),
            expires_at: Instant::now() + Duration::from_secs(r.expires_in.unwrap_or(3600).saturating_sub(60)),
        }
    }
    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn pkce_pair() -> (String, String) {
    let mut rng = rand::rng();
    let verifier: String = (0..64)
        .map(|_| {
            const CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
            CH[rng.random_range(0..CH.len())] as char
        })
        .collect();
    let challenge = b64(&sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn random_state() -> String {
    let b: [u8; 16] = rand::random();
    b64(&b)
}

/// Run the interactive flow. Blocking; call from a worker thread.
pub fn authorize(spec: &OAuthSpec, http: &reqwest::blocking::Client) -> anyhow::Result<TokenResponse> {
    let listener = TcpListener::bind(("127.0.0.1", spec.port))
        .map_err(|e| anyhow::anyhow!("Could not listen on 127.0.0.1:{} for the login redirect ({e}). Is another app using it?", spec.port))?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}{}", spec.path);
    let (verifier, challenge) = pkce_pair();
    let state = random_state();
    let mut url = url::Url::parse(&spec.auth_url)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &spec.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge)
            .append_pair("state", &state)
            .append_pair("scope", &spec.scopes.join(" "));
        for (k, v) in &spec.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    open::that_detached(url.as_str()).map_err(|e| anyhow::anyhow!("Could not open the browser: {e}"))?;

    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        if Instant::now() > deadline {
            anyhow::bail!("Login timed out after 5 minutes.");
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line)?;
                let target = line.split_whitespace().nth(1).unwrap_or("/");
                let u = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
                if u.path() != spec.path {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                let get = |k: &str| u.query_pairs().find(|(a, _)| a == k).map(|(_, v)| v.to_string());
                let (ok, msg) = match (get("code"), get("error"), get("state")) {
                    (Some(c), _, Some(s)) if s == state => (Some(c), "Aqua Glass is now connected. You can close this tab.".to_string()),
                    (_, Some(e), _) => (None, format!("Login failed: {e}")),
                    _ => (None, "Login failed: invalid response (state mismatch).".to_string()),
                };
                let body = format!("<html><body style='font-family:sans-serif;background:#0a111c;color:#e2f0fa'><h2>{msg}</h2></body></html>");
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                match ok {
                    Some(c) => break c,
                    None => anyhow::bail!(msg),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    };

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", spec.client_id.clone()),
        ("code_verifier", verifier),
    ];
    if let Some(s) = &spec.client_secret {
        form.push(("client_secret", s.clone()));
    }
    let resp = http.post(&spec.token_url).form(&form).send()?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        anyhow::bail!("Token exchange failed ({status}): {}", crate::security::scrub(&text));
    }
    Ok(serde_json::from_str(&text)?)
}

pub fn refresh(spec: &OAuthSpec, http: &reqwest::blocking::Client, refresh_token: &str) -> anyhow::Result<TokenResponse> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", spec.client_id.clone()),
    ];
    if let Some(s) = &spec.client_secret {
        form.push(("client_secret", s.clone()));
    }
    let resp = http.post(&spec.token_url).form(&form).send()?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        anyhow::bail!("Session expired or revoked ({status}). Please sign in again.");
    }
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let (v, c) = pkce_pair();
        assert!(v.len() >= 43 && v.len() <= 128);
        assert_eq!(c, b64(&sha2::Sha256::digest(v.as_bytes())));
        assert!(!c.contains('='));
    }
    #[test]
    fn rfc7636_vector() {
        // Appendix B of RFC 7636
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64(&sha2::Sha256::digest(v.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
