//! Last.fm web services (read-only, API key only — no user account needed):
//! `track.getSimilar`, `artist.getSimilar`, `artist.getTopTracks`.
//! Used as the similarity source for Similarity Radio. Responses are kept in a
//! small bounded cache. Get a free API key at https://www.last.fm/api/account/create.

use std::collections::VecDeque;
use std::sync::Mutex;

const API: &str = "https://ws.audioscrobbler.com/2.0/";
const CACHE_MAX: usize = 48;

#[derive(Clone, Debug, PartialEq)]
pub struct Similar {
    pub artist: String,
    pub title: String,
    /// 0..1 similarity (1 for top tracks, which are scored by the caller).
    pub score: f32,
    /// Last.fm listener count when known (smaller = less known).
    pub listeners: Option<u64>,
    pub rank: usize,
}

pub struct LastFm {
    key: String,
    cache: Mutex<VecDeque<(String, serde_json::Value)>>,
}

fn num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub fn parse_similar_tracks(v: &serde_json::Value) -> Vec<Similar> {
    v["similartracks"]["track"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, t)| {
            let title = t["name"].as_str()?.to_string();
            let artist = t["artist"]["name"].as_str()?.to_string();
            Some(Similar { artist, title, score: num(&t["match"]).unwrap_or(0.0) as f32, listeners: num(&t["playcount"]).map(|x| x as u64), rank: i })
        })
        .collect()
}

pub fn parse_similar_artists(v: &serde_json::Value) -> Vec<(String, f32)> {
    v["similarartists"]["artist"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?.to_string(), num(&a["match"]).unwrap_or(0.0) as f32)))
        .collect()
}

pub fn parse_top_tracks(v: &serde_json::Value) -> Vec<Similar> {
    v["toptracks"]["track"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Similar {
                title: t["name"].as_str()?.to_string(),
                artist: t["artist"]["name"].as_str()?.to_string(),
                score: 1.0,
                listeners: num(&t["listeners"]).map(|x| x as u64),
                rank: i,
            })
        })
        .collect()
}

impl LastFm {
    pub fn new(key: &str) -> LastFm {
        LastFm { key: key.trim().to_string(), cache: Mutex::new(VecDeque::new()) }
    }

    fn call(&self, params: &[(&str, &str)]) -> anyhow::Result<serde_json::Value> {
        let mut all: Vec<(&str, &str)> = params.to_vec();
        all.push(("api_key", &self.key));
        all.push(("format", "json"));
        all.push(("autocorrect", "1"));
        let url = url::Url::parse_with_params(API, &all)?;
        let ck = params.iter().map(|(a, b)| format!("{a}={}", b.to_lowercase())).collect::<Vec<_>>().join("&");
        if let Some((_, v)) = self.cache.lock().unwrap().iter().find(|(k, _)| *k == ck) {
            return Ok(v.clone());
        }
        let resp = crate::http().get(url.as_str()).send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                anyhow::anyhow!("couldn't reach Last.fm (network)")
            } else {
                anyhow::anyhow!("Last.fm request failed")
            }
        })?;
        let status = resp.status();
        let v: serde_json::Value = resp.json().unwrap_or_default();
        if let Some(code) = v["error"].as_i64() {
            let msg = v["message"].as_str().unwrap_or("error");
            match code {
                10 | 26 => anyhow::bail!("Last.fm API key is invalid or suspended"),
                29 => anyhow::bail!("Last.fm rate limit — retrying later"),
                6 => return Ok(serde_json::Value::Null), // not found › empty
                _ => anyhow::bail!("Last.fm error {code}: {msg}"),
            }
        }
        if !status.is_success() {
            anyhow::bail!("Last.fm HTTP {status}");
        }
        let mut c = self.cache.lock().unwrap();
        c.push_back((ck, v.clone()));
        while c.len() > CACHE_MAX {
            c.pop_front();
        }
        Ok(v)
    }

    pub fn similar_tracks(&self, artist: &str, title: &str, limit: u32) -> anyhow::Result<Vec<Similar>> {
        let l = limit.to_string();
        Ok(parse_similar_tracks(&self.call(&[("method", "track.getsimilar"), ("artist", artist), ("track", title), ("limit", &l)])?))
    }

    pub fn similar_artists(&self, artist: &str, limit: u32) -> anyhow::Result<Vec<(String, f32)>> {
        let l = limit.to_string();
        Ok(parse_similar_artists(&self.call(&[("method", "artist.getsimilar"), ("artist", artist), ("limit", &l)])?))
    }

    pub fn top_tracks(&self, artist: &str, limit: u32) -> anyhow::Result<Vec<Similar>> {
        let l = limit.to_string();
        Ok(parse_top_tracks(&self.call(&[("method", "artist.gettoptracks"), ("artist", artist), ("limit", &l)])?))
    }

    /// Validate the key with a cheap call.
    pub fn check(&self) -> anyhow::Result<()> {
        self.similar_artists("Radiohead", 1).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    // Shapes follow the documented Last.fm JSON responses (numbers sometimes as strings).
    #[test]
    fn parses_documented_shapes() {
        let st = json!({"similartracks":{"track":[
            {"name":"Strong Enough","playcount":291817,"match":1,"artist":{"name":"Cher"}},
            {"name":"Hey Mickey","playcount":"120","match":"0.53","artist":{"name":"Toni Basil"}},
            {"name":"broken"}]}});
        let v = parse_similar_tracks(&st);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].score, 0.53);
        assert_eq!(v[1].listeners, Some(120));
        let sa = json!({"similarartists":{"artist":[{"name":"Madonna","match":"1"},{"name":"Kylie","match":0.8}]}});
        assert_eq!(parse_similar_artists(&sa), vec![("Madonna".to_string(), 1.0), ("Kylie".to_string(), 0.8)]);
        let tt = json!({"toptracks":{"track":[{"name":"Believe","listeners":"1200000","artist":{"name":"Cher"}}]}});
        let t = parse_top_tracks(&tt);
        assert_eq!(t[0].listeners, Some(1_200_000));
        assert!(parse_similar_tracks(&serde_json::Value::Null).is_empty());
    }
}
