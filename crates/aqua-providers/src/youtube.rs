//! YouTube Music integration through the official YouTube Data API v3.
//!
//! What Google officially offers third parties:
//!  * OAuth sign-in (you create a free Google Cloud "Desktop app" OAuth client),
//!  * your playlists, liked videos, search, playlist creation/editing.
//!
//! What it does NOT offer: a YouTube Music-specific API, album objects, or an
//! audio stream API. The YouTube Terms of Service forbid extracting/downloading
//! audio outside YouTube's own player. Aqua Glass therefore opens YouTube tracks
//! in YouTube Music (or YouTube) in the browser for playback; this is documented
//! in the UI and README. Default quota is 10,000 units/day: search = 100 units,
//! playlist inserts = 50 units, reads = 1 unit.

use super::oauth::{self, OAuthSpec, Token};
use super::{request_json, ApiError};
use aqua_core::model::ProviderKind;
use aqua_core::model::*;
use aqua_core::secrets as security;
use serde_json::{json, Value};
use std::sync::Mutex;

const API: &str = "https://www.googleapis.com/youtube/v3";
pub const SCOPE: &str = "https://www.googleapis.com/auth/youtube";

pub struct YouTubeApi {
    spec: OAuthSpec,
    token: Mutex<Option<Token>>,
}

#[derive(Clone, Debug, Default)]
pub struct Channel {
    pub id: String,
    pub title: String,
}

impl YouTubeApi {
    pub fn new(client_id: &str) -> YouTubeApi {
        YouTubeApi {
            spec: OAuthSpec {
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                client_id: client_id.trim().to_string(),
                client_secret: security::get(security::keys::YOUTUBE_CLIENT_SECRET),
                scopes: vec![SCOPE.into()],
                port: 0,
                path: "/callback".into(),
                extra_auth_params: vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
            },
            token: Mutex::new(None),
        }
    }

    pub fn configured(&self) -> bool {
        !self.spec.client_id.is_empty()
    }

    pub fn login(&self) -> anyhow::Result<()> {
        if !self.configured() {
            anyhow::bail!("Enter your Google OAuth client id first (Settings › Music Services › YouTube Music).");
        }
        let r = oauth::authorize(&self.spec, crate::http())?;
        let t = Token::from_response(r, None);
        if let Some(rt) = &t.refresh {
            security::set(security::keys::YOUTUBE_REFRESH, rt)?;
        }
        *self.token.lock().unwrap() = Some(t);
        Ok(())
    }

    pub fn restore(&self) -> anyhow::Result<bool> {
        if !self.configured() {
            return Ok(false);
        }
        let Some(rt) = security::get(security::keys::YOUTUBE_REFRESH) else { return Ok(false) };
        let r = oauth::refresh(&self.spec, crate::http(), &rt)?;
        *self.token.lock().unwrap() = Some(Token::from_response(r, Some(rt)));
        Ok(true)
    }

    pub fn is_signed_in(&self) -> bool {
        self.token.lock().unwrap().is_some()
    }

    /// Revoke the grant at Google and forget local tokens.
    pub fn logout(&self) {
        let rt = security::get(security::keys::YOUTUBE_REFRESH);
        if let Some(rt) = rt {
            let _ = crate::http().post("https://oauth2.googleapis.com/revoke").form(&[("token", rt)]).send();
        }
        *self.token.lock().unwrap() = None;
        security::delete(security::keys::YOUTUBE_REFRESH);
    }

    fn access_token(&self, force: bool) -> Result<String, ApiError> {
        let (cur, rt) = {
            let g = self.token.lock().unwrap();
            match &*g {
                Some(t) => (if t.expired() || force { None } else { Some(t.access.clone()) }, t.refresh.clone()),
                None => return Err(ApiError::Unauthorized),
            }
        };
        if let Some(a) = cur {
            return Ok(a);
        }
        let rt = rt.ok_or(ApiError::Unauthorized)?;
        let r = oauth::refresh(&self.spec, crate::http(), &rt).map_err(|_| ApiError::Unauthorized)?;
        let t = Token::from_response(r, Some(rt));
        let a = t.access.clone();
        *self.token.lock().unwrap() = Some(t);
        Ok(a)
    }

    fn get(&self, url: &str) -> Result<Value, ApiError> {
        request_json(reqwest::Method::GET, url, None, &|f| self.access_token(f))
    }

    fn paged(&self, base: &str, max_items: usize, mut f: impl FnMut(&Value)) -> Result<(), ApiError> {
        let mut token: Option<String> = None;
        let mut n = 0;
        loop {
            let url = match &token {
                Some(t) => format!("{base}&pageToken={t}"),
                None => base.to_string(),
            };
            let v = self.get(&url)?;
            for it in v["items"].as_array().into_iter().flatten() {
                f(it);
                n += 1;
            }
            token = v["nextPageToken"].as_str().map(|s| s.to_string());
            if token.is_none() || n >= max_items {
                return Ok(());
            }
        }
    }

    pub fn channel(&self) -> Result<Channel, ApiError> {
        let v = self.get(&format!("{API}/channels?part=snippet&mine=true"))?;
        let c = &v["items"][0];
        Ok(Channel { id: c["id"].as_str().unwrap_or_default().into(), title: c["snippet"]["title"].as_str().unwrap_or_default().into() })
    }

    pub fn playlists(&self) -> Result<Vec<Playlist>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/playlists?part=snippet,contentDetails&mine=true&maxResults=50"), 5000, |it| {
            out.push(parse_playlist(it, true))
        })?;
        Ok(out)
    }

    /// Videos in a playlist, in order, with durations (one extra `videos` call per 50 items).
    pub fn playlist_items(&self, playlist_id: &str) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::new();
        self.paged(
            &format!("{API}/playlistItems?part=snippet,contentDetails&maxResults=50&playlistId={playlist_id}"),
            20_000,
            |it| {
                if let Some(t) = parse_playlist_item(it) {
                    out.push(t)
                }
            },
        )?;
        self.fill_durations(&mut out)?;
        Ok(out)
    }

    /// "Liked music": videos the user rated "like", restricted to the Music category where possible.
    pub fn liked(&self) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::new();
        self.paged(&format!("{API}/videos?part=snippet,contentDetails&myRating=like&maxResults=50"), 5000, |it| {
            if it["snippet"]["categoryId"].as_str() == Some("10") {
                out.push(parse_video(it))
            }
        })?;
        Ok(out)
    }

    fn fill_durations(&self, tracks: &mut [Track]) -> Result<(), ApiError> {
        for chunk in tracks.chunks_mut(50) {
            let ids: Vec<&str> = chunk.iter().map(|t| t.id.as_str()).collect();
            let v = self.get(&format!("{API}/videos?part=contentDetails&id={}", ids.join(",")))?;
            for it in v["items"].as_array().into_iter().flatten() {
                let id = it["id"].as_str().unwrap_or_default();
                let d = parse_iso8601_duration(it["contentDetails"]["duration"].as_str().unwrap_or(""));
                for t in chunk.iter_mut().filter(|t| t.id == id) {
                    t.duration_ms = d;
                }
            }
            // Videos missing from the response were deleted or made private.
            for t in chunk.iter_mut() {
                if !v["items"].as_array().into_iter().flatten().any(|i| i["id"].as_str() == Some(&t.id)) {
                    t.unavailable = true;
                }
            }
        }
        Ok(())
    }

    /// Music-category video search (100 quota units per call).
    pub fn search_tracks(&self, q: &str, limit: u32) -> Result<Vec<Track>, ApiError> {
        let lim = limit.clamp(1, 25).to_string();
        let url = url::Url::parse_with_params(
            &format!("{API}/search"),
            &[("part", "snippet"), ("type", "video"), ("videoCategoryId", "10"), ("maxResults", lim.as_str()), ("q", q)],
        )
        .map_err(|e| ApiError::Other(0, e.to_string()))?;
        let v = self.get(url.as_str())?;
        let mut out: Vec<Track> = v["items"].as_array().into_iter().flatten().filter_map(parse_search_item).collect();
        self.fill_durations(&mut out)?;
        Ok(out)
    }

    pub fn search(&self, q: &str) -> Result<SearchResults, ApiError> {
        Ok(SearchResults { tracks: self.search_tracks(q, 15)?, ..Default::default() })
    }

    pub fn create_playlist(&self, name: &str, description: &str) -> Result<String, ApiError> {
        let body = json!({"snippet": {"title": name, "description": description}, "status": {"privacyStatus": "private"}});
        let v = request_json(reqwest::Method::POST, &format!("{API}/playlists?part=snippet,status"), Some(&body), &|f| self.access_token(f))?;
        v["id"].as_str().map(|s| s.to_string()).ok_or(ApiError::Other(0, "no playlist id".into()))
    }

    pub fn add_item(&self, playlist_id: &str, video_id: &str) -> Result<(), ApiError> {
        let body = json!({"snippet": {"playlistId": playlist_id, "resourceId": {"kind": "youtube#video", "videoId": video_id}}});
        request_json(reqwest::Method::POST, &format!("{API}/playlistItems?part=snippet"), Some(&body), &|f| self.access_token(f))?;
        Ok(())
    }

    /// Delete a playlist created by Aqua Glass (only on explicit user request).
    pub fn delete_playlist(&self, playlist_id: &str) -> Result<(), ApiError> {
        request_json(reqwest::Method::DELETE, &format!("{API}/playlists?id={playlist_id}"), None, &|f| self.access_token(f))?;
        Ok(())
    }

    pub fn like(&self, video_id: &str) -> Result<(), ApiError> {
        request_json(reqwest::Method::POST, &format!("{API}/videos/rate?id={video_id}&rating=like"), None, &|f| self.access_token(f))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Parsing

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn thumb(sn: &Value) -> String {
    for k in ["medium", "high", "default", "standard"] {
        if let Some(u) = sn["thumbnails"][k]["url"].as_str() {
            return u.to_string();
        }
    }
    String::new()
}

/// "Artist - Topic" channels belong to auto-generated YouTube Music uploads.
pub fn clean_channel(c: &str) -> String {
    c.trim_end_matches(" - Topic").trim_end_matches("VEVO").trim().to_string()
}

/// Many music videos are titled "Artist - Title (Official Video)"; split when the channel is not a Topic channel.
pub fn split_title(title: &str, channel: &str) -> (String, Vec<String>) {
    let ch = clean_channel(channel);
    if !channel.ends_with(" - Topic") {
        if let Some((a, t)) = title.split_once(" - ") {
            return (t.trim().to_string(), vec![a.trim().to_string()]);
        }
    }
    (title.to_string(), if ch.is_empty() { vec![] } else { vec![ch] })
}

pub fn parse_playlist(v: &Value, mine: bool) -> Playlist {
    let sn = &v["snippet"];
    Playlist {
        provider: ProviderKind::YouTube,
        id: s(&v["id"]),
        name: s(&sn["title"]),
        description: s(&sn["description"]),
        owner: s(&sn["channelTitle"]),
        owned_by_user: mine,
        collaborative: false,
        public: None,
        artwork_url: thumb(sn),
        track_count: v["contentDetails"]["itemCount"].as_u64().unwrap_or(0) as u32,
        snapshot: s(&v["etag"]),
        is_liked: false,
        items_error: String::new(),
    }
}

pub fn parse_playlist_item(v: &Value) -> Option<Track> {
    let sn = &v["snippet"];
    let vid = v["contentDetails"]["videoId"].as_str().or(sn["resourceId"]["videoId"].as_str())?;
    let title = s(&sn["title"]);
    let deleted = title == "Deleted video" || title == "Private video";
    let ch = sn["videoOwnerChannelTitle"].as_str().unwrap_or_default();
    let (t, artists) = split_title(&title, ch);
    Some(Track {
        provider: ProviderKind::YouTube,
        id: vid.to_string(),
        title: t,
        artists,
        artwork_url: thumb(sn),
        unavailable: deleted,
        ..Default::default()
    })
}

pub fn parse_video(v: &Value) -> Track {
    let sn = &v["snippet"];
    let (t, artists) = split_title(&s(&sn["title"]), sn["channelTitle"].as_str().unwrap_or_default());
    Track {
        provider: ProviderKind::YouTube,
        id: s(&v["id"]),
        title: t,
        artists,
        artwork_url: thumb(sn),
        duration_ms: parse_iso8601_duration(v["contentDetails"]["duration"].as_str().unwrap_or("")),
        ..Default::default()
    }
}

pub fn parse_search_item(v: &Value) -> Option<Track> {
    let id = v["id"]["videoId"].as_str()?;
    let sn = &v["snippet"];
    let (t, artists) = split_title(&html_unescape(&s(&sn["title"])), sn["channelTitle"].as_str().unwrap_or_default());
    Some(Track { provider: ProviderKind::YouTube, id: id.into(), title: t, artists, artwork_url: thumb(sn), ..Default::default() })
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

/// Parse ISO 8601 durations like `PT1H2M3S` into milliseconds.
pub fn parse_iso8601_duration(d: &str) -> u64 {
    let Some(rest) = d.strip_prefix('P') else { return 0 };
    let mut total = 0u64;
    let mut num = String::new();
    let mut in_time = false;
    for ch in rest.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' | '.' => num.push(ch),
            u => {
                let n: f64 = num.parse().unwrap_or(0.0);
                num.clear();
                let secs = match (u, in_time) {
                    ('D', false) => n * 86400.0,
                    ('W', false) => n * 604800.0,
                    ('H', true) => n * 3600.0,
                    ('M', true) => n * 60.0,
                    ('S', true) => n,
                    _ => 0.0,
                };
                total += (secs * 1000.0) as u64;
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations() {
        assert_eq!(parse_iso8601_duration("PT3M21S"), 201_000);
        assert_eq!(parse_iso8601_duration("PT1H"), 3_600_000);
        assert_eq!(parse_iso8601_duration("P1DT1S"), 86_401_000);
        assert_eq!(parse_iso8601_duration("garbage"), 0);
    }
    #[test]
    fn titles() {
        assert_eq!(split_title("Song", "Artist - Topic"), ("Song".into(), vec!["Artist".to_string()]));
        assert_eq!(split_title("Artist - Song (Official Video)", "ArtistVEVO"), ("Song (Official Video)".into(), vec!["Artist".to_string()]));
    }
    #[test]
    fn playlist_item() {
        let v = json!({"snippet":{"title":"Song","videoOwnerChannelTitle":"Band - Topic","thumbnails":{"medium":{"url":"u"}}},"contentDetails":{"videoId":"abc"}});
        let t = parse_playlist_item(&v).unwrap();
        assert_eq!(t.id, "abc");
        assert_eq!(t.artists, vec!["Band"]);
        let d = json!({"snippet":{"title":"Deleted video"},"contentDetails":{"videoId":"x"}});
        assert!(parse_playlist_item(&d).unwrap().unavailable);
    }
}
