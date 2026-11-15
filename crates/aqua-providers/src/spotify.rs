//! Spotify Web API client.
//!
//! Authentication: OAuth 2.0 Authorization Code + PKCE on a loopback redirect.
//! By default Aqua Glass uses the same public client id as librespot (the
//! library that provides Premium playback), so one sign-in serves both the Web
//! API and playback. Users may instead enter their own Spotify Developer client
//! id (Settings › Music Services); note that since February 2026 Spotify's
//! Development-Mode apps need a Premium owner, allow 5 users and have a reduced
//! endpoint set (e.g. playlist `tracks` › `items`).
//!
//! The parser accepts both the pre- and post-2026 field names.

use super::oauth::{self, OAuthSpec, Token};
use super::{request_json, ApiError};
use aqua_core::model::ProviderKind;
use aqua_core::model::*;
use aqua_core::secrets as security;
use serde_json::{json, Value};
use std::sync::Mutex;

pub const LIBRESPOT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const API: &str = "https://api.spotify.com/v1";

pub const SCOPES: &[&str] = &[
    "streaming",
    "user-read-private",
    "user-read-email",
    "user-library-read",
    "user-library-modify",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
    "user-follow-read",
    "user-read-recently-played",
    "user-read-playback-state",
    "user-modify-playback-state",
];

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub id: String,
    pub display_name: String,
    pub product: String,
}

/// One OAuth session (client id + tokens + where its refresh token is stored).
struct Auth {
    spec: OAuthSpec,
    token: Mutex<Option<Token>>,
    key: &'static str,
}

impl Auth {
    fn new(client_id: &str, port: u16, key: &'static str) -> Auth {
        Auth {
            spec: OAuthSpec {
                auth_url: "https://accounts.spotify.com/authorize".into(),
                token_url: "https://accounts.spotify.com/api/token".into(),
                client_id: client_id.to_string(),
                client_secret: None,
                scopes: SCOPES.iter().map(|s| s.to_string()).collect(),
                port,
                path: "/login".into(),
                extra_auth_params: vec![],
            },
            token: Mutex::new(None),
            key,
        }
    }
    fn login(&self) -> anyhow::Result<()> {
        let r = oauth::authorize(&self.spec, crate::http())?;
        let t = Token::from_response(r, None);
        if let Some(rt) = &t.refresh {
            security::set(self.key, rt)?;
        }
        *self.token.lock().unwrap() = Some(t);
        Ok(())
    }
    fn restore(&self) -> anyhow::Result<bool> {
        let Some(rt) = security::get(self.key) else { return Ok(false) };
        self.refresh_with(&rt)?;
        Ok(true)
    }
    fn refresh_with(&self, rt: &str) -> anyhow::Result<()> {
        let r = oauth::refresh(&self.spec, crate::http(), rt)?;
        let t = Token::from_response(r, Some(rt.to_string()));
        if let Some(new_rt) = &t.refresh {
            if new_rt != rt {
                security::set(self.key, new_rt)?;
            }
        }
        *self.token.lock().unwrap() = Some(t);
        Ok(())
    }
    fn signed_in(&self) -> bool {
        self.token.lock().unwrap().is_some()
    }
    fn logout(&self) {
        *self.token.lock().unwrap() = None;
        security::delete(self.key);
    }
    fn access_token(&self, force_refresh: bool) -> Result<String, ApiError> {
        let (cur, rt) = {
            let g = self.token.lock().unwrap();
            match &*g {
                Some(t) => (if t.expired() || force_refresh { None } else { Some(t.access.clone()) }, t.refresh.clone()),
                None => return Err(ApiError::Unauthorized),
            }
        };
        if let Some(a) = cur {
            return Ok(a);
        }
        let rt = rt.ok_or(ApiError::Unauthorized)?;
        self.refresh_with(&rt).map_err(|_| ApiError::Unauthorized)?;
        Ok(self.token.lock().unwrap().as_ref().map(|t| t.access.clone()).unwrap_or_default())
    }
}

pub struct SpotifyApi {
    /// Web API session: your own app's client id when configured (own rate-limit
    /// quota), otherwise librespot's public id (quota shared by all librespot users).
    web: Auth,
    /// Separate playback session (librespot's id) — only when a custom id is used.
    playback: Option<Auth>,
    /// Recent identical searches (query › results), kept 5 minutes.
    search_cache: Mutex<Vec<(String, std::time::Instant, SearchResults)>>,
}

impl SpotifyApi {
    pub fn new(custom_client_id: &str, port: u16) -> SpotifyApi {
        let custom = custom_client_id.trim();
        let (web, playback) = if custom.is_empty() || custom == LIBRESPOT_CLIENT_ID {
            (Auth::new(LIBRESPOT_CLIENT_ID, port, security::keys::SPOTIFY_REFRESH), None)
        } else {
            (Auth::new(custom, port, security::keys::SPOTIFY_WEBAPI_REFRESH), Some(Auth::new(LIBRESPOT_CLIENT_ID, port, security::keys::SPOTIFY_REFRESH)))
        };
        SpotifyApi { web, playback, search_cache: Mutex::new(Vec::new()) }
    }

    pub fn uses_default_client(&self) -> bool {
        self.playback.is_none()
    }

    /// Interactive browser sign-in. With a custom client id this signs in twice in a
    /// row (your app for the Web API, then librespot's for playback); the second
    /// time Spotify usually just confirms because you're already logged in.
    pub fn login(&self) -> anyhow::Result<()> {
        self.web.login()?;
        if let Some(p) = &self.playback {
            if let Err(e) = p.login() {
                log::warn!("Spotify playback sign-in failed: {e}");
            }
        }
        Ok(())
    }

    /// Restore sessions from stored refresh tokens. Returns false if none stored.
    pub fn restore(&self) -> anyhow::Result<bool> {
        let ok = self.web.restore()?;
        if ok {
            if let Some(p) = &self.playback {
                let _ = p.restore();
            }
        }
        Ok(ok)
    }

    pub fn is_signed_in(&self) -> bool {
        self.web.signed_in()
    }

    /// Whether a playback-capable token exists (always true with the default client).
    pub fn playback_signed_in(&self) -> bool {
        self.playback.as_ref().map(|p| p.signed_in()).unwrap_or_else(|| self.web.signed_in())
    }

    /// Sign in only the playback session (when it's missing).
    pub fn login_playback(&self) -> anyhow::Result<()> {
        match &self.playback {
            Some(p) => p.login(),
            None => self.web.login(),
        }
    }

    pub fn logout(&self) {
        self.web.logout();
        if let Some(p) = &self.playback {
            p.logout();
        }
        // also forget a token left over from the other mode
        security::delete(security::keys::SPOTIFY_WEBAPI_REFRESH);
        security::delete(security::keys::SPOTIFY_REFRESH);
    }

    /// Web API access token (refreshing when needed).
    pub fn access_token(&self, force_refresh: bool) -> Result<String, ApiError> {
        self.web.access_token(force_refresh)
    }

    /// Token for librespot playback.
    pub fn playback_token(&self) -> Result<String, ApiError> {
        match &self.playback {
            Some(p) => p.access_token(false),
            None => self.web.access_token(false),
        }
    }

    fn get(&self, url: &str) -> Result<Value, ApiError> {
        request_json(reqwest::Method::GET, url, None, &|f| self.access_token(f))
    }
    fn send(&self, m: reqwest::Method, url: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        request_json(m, url, body, &|f| self.access_token(f))
    }

    /// Follow `next` links.
    fn paged(&self, first: &str, max_items: usize, mut f: impl FnMut(&Value)) -> Result<(), ApiError> {
        let mut url = Some(first.to_string());
        let mut n = 0;
        while let Some(u) = url {
            let v = self.get(&u)?;
            // some endpoints wrap the page (e.g. /me/following › {"artists": {...}})
            let page = if v.get("items").is_some() { &v } else { v.as_object().and_then(|o| o.values().next()).unwrap_or(&v) };
            for it in page["items"].as_array().into_iter().flatten() {
                f(it);
                n += 1;
            }
            if n >= max_items {
                break;
            }
            url = page["next"].as_str().map(|s| s.to_string());
        }
        Ok(())
    }

    pub fn me(&self) -> Result<Profile, ApiError> {
        let v = self.get(&format!("{API}/me"))?;
        Ok(Profile {
            id: v["id"].as_str().unwrap_or_default().into(),
            display_name: v["display_name"].as_str().or(v["id"].as_str()).unwrap_or_default().into(),
            product: v["product"].as_str().unwrap_or_default().into(),
        })
    }

    pub fn playlists(&self, user_id: &str) -> Result<Vec<Playlist>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/me/playlists?limit=50"), 10_000, |it| {
            if let Some(p) = parse_playlist(it, user_id) {
                out.push(p);
            }
        })?;
        Ok(out)
    }

    /// Items of a playlist in order. Uses `/items` (2026 API) and falls back to `/tracks`.
    pub fn playlist_items(&self, id: &str) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::new();
        let first = format!("{API}/playlists/{id}/items?limit=100&additional_types=track");
        match self.paged(&first, 100_000, |it| {
            if let Some(t) = parse_playlist_item(it) {
                out.push(t)
            }
        }) {
            Ok(()) => Ok(out),
            Err(ApiError::NotFound) => {
                out.clear();
                self.paged(&format!("{API}/playlists/{id}/tracks?limit=100"), 100_000, |it| {
                    if let Some(t) = parse_playlist_item(it) {
                        out.push(t)
                    }
                })?;
                Ok(out)
            }
            Err(e) => Err(e),
        }
    }

    /// Cheap change check for a saved collection: (total, newest item id) from a 1-item page.
    /// `which` = "tracks" (Liked Songs), "albums" or "artists" (followed).
    pub fn collection_signature(&self, which: &str) -> Result<(u64, String), ApiError> {
        let (url, ptr) = match which {
            "tracks" => (format!("{API}/me/tracks?limit=1"), "/items/0/track/id"),
            "albums" => (format!("{API}/me/albums?limit=1"), "/items/0/album/id"),
            _ => (format!("{API}/me/following?type=artist&limit=1"), "/artists/items/0/id"),
        };
        let v = self.get(&url)?;
        Ok(signature_of(&v, ptr))
    }

    pub fn liked_tracks(&self) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/me/tracks?limit=50"), 100_000, |it| {
            if let Some(t) = parse_track(&it["track"]) {
                out.push(t)
            }
        })?;
        Ok(out)
    }

    pub fn saved_albums(&self) -> Result<Vec<Album>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/me/albums?limit=50"), 20_000, |it| {
            if let Some(a) = parse_album(&it["album"]) {
                out.push(a)
            }
        })?;
        Ok(out)
    }

    pub fn followed_artists(&self) -> Result<Vec<Artist>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/me/following?type=artist&limit=50"), 20_000, |it| {
            if let Some(a) = parse_artist(it) {
                out.push(a)
            }
        })?;
        Ok(out)
    }

    pub fn recently_played(&self) -> Result<Vec<Track>, ApiError> {
        let v = self.get(&format!("{API}/me/player/recently-played?limit=50"))?;
        Ok(v["items"].as_array().into_iter().flatten().filter_map(|i| parse_track(&i["track"])).collect())
    }

    pub fn album_tracks(&self, album: &Album) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/albums/{}/tracks?limit=50", album.id), 2000, |it| {
            if let Some(mut t) = parse_track(it) {
                t.album = album.name.clone();
                t.album_id = album.id.clone();
                t.artwork_url = album.artwork_url.clone();
                out.push(t)
            }
        })?;
        Ok(out)
    }

    pub fn artist_albums(&self, artist_id: &str) -> Result<Vec<Album>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/artists/{artist_id}/albums?include_groups=album,single&limit=50"), 200, |it| {
            if let Some(a) = parse_album(it) {
                out.push(a)
            }
        })?;
        Ok(out)
    }

    pub fn search(&self, q: &str) -> Result<SearchResults, ApiError> {
        let key = q.trim().to_lowercase();
        {
            let mut c = self.search_cache.lock().unwrap();
            c.retain(|(_, t, _)| t.elapsed() < std::time::Duration::from_secs(300));
            if let Some((_, _, r)) = c.iter().find(|(k, _, _)| *k == key) {
                return Ok(r.clone());
            }
        }
        let r = self.search_uncached(q)?;
        let mut c = self.search_cache.lock().unwrap();
        c.push((key, std::time::Instant::now(), r.clone()));
        if c.len() > 16 {
            c.remove(0);
        }
        Ok(r)
    }

    fn search_uncached(&self, q: &str) -> Result<SearchResults, ApiError> {
        let url = url::Url::parse_with_params(
            &format!("{API}/search"),
            &[("q", q), ("type", "track,album,artist,playlist"), ("limit", "10")],
        )
        .map_err(|e| ApiError::Other(0, e.to_string()))?;
        let v = self.get(url.as_str())?;
        Ok(parse_search(&v))
    }

    /// Find an artist by name (for tracks saved without artist ids). Prefers an
    /// exact (case-insensitive) name match, else Spotify's top result.
    pub fn find_artist(&self, name: &str) -> Result<Option<Artist>, ApiError> {
        let url = url::Url::parse_with_params(&format!("{API}/search"), &[("q", name), ("type", "artist"), ("limit", "5")])
            .map_err(|e| ApiError::Other(0, e.to_string()))?;
        let v = self.get(url.as_str())?;
        let found: Vec<Artist> = v["artists"]["items"].as_array().into_iter().flatten().filter_map(parse_artist).collect();
        Ok(found.iter().find(|a| a.name.eq_ignore_ascii_case(name.trim())).or(found.first()).cloned())
    }

    /// Search for track candidates (used by transfer matching).
    pub fn search_tracks(&self, q: &str, limit: u32) -> Result<Vec<Track>, ApiError> {
        let lim = limit.to_string();
        let url = url::Url::parse_with_params(&format!("{API}/search"), &[("q", q), ("type", "track"), ("limit", lim.as_str())])
            .map_err(|e| ApiError::Other(0, e.to_string()))?;
        let v = self.get(url.as_str())?;
        Ok(v["tracks"]["items"].as_array().into_iter().flatten().filter_map(parse_track).collect())
    }

    /// Create a private playlist for the current user (POST /me/playlists, falling back to /users/{id}/playlists).
    pub fn create_playlist(&self, user_id: &str, name: &str, description: &str) -> Result<String, ApiError> {
        let body = json!({"name": name, "description": description, "public": false});
        let v = match self.send(reqwest::Method::POST, &format!("{API}/me/playlists"), Some(&body)) {
            Err(ApiError::NotFound) => self.send(reqwest::Method::POST, &format!("{API}/users/{user_id}/playlists"), Some(&body))?,
            r => r?,
        };
        v["id"].as_str().map(|s| s.to_string()).ok_or(ApiError::Other(0, "no playlist id returned".into()))
    }

    /// Append tracks (max 100 per request, chunks automatically).
    pub fn add_items(&self, playlist_id: &str, uris: &[String]) -> Result<(), ApiError> {
        for chunk in uris.chunks(100) {
            let body = json!({ "uris": chunk });
            match self.send(reqwest::Method::POST, &format!("{API}/playlists/{playlist_id}/items"), Some(&body)) {
                Err(ApiError::NotFound) => {
                    self.send(reqwest::Method::POST, &format!("{API}/playlists/{playlist_id}/tracks"), Some(&body))?;
                }
                r => {
                    r?;
                }
            }
        }
        Ok(())
    }

    /// Remove (unfollow) a playlist from the user's library — Spotify has no hard delete.
    /// Only called on explicit user request (undo of a transfer).
    pub fn unfollow_playlist(&self, playlist_id: &str) -> Result<(), ApiError> {
        let uri = format!("spotify:playlist:{playlist_id}");
        let u = url::Url::parse_with_params(&format!("{API}/me/library"), &[("uris", uri.as_str())]).unwrap();
        match self.send(reqwest::Method::DELETE, u.as_str(), None) {
            Err(ApiError::NotFound) => self.send(reqwest::Method::DELETE, &format!("{API}/playlists/{playlist_id}/followers"), None).map(|_| ()),
            r => r.map(|_| ()),
        }
    }

    /// Save a track to Liked Songs (PUT /me/library, falling back to /me/tracks).
    pub fn like(&self, track_id: &str) -> Result<(), ApiError> {
        let uri = format!("spotify:track:{track_id}");
        let u = url::Url::parse_with_params(&format!("{API}/me/library"), &[("uris", uri.as_str())]).unwrap();
        match self.send(reqwest::Method::PUT, u.as_str(), None) {
            Err(ApiError::NotFound) => {
                self.send(reqwest::Method::PUT, &format!("{API}/me/tracks?ids={track_id}"), None)?;
                Ok(())
            }
            r => r.map(|_| ()),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing (pure; unit tested)

pub fn signature_of(v: &Value, first_id_ptr: &str) -> (u64, String) {
    let total = v["total"].as_u64().or_else(|| v["artists"]["total"].as_u64()).unwrap_or(0);
    (total, v.pointer(first_id_ptr).and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn best_image(v: &Value) -> String {
    // pick the smallest image >= 160px, else the first
    let imgs: Vec<&Value> = v.as_array().into_iter().flatten().collect();
    let mut best: Option<(&Value, i64)> = None;
    for i in &imgs {
        let w = i["width"].as_i64().unwrap_or(640);
        if w >= 160 && best.is_none_or(|(_, bw)| w < bw) {
            best = Some((i, w));
        }
    }
    best.map(|(i, _)| s(&i["url"])).or_else(|| imgs.first().map(|i| s(&i["url"]))).unwrap_or_default()
}

pub fn parse_track(v: &Value) -> Option<Track> {
    if v.is_null() || v["type"].as_str().is_some_and(|t| t != "track") {
        return None;
    }
    let id = s(&v["id"]);
    let is_local = v["is_local"].as_bool().unwrap_or(false);
    if id.is_empty() && !is_local {
        return None;
    }
    Some(Track {
        provider: ProviderKind::Spotify,
        uri: v["uri"].as_str().map(|x| x.to_string()).unwrap_or_else(|| format!("spotify:track:{id}")),
        id,
        title: s(&v["name"]),
        artists: v["artists"].as_array().into_iter().flatten().map(|a| s(&a["name"])).collect(),
        artist_ids: v["artists"].as_array().into_iter().flatten().map(|a| s(&a["id"])).collect(),
        album: s(&v["album"]["name"]),
        album_id: s(&v["album"]["id"]),
        duration_ms: v["duration_ms"].as_u64().unwrap_or(0),
        explicit: v["explicit"].as_bool().unwrap_or(false),
        isrc: s(&v["external_ids"]["isrc"]),
        artwork_url: best_image(&v["album"]["images"]),
        track_number: v["track_number"].as_u64().unwrap_or(0) as u32,
        unavailable: is_local || v["is_playable"].as_bool() == Some(false),
    })
}

/// Playlist item: 2026 API uses `item`, older uses `track`.
pub fn parse_playlist_item(v: &Value) -> Option<Track> {
    let t = if !v["item"].is_null() { &v["item"] } else { &v["track"] };
    parse_track(t)
}

pub fn parse_playlist(v: &Value, user_id: &str) -> Option<Playlist> {
    if v.is_null() {
        return None;
    }
    let owner_id = s(&v["owner"]["id"]);
    let count = v["items"]["total"].as_u64().or(v["tracks"]["total"].as_u64()).unwrap_or(0);
    Some(Playlist {
        provider: ProviderKind::Spotify,
        id: s(&v["id"]),
        name: s(&v["name"]),
        description: html_unescape(&s(&v["description"])),
        owner: v["owner"]["display_name"].as_str().map(|x| x.to_string()).unwrap_or(owner_id.clone()),
        owned_by_user: !user_id.is_empty() && owner_id == user_id,
        collaborative: v["collaborative"].as_bool().unwrap_or(false),
        public: v["public"].as_bool(),
        artwork_url: best_image(&v["images"]),
        track_count: count as u32,
        snapshot: s(&v["snapshot_id"]),
        is_liked: false,
        items_error: String::new(),
    })
}

pub fn parse_album(v: &Value) -> Option<Album> {
    if v.is_null() || s(&v["id"]).is_empty() {
        return None;
    }
    Some(Album {
        provider: ProviderKind::Spotify,
        id: s(&v["id"]),
        name: s(&v["name"]),
        artists: v["artists"].as_array().into_iter().flatten().map(|a| s(&a["name"])).collect(),
        artwork_url: best_image(&v["images"]),
        release_date: s(&v["release_date"]),
        total_tracks: v["total_tracks"].as_u64().unwrap_or(0) as u32,
    })
}

pub fn parse_artist(v: &Value) -> Option<Artist> {
    if v.is_null() || s(&v["id"]).is_empty() {
        return None;
    }
    Some(Artist {
        provider: ProviderKind::Spotify,
        id: s(&v["id"]),
        name: s(&v["name"]),
        artwork_url: best_image(&v["images"]),
        genres: v["genres"].as_array().into_iter().flatten().map(s).collect(),
    })
}

pub fn parse_search(v: &Value) -> SearchResults {
    SearchResults {
        tracks: v["tracks"]["items"].as_array().into_iter().flatten().filter_map(parse_track).collect(),
        albums: v["albums"]["items"].as_array().into_iter().flatten().filter_map(parse_album).collect(),
        artists: v["artists"]["items"].as_array().into_iter().flatten().filter_map(parse_artist).collect(),
        playlists: v["playlists"]["items"].as_array().into_iter().flatten().filter_map(|p| parse_playlist(p, "")).collect(),
        errors: vec![],
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#x27;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes follow the public Web API reference (both pre- and post-Feb-2026 names).
    #[test]
    fn parses_playlist_items_new_and_old() {
        let track = json!({"type":"track","id":"4uLU6hMCjMI75M1A2tKUQC","uri":"spotify:track:4uLU6hMCjMI75M1A2tKUQC","name":"Song","duration_ms":201000,
            "explicit":false,"artists":[{"name":"A"},{"name":"B"}],"album":{"id":"al","name":"Alb","images":[{"url":"big","width":640},{"url":"mid","width":300},{"url":"small","width":64}]},
            "external_ids":{"isrc":"USUM71703861"}});
        let new = json!({"added_at":"x","item": track});
        let old = json!({"added_at":"x","track": track});
        let a = parse_playlist_item(&new).unwrap();
        let b = parse_playlist_item(&old).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.artists, vec!["A", "B"]);
        assert_eq!(a.artwork_url, "mid");
        assert_eq!(a.isrc, "USUM71703861");
    }

    #[test]
    fn skips_episodes_and_nulls() {
        assert!(parse_playlist_item(&json!({"item": null})).is_none());
        assert!(parse_playlist_item(&json!({"item": {"type":"episode","id":"x"}})).is_none());
    }

    #[test]
    fn parses_playlist_counts() {
        let p_new = json!({"id":"p","name":"N","description":"a &amp; b","owner":{"id":"me","display_name":"Me"},"items":{"total":12},"snapshot_id":"s1","images":[]});
        let p_old = json!({"id":"p","name":"N","owner":{"id":"other"},"tracks":{"total":5}});
        let a = parse_playlist(&p_new, "me").unwrap();
        assert_eq!(a.track_count, 12);
        assert!(a.owned_by_user);
        assert_eq!(a.description, "a & b");
        let b = parse_playlist(&p_old, "me").unwrap();
        assert_eq!(b.track_count, 5);
        assert!(!b.owned_by_user);
    }

    #[test]
    fn tracks_keep_artist_ids_in_order() {
        let t = parse_track(&json!({"id":"t1","name":"Song","artists":[{"id":"a1","name":"One"},{"id":"a2","name":"Two"}],"album":{"id":"al","name":"Album","images":[]}})).unwrap();
        assert_eq!(t.artists, vec!["One", "Two"]);
        assert_eq!(t.artist_ids, vec!["a1", "a2"]);
        assert_eq!(t.album_id, "al");
    }

    #[test]
    fn collection_signatures() {
        let liked = json!({"total": 680, "items": [{"track": {"id": "abc"}}]});
        assert_eq!(signature_of(&liked, "/items/0/track/id"), (680, "abc".to_string()));
        let artists = json!({"artists": {"total": 12, "items": [{"id": "x"}]}});
        assert_eq!(signature_of(&artists, "/artists/items/0/id"), (12, "x".to_string()));
        assert_eq!(signature_of(&json!({}), "/items/0/id"), (0, String::new()));
    }

    #[test]
    fn parses_search() {
        let v = json!({"tracks":{"items":[{"type":"track","id":"t","name":"T","artists":[],"album":{"images":[]}}]},
            "albums":{"items":[{"id":"a","name":"A","artists":[{"name":"X"}],"images":[]}]},
            "artists":{"items":[{"id":"r","name":"R","images":[]}, null]},
            "playlists":{"items":[null,{"id":"p","name":"P","owner":{"id":"o"}}]}});
        let r = parse_search(&v);
        assert_eq!((r.tracks.len(), r.albums.len(), r.artists.len(), r.playlists.len()), (1, 1, 1, 1));
    }
}
