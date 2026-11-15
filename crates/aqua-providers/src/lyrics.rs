//! Lyrics from LRCLIB (https://lrclib.net) — a free, open, community lyrics
//! database with time-synced LRC. Spotify's own lyrics are not
//! available to third-party apps, so this is the licensed-for-reuse source used.
//! Translations are not offered by LRCLIB, so Aqua Glass does not offer them.

use aqua_core::model::Track;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub t_ms: i64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub t_ms: i64,
    pub text: String,
    /// Word timings for enhanced LRC (`<mm:ss.xx>word`).
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    Synced(Vec<Line>),
    Plain(Vec<String>),
    Instrumental,
    NotFound,
}

impl Lyrics {
    /// Index of the current line at `pos_ms` (synced only).
    pub fn current_line(&self, pos_ms: i64) -> Option<usize> {
        match self {
            Lyrics::Synced(lines) => {
                if lines.is_empty() || pos_ms < lines[0].t_ms {
                    return None;
                }
                Some(lines.partition_point(|l| l.t_ms <= pos_ms).saturating_sub(1))
            }
            _ => None,
        }
    }
    pub fn line_text(&self, i: usize) -> Option<&str> {
        match self {
            Lyrics::Synced(l) => l.get(i).map(|l| l.text.as_str()),
            Lyrics::Plain(l) => l.get(i).map(|s| s.as_str()),
            _ => None,
        }
    }
}

fn parse_ts(s: &str) -> Option<i64> {
    // mm:ss(.xx|.xxx)
    let (m, rest) = s.split_once(':')?;
    let m: i64 = m.trim().parse().ok()?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((a, b)) => (a, b),
        None => (rest, "0"),
    };
    let sec: i64 = sec.trim().parse().ok()?;
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 100,
        2 => frac.parse::<i64>().ok()? * 10,
        _ => frac[..3].parse::<i64>().ok()?,
    };
    Some(m * 60_000 + sec * 1000 + frac_ms)
}

/// Parse LRC (incl. multiple timestamps per line, `[offset:]` and enhanced word tags).
pub fn parse_lrc(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut offset = 0i64;
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            if let Some(o) = tag.strip_prefix("offset:") {
                offset = o.trim().parse().unwrap_or(0);
            } else if let Some(t) = parse_ts(tag) {
                stamps.push(t);
            }
            rest = r[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        // enhanced word timing
        let mut words = Vec::new();
        let mut plain = String::new();
        if rest.contains('<') {
            let mut s = rest;
            while let Some(i) = s.find('<') {
                plain.push_str(&s[..i]);
                let Some(j) = s[i..].find('>') else { break };
                let ts = parse_ts(&s[i + 1..i + j]);
                s = &s[i + j + 1..];
                let next = s.find('<').unwrap_or(s.len());
                let w = &s[..next];
                if let Some(t) = ts {
                    if !w.trim().is_empty() {
                        words.push(Word { t_ms: t - offset, text: w.to_string() });
                    }
                }
                plain.push_str(w);
                s = &s[next..];
            }
            plain.push_str(s);
        } else {
            plain = rest.to_string();
        }
        let text = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        for t in stamps {
            out.push(Line { t_ms: t - offset, text: text.clone(), words: words.clone() });
        }
    }
    out.sort_by_key(|l| l.t_ms);
    out
}

#[derive(Deserialize)]
struct LrcLibResp {
    #[serde(default, rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(default, rename = "plainLyrics")]
    plain: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

pub fn from_response(r: &serde_json::Value) -> Lyrics {
    let Ok(r) = serde_json::from_value::<LrcLibResp>(r.clone()) else { return Lyrics::NotFound };
    if r.instrumental {
        return Lyrics::Instrumental;
    }
    if let Some(s) = r.synced.filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(&s);
        if !lines.is_empty() {
            return Lyrics::Synced(lines);
        }
    }
    if let Some(p) = r.plain.filter(|s| !s.trim().is_empty()) {
        return Lyrics::Plain(p.lines().map(|l| l.to_string()).collect());
    }
    Lyrics::NotFound
}

/// Fetch lyrics (blocking). Uses an exact lookup first, then a search.
pub fn fetch(track: &Track) -> anyhow::Result<Lyrics> {
    fetch_inner(track).map_err(|e| {
        if e.downcast_ref::<reqwest::Error>().is_some_and(|r| r.is_connect() || r.is_timeout() || r.is_request()) {
            anyhow::anyhow!("couldn't reach lrclib.net (check your internet connection)")
        } else {
            e
        }
    })
}

fn fetch_inner(track: &Track) -> anyhow::Result<Lyrics> {
    let http = crate::http();
    let artist = track.artists.first().cloned().unwrap_or_default();
    if artist.is_empty() {
        anyhow::bail!("this track has no artist tag, so lyrics can't be looked up");
    }
    let dur = (track.duration_ms / 1000).to_string();
    let mut params = vec![("track_name", track.title.as_str()), ("artist_name", artist.as_str())];
    if !track.album.is_empty() {
        params.push(("album_name", track.album.as_str()));
    }
    if track.duration_ms > 0 {
        params.push(("duration", dur.as_str()));
    }
    let url = url::Url::parse_with_params("https://lrclib.net/api/get", &params)?;
    let resp = http.get(url.as_str()).send()?;
    if resp.status().is_success() {
        let v: serde_json::Value = resp.json()?;
        let l = from_response(&v);
        if l != Lyrics::NotFound {
            return Ok(l);
        }
    } else if resp.status().as_u16() != 404 {
        anyhow::bail!("LRCLIB error {}", resp.status());
    }
    let url = url::Url::parse_with_params("https://lrclib.net/api/search", &[("track_name", track.title.as_str()), ("artist_name", artist.as_str())])?;
    let v: serde_json::Value = http.get(url.as_str()).send()?.json()?;
    // choose the closest duration among results
    let best = v.as_array().into_iter().flatten().min_by_key(|r| {
        let d = (r["duration"].as_f64().unwrap_or(0.0) * 1000.0) as i64;
        (d - track.duration_ms as i64).abs() + if r["syncedLyrics"].is_string() { 0 } else { 5000 }
    });
    Ok(best.map(from_response).unwrap_or(Lyrics::NotFound))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_basic_and_multi() {
        let l = parse_lrc("[ti:x]\n[00:01.50]Hello\n[00:03.00][00:10.00]Again\nnot a line\n[01:02.345] Long  spaces ");
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].t_ms, 1500);
        assert_eq!(l[1].text, "Again");
        assert_eq!(l[2].t_ms, 10_000);
        assert_eq!(l[3].t_ms, 62_345);
        assert_eq!(l[3].text, "Long spaces");
    }
    #[test]
    fn offset_and_words() {
        let l = parse_lrc("[offset:500]\n[00:02.00]<00:02.00>One <00:02.50>two");
        assert_eq!(l[0].t_ms, 1500);
        assert_eq!(l[0].text, "One two");
        assert_eq!(l[0].words.len(), 2);
        assert_eq!(l[0].words[1].t_ms, 2000);
    }
    #[test]
    fn current_line_lookup() {
        let ly = Lyrics::Synced(parse_lrc("[00:01.00]a\n[00:02.00]b\n[00:03.00]c"));
        assert_eq!(ly.current_line(500), None);
        assert_eq!(ly.current_line(1000), Some(0));
        assert_eq!(ly.current_line(2500), Some(1));
        assert_eq!(ly.current_line(99_000), Some(2));
    }
    #[test]
    fn response_variants() {
        assert_eq!(from_response(&serde_json::json!({"instrumental":true})), Lyrics::Instrumental);
        assert!(matches!(from_response(&serde_json::json!({"plainLyrics":"a\nb"})), Lyrics::Plain(v) if v.len()==2));
        assert!(matches!(from_response(&serde_json::json!({"syncedLyrics":"[00:01.00]x","plainLyrics":"x"})), Lyrics::Synced(_)));
        assert_eq!(from_response(&serde_json::json!({})), Lyrics::NotFound);
    }
}
