//! Cross-provider track matching.
//!
//! Scores candidates by ISRC, title, artists, album and duration, and
//! penalises *version* mismatches (live / remix / remaster / acoustic / cover /
//! clean vs explicit …) so a significantly different recording is never
//! auto-selected: such results land in "ambiguous" or "unmatched" for review.

use crate::model::Track;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchStatus {
    Matched,
    Ambiguous,
    Unmatched,
}

pub const AUTO_THRESHOLD: f32 = 0.82;
pub const REVIEW_THRESHOLD: f32 = 0.55;

const VERSION_WORDS: &[(&str, &str)] = &[
    ("live", "live"),
    ("remix", "remix"),
    ("mix", "remix"),
    ("rmx", "remix"),
    ("remaster", "remaster"),
    ("remastered", "remaster"),
    ("acoustic", "acoustic"),
    ("unplugged", "acoustic"),
    ("instrumental", "instrumental"),
    ("karaoke", "karaoke"),
    ("cover", "cover"),
    ("demo", "demo"),
    ("edit", "edit"),
    ("extended", "extended"),
    ("radio", "edit"),
    ("clean", "clean"),
    ("sped", "speed"),
    ("slowed", "speed"),
    ("nightcore", "speed"),
    ("reverb", "speed"),
    ("orchestral", "orchestral"),
    ("piano", "piano"),
    ("version", "version"),
];

/// Noise found in YouTube titles that says nothing about the recording.
const NOISE: &[&str] = &["official", "video", "audio", "lyrics", "lyric", "music", "hd", "hq", "4k", "mv", "visualizer", "topic", "explicit", "visualiser"];

pub fn normalize(s: &str) -> String {
    let s = s.to_lowercase().replace('&', " and ");
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip "feat. X" and bracketed qualifiers, returning (core title, qualifier text).
pub fn split_qualifiers(title: &str) -> (String, String) {
    let mut core = String::new();
    let mut qual = String::new();
    let mut depth = 0;
    for ch in title.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth > 0 => qual.push(ch),
            _ => core.push(ch),
        }
        if depth > 0 && matches!(ch, '(' | '[' | '{') {
            qual.push(' ');
        }
    }
    // " - Live at Wembley", " - 2011 Remaster"
    if let Some((a, b)) = core.clone().split_once(" - ") {
        core = a.to_string();
        qual.push(' ');
        qual.push_str(b);
    }
    let lc = core.to_lowercase();
    for marker in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = lc.find(marker) {
            qual.push(' ');
            qual.push_str(&core[i..]);
            core.truncate(i);
            break;
        }
    }
    (normalize(&core), normalize(&qual))
}

pub fn version_tags(title: &str, album: &str) -> BTreeSet<&'static str> {
    let (_, q) = split_qualifiers(title);
    let mut tags = BTreeSet::new();
    for w in q.split_whitespace() {
        for (k, tag) in VERSION_WORDS {
            if w == *k {
                tags.insert(*tag);
            }
        }
    }
    let al = normalize(album);
    if al.split_whitespace().any(|w| w == "live") {
        tags.insert("live");
    }
    tags.remove("version");
    tags
}

fn tokens(s: &str) -> BTreeSet<String> {
    normalize(s).split_whitespace().filter(|w| !NOISE.contains(w)).map(|w| w.to_string()).collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let i = a.intersection(b).count() as f32;
    let u = a.union(b).count() as f32;
    if u == 0.0 { 0.0 } else { i / u }
}

pub fn levenshtein_ratio(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let c = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    1.0 - prev[b.len()] as f32 / a.len().max(b.len()) as f32
}

#[derive(Clone, Debug)]
pub struct ScoreDetail {
    pub score: f32,
    pub notes: Vec<String>,
}

pub fn score(src: &Track, cand: &Track) -> ScoreDetail {
    let mut notes = Vec::new();
    if !src.isrc.is_empty() && src.isrc.eq_ignore_ascii_case(&cand.isrc) {
        return ScoreDetail { score: 1.0, notes: vec!["ISRC match".into()] };
    }
    let (sc, _) = split_qualifiers(&src.title);
    let (cc, _) = split_qualifiers(&cand.title);
    // YouTube titles frequently embed the artist ("Artist - Title"); compare against title+artists too
    let title_sim = levenshtein_ratio(&sc, &cc).max(jaccard(&tokens(&sc), &tokens(&cc)));
    let src_artists: BTreeSet<String> = src.artists.iter().flat_map(|a| tokens(a)).collect();
    let cand_artist_text = format!("{} {}", cand.artists.join(" "), cand.title);
    let cand_artists: BTreeSet<String> = tokens(&cand_artist_text);
    let artist_sim = if src_artists.is_empty() {
        0.5
    } else {
        let frac = src_artists.intersection(&cand_artists).count() as f32 / src_artists.len() as f32;
        // services often credit only the primary artist
        let primary = tokens(&src.artists[0]);
        let primary_ok = !primary.is_empty() && primary.iter().all(|t| cand_artists.contains(t));
        if primary_ok { frac.max(0.9) } else { frac }
    };
    let dur_sim = if src.duration_ms == 0 || cand.duration_ms == 0 {
        0.5
    } else {
        let d = (src.duration_ms as i64 - cand.duration_ms as i64).unsigned_abs();
        match d {
            0..=2000 => 1.0,
            2001..=5000 => 0.8,
            5001..=10000 => 0.4,
            _ => 0.0,
        }
    };
    let album_sim = if src.album.is_empty() || cand.album.is_empty() { 0.5 } else { jaccard(&tokens(&src.album), &tokens(&cand.album)) };
    let mut s = 0.45 * title_sim + 0.30 * artist_sim + 0.17 * dur_sim + 0.08 * album_sim;

    let st = version_tags(&src.title, &src.album);
    let ct = version_tags(&cand.title, &cand.album);
    let diff: Vec<&&str> = st.symmetric_difference(&ct).collect();
    for d in &diff {
        let pen = match **d {
            "remaster" => 0.04,
            "edit" | "extended" => 0.12,
            _ => 0.35,
        };
        s -= pen;
        notes.push(format!("version differs: {d}"));
    }
    if src.explicit && cand.explicit != src.explicit && !cand.album.is_empty() {
        s -= 0.1;
        notes.push("explicit vs clean".into());
    }
    if dur_sim == 0.0 {
        notes.push("duration differs by >10s".into());
    }
    ScoreDetail { score: s.clamp(0.0, 1.0), notes }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub track: Track,
    pub score: f32,
    pub notes: Vec<String>,
}

/// Rank candidates and classify.
pub fn rank(src: &Track, cands: Vec<Track>) -> (Vec<Candidate>, MatchStatus) {
    let mut v: Vec<Candidate> = cands
        .into_iter()
        .filter(|c| !c.unavailable)
        .map(|c| {
            let d = score(src, &c);
            Candidate { track: c, score: d.score, notes: d.notes }
        })
        .collect();
    v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup_by(|a, b| a.track.id == b.track.id);
    let status = match v.first() {
        None => MatchStatus::Unmatched,
        Some(best) => {
            let margin = v.get(1).map(|s| best.score - s.score).unwrap_or(1.0);
            if best.score >= AUTO_THRESHOLD && (margin >= 0.03 || best.score >= 0.97) && best.notes.iter().all(|n| !n.starts_with("version")) {
                MatchStatus::Matched
            } else if best.score >= REVIEW_THRESHOLD {
                MatchStatus::Ambiguous
            } else {
                MatchStatus::Unmatched
            }
        }
    };
    (v, status)
}

/// Search query for a track on another service.
pub fn query_for(t: &Track) -> String {
    let (core, _) = split_qualifiers(&t.title);
    let artist = t.artists.first().map(|a| a.as_str()).unwrap_or("");
    format!("{core} {artist}").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;

    fn tr(title: &str, artist: &str, album: &str, dur: u64) -> Track {
        Track { title: title.into(), artists: vec![artist.into()], album: album.into(), duration_ms: dur, id: format!("{title}{artist}{dur}"), provider: ProviderKind::YouTube, ..Default::default() }
    }

    #[test]
    fn qualifiers() {
        assert_eq!(split_qualifiers("Song (feat. X) [Live]"), ("song".into(), "feat x live".into()));
        assert_eq!(split_qualifiers("Song - 2011 Remaster").0, "song");
        assert!(version_tags("Song - Live at Wembley", "").contains("live"));
        assert!(version_tags("Song", "Live in Paris").contains("live"));
        assert!(version_tags("Song (Remastered 2009)", "").contains("remaster"));
        assert!(version_tags("Song (Official Video)", "").is_empty());
    }

    #[test]
    fn isrc_wins() {
        let mut a = tr("X", "Y", "", 1000);
        let mut b = tr("Completely different", "Z", "", 99999);
        a.isrc = "GBAYE0601498".into();
        b.isrc = "gbaye0601498".into();
        assert_eq!(score(&a, &b).score, 1.0);
    }

    #[test]
    fn exact_match_preferred_over_live_and_remix() {
        let src = tr("Bohemian Rhapsody", "Queen", "A Night at the Opera", 354_000);
        let cands = vec![
            tr("Bohemian Rhapsody (Live Aid 1985)", "Queen", "", 360_000),
            tr("Queen - Bohemian Rhapsody (Official Video Remastered)", "Queen Official", "", 359_000),
            tr("Bohemian Rhapsody", "Queen", "", 355_000),
            tr("Bohemian Rhapsody (Remix)", "DJ Someone", "", 210_000),
        ];
        let (r, st) = rank(&src, cands);
        assert_eq!(r[0].track.title, "Bohemian Rhapsody");
        assert_eq!(st, MatchStatus::Matched);
    }

    #[test]
    fn only_live_version_is_not_auto_matched() {
        let src = tr("Wonderwall", "Oasis", "Morning Glory", 258_000);
        let (r, st) = rank(&src, vec![tr("Wonderwall (Live at Knebworth)", "Oasis", "", 270_000)]);
        assert_ne!(st, MatchStatus::Matched, "live recording must need review: {:?}", r[0]);
    }

    #[test]
    fn cover_not_auto_matched_and_garbage_unmatched() {
        let src = tr("Hallelujah", "Leonard Cohen", "Various Positions", 279_000);
        let (_, st) = rank(&src, vec![tr("Hallelujah (Cover)", "Random Singer", "", 250_000)]);
        assert_ne!(st, MatchStatus::Matched);
        let (_, st) = rank(&src, vec![tr("Totally Unrelated", "Nobody", "", 100_000)]);
        assert_eq!(st, MatchStatus::Unmatched);
        let (_, st) = rank(&src, vec![]);
        assert_eq!(st, MatchStatus::Unmatched);
    }

    #[test]
    fn youtube_style_titles_match() {
        let src = tr("Get Lucky", "Daft Punk", "Random Access Memories", 369_000);
        let mut c = tr("Get Lucky", "Daft Punk", "", 369_500);
        c.artists = vec!["Daft Punk - Topic".replace(" - Topic", "")];
        let (_, st) = rank(&src, vec![c]);
        assert_eq!(st, MatchStatus::Matched);
        let src2 = tr("Get Lucky (feat. Pharrell Williams)", "Daft Punk", "", 369_000);
        let c2 = tr("Get Lucky (Official Audio) ft. Pharrell Williams, Nile Rodgers", "Daft Punk", "", 368_000);
        let (r, _) = rank(&src2, vec![c2]);
        assert!(r[0].score > REVIEW_THRESHOLD, "{}", r[0].score);
    }

    #[test]
    fn levenshtein() {
        assert_eq!(levenshtein_ratio("abc", "abc"), 1.0);
        assert!(levenshtein_ratio("kitten", "sitting") > 0.5);
        assert_eq!(query_for(&tr("Song (feat. X)", "Art", "", 0)), "song Art");
    }
}
