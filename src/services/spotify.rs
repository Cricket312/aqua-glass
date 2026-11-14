//! Spotify Web API client.
//!
//! Authentication: OAuth 2.0 Authorization Code + PKCE on a loopback redirect.
//! By default Aqua Glass uses the same public client id as librespot (the
//! library that provides Premium playback), so one sign-in serves both the Web
//! API and playback. Users may instead enter their own Spotify Developer client
//! id (Settings → Music Services); note that since February 2026 Spotify's
//! Development-Mode apps need a Premium owner, allow 5 users and have a reduced
//! endpoint set (e.g. playlist `tracks` → `items`).
//!
//! The parser accepts both the pre- and post-2026 field names.

use super::oauth::{self, OAuthSpec, Token};
use super::{request_json, ApiError};
use crate::config::ProviderKind;
use crate::model::*;
use crate::security;
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

pub struct SpotifyApi {
    spec: OAuthSpec,
    token: Mutex<Option<Token>>,
}

impl SpotifyApi {
    pub fn new(custom_client_id: &str, port: u16) -> SpotifyApi {
        let client_id = if custom_client_id.trim().is_empty() { LIBRESPOT_CLIENT_ID.to_string() } else { custom_client_id.trim().to_string() };
        SpotifyApi {
            spec: OAuthSpec {
                auth_url: "https://accounts.spotify.com/authorize".into(),
                token_url: "https://accounts.spotify.com/api/token".into(),
                client_id,
                client_secret: None,
                scopes: SCOPES.iter().map(|s| s.to_string()).collect(),
                port,
                path: "/login".into(),
                extra_auth_params: vec![],
            },
            token: Mutex::new(None),
        }
    }

    pub fn uses_default_client(&self) -> bool {
        self.spec.client_id == LIBRESPOT_CLIENT_ID
    }

    /// Interactive browser sign-in.
    pub fn login(&self) -> anyhow::Result<()> {
        let r = oauth::authorize(&self.spec, super::http())?;
        let t = Token::from_response(r, None);
        if let Some(rt) = &t.refresh {
            security::set(security::keys::SPOTIFY_REFRESH, rt)?;
        }
        *self.token.lock().unwrap() = Some(t);
        Ok(())
    }

    /// Restore a session from the stored refresh token. Returns false if none stored.
    pub fn restore(&self) -> anyhow::Result<bool> {
        let Some(rt) = security::get(security::keys::SPOTIFY_REFRESH) else { return Ok(false) };
        self.refresh_with(&rt)?;
        Ok(true)
    }

    fn refresh_with(&self, rt: &str) -> anyhow::Result<()> {
        let r = oauth::refresh(&self.spec, super::http(), rt)?;
        let t = Token::from_response(r, Some(rt.to_string()));
        if let Some(new_rt) = &t.refresh {
            if new_rt != rt {
                security::set(security::keys::SPOTIFY_REFRESH, new_rt)?;
            }
        }
        *self.token.lock().unwrap() = Some(t);
        Ok(())
    }

    pub fn is_signed_in(&self) -> bool {
        self.token.lock().unwrap().is_some()
    }

    pub fn logout(&self) {
        *self.token.lock().unwrap() = None;
        security::delete(security::keys::SPOTIFY_REFRESH);
    }

    /// Valid access token (refreshing when needed). Also used for librespot.
    pub fn access_token(&self, force_refresh: bool) -> Result<String, ApiError> {
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
            // some endpoints wrap the page (e.g. /me/following → {"artists": {...}})
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
        let url = url::Url::parse_with_params(
            &format!("{API}/search"),
            &[("q", q), ("type", "track,album,artist,playlist"), ("limit", "10")],
        )
        .map_err(|e| ApiError::Other(0, e.to_string()))?;
        let v = self.get(url.as_str())?;
        Ok(parse_search(&v))
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
    fn parses_search() {
        let v = json!({"tracks":{"items":[{"type":"track","id":"t","name":"T","artists":[],"album":{"images":[]}}]},
            "albums":{"items":[{"id":"a","name":"A","artists":[{"name":"X"}],"images":[]}]},
            "artists":{"items":[{"id":"r","name":"R","images":[]}, null]},
            "playlists":{"items":[null,{"id":"p","name":"P","owner":{"id":"o"}}]}});
        let r = parse_search(&v);
        assert_eq!((r.tracks.len(), r.albums.len(), r.artists.len(), r.playlists.len()), (1, 1, 1, 1));
    }
}
