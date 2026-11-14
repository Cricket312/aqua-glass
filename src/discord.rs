//! Discord Rich Presence over local IPC (named pipe `\\.\pipe\discord-ipc-N`).
//!
//! The worker thread exists only while the feature is enabled. Updates are sent
//! only when something visible changes (track, play/pause) and at most once
//! every 5 seconds, matching Discord's rate limits. If Discord isn't running,
//! connection attempts back off to once a minute.

use crate::audio::engine::PlayerState;
use crate::config::ProviderKind;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiscordOpts {
    pub app_id: String,
    pub title: bool,
    pub artist: bool,
    pub album: bool,
    pub artwork: bool,
    pub elapsed: bool,
    pub service: bool,
    pub paused_status: bool,
    pub privacy: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presence {
    pub details: String,
    pub state: String,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub start_unix: Option<i64>,
}

/// Pure function: what should be shown (None = clear presence).
pub fn build(st: &PlayerState, pos_ms: u64, o: &DiscordOpts) -> Option<Presence> {
    let t = st.current.as_ref()?;
    if !st.playing && !o.paused_status && !st.external {
        return None;
    }
    if o.privacy {
        return Some(Presence { details: "Listening to music".into(), state: String::new(), large_image: None, large_text: None, start_unix: None });
    }
    let mut details = if o.title { t.title.clone() } else { "Listening to music".into() };
    if !st.playing && !st.external {
        details = format!("⏸ {details}");
    }
    let mut state_parts = Vec::new();
    if o.artist && !t.artists.is_empty() {
        state_parts.push(format!("by {}", t.artist_line()));
    }
    if o.service {
        state_parts.push(format!("on {}", t.provider.label()));
    }
    let large_image = if o.artwork && t.provider != ProviderKind::Local && t.artwork_url.starts_with("https://") { Some(t.artwork_url.clone()) } else { None };
    let large_text = if o.album && !t.album.is_empty() { Some(t.album.clone()) } else { None };
    let start_unix = if o.elapsed && st.playing { Some(crate::util::now_unix() - (pos_ms / 1000) as i64) } else { None };
    let clip = |s: String| -> String { if s.chars().count() > 120 { s.chars().take(120).collect() } else if s.chars().count() < 2 { format!("{s}  ") } else { s } };
    Some(Presence { details: clip(details), state: clip(state_parts.join(" ")), large_image, large_text, start_unix })
}

pub enum Msg {
    State(Box<PlayerState>, u64),
    Opts(DiscordOpts),
    Stop,
}

pub struct Discord {
    tx: mpsc::Sender<Msg>,
}

impl Discord {
    pub fn start(opts: DiscordOpts) -> Discord {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("discord".into()).spawn(move || worker(rx, opts)).expect("discord thread");
        Discord { tx }
    }
    pub fn update(&self, st: &PlayerState, pos_ms: u64) {
        let _ = self.tx.send(Msg::State(Box::new(st.clone()), pos_ms));
    }
    pub fn set_opts(&self, o: DiscordOpts) {
        let _ = self.tx.send(Msg::Opts(o));
    }
}

impl Drop for Discord {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

#[cfg(feature = "discord")]
fn worker(rx: mpsc::Receiver<Msg>, mut opts: DiscordOpts) {
    use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
    let mut client: Option<DiscordIpcClient> = None;
    let mut last_attempt: Option<Instant> = None;
    let mut last_sent: Option<Presence> = None;
    let mut last_send_at = Instant::now() - Duration::from_secs(60);
    let mut wanted: Option<Presence> = None;
    let mut dirty = false;
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Opts(o)) => {
                if o.app_id != opts.app_id {
                    if let Some(mut c) = client.take() {
                        let _ = c.close();
                    }
                }
                opts = o;
                dirty = true;
            }
            Ok(Msg::State(st, pos)) => {
                let p = build(&st, pos, &opts);
                // ignore pure timestamp drift (< 3 s) to avoid needless updates
                let same = match (&p, &wanted) {
                    (Some(a), Some(b)) => a.details == b.details && a.state == b.state && a.large_image == b.large_image && a.start_unix.zip(b.start_unix).is_none_or(|(x, y)| (x - y).abs() < 3) && a.start_unix.is_some() == b.start_unix.is_some(),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    wanted = p;
                    dirty = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !dirty || opts.app_id.trim().is_empty() {
            continue;
        }
        if last_send_at.elapsed() < Duration::from_secs(5) {
            continue;
        }
        if client.is_none() {
            if last_attempt.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
                continue;
            }
            last_attempt = Some(Instant::now());
            let mut c = DiscordIpcClient::new(opts.app_id.trim());
            if c.connect().is_ok() {
                client = Some(c);
                last_sent = None;
            } else {
                continue; // Discord not running
            }
        }
        let c = client.as_mut().unwrap();
        let res = match &wanted {
            None => c.clear_activity(),
            Some(p) => {
                if last_sent.as_ref() == Some(p) {
                    dirty = false;
                    continue;
                }
                let mut a = activity::Activity::new().activity_type(activity::ActivityType::Listening).details(p.details.as_str());
                if !p.state.trim().is_empty() {
                    a = a.state(p.state.as_str());
                }
                if p.large_image.is_some() || p.large_text.is_some() {
                    let mut assets = activity::Assets::new();
                    if let Some(i) = &p.large_image {
                        assets = assets.large_image(i.as_str());
                    }
                    if let Some(t) = &p.large_text {
                        assets = assets.large_text(t.as_str());
                    }
                    a = a.assets(assets);
                }
                if let Some(s) = p.start_unix {
                    a = a.timestamps(activity::Timestamps::new().start(s));
                }
                c.set_activity(a)
            }
        };
        last_send_at = Instant::now();
        match res {
            Ok(()) => {
                last_sent = wanted.clone();
                dirty = false;
            }
            Err(_) => {
                client = None; // Discord closed; retry later
            }
        }
    }
    if let Some(mut c) = client {
        let _ = c.clear_activity();
        let _ = c.close();
    }
}

#[cfg(not(feature = "discord"))]
fn worker(rx: mpsc::Receiver<Msg>, _opts: DiscordOpts) {
    while let Ok(m) = rx.recv() {
        if matches!(m, Msg::Stop) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;

    fn st(playing: bool) -> PlayerState {
        PlayerState {
            current: Some(Track { title: "Song".into(), artists: vec!["Art".into()], album: "Alb".into(), provider: ProviderKind::Spotify, artwork_url: "https://i.scdn.co/x".into(), ..Default::default() }),
            playing,
            ..Default::default()
        }
    }
    fn all() -> DiscordOpts {
        DiscordOpts { app_id: "1".into(), title: true, artist: true, album: true, artwork: true, elapsed: true, service: true, paused_status: true, privacy: false }
    }

    #[test]
    fn full_presence() {
        let p = build(&st(true), 10_000, &all()).unwrap();
        assert_eq!(p.details, "Song");
        assert_eq!(p.state, "by Art on Spotify");
        assert_eq!(p.large_text.as_deref(), Some("Alb"));
        assert!(p.large_image.is_some());
        assert!(p.start_unix.is_some());
    }

    #[test]
    fn privacy_and_options() {
        let p = build(&st(true), 0, &DiscordOpts { privacy: true, ..all() }).unwrap();
        assert_eq!(p.details, "Listening to music");
        assert!(p.large_image.is_none() && p.state.is_empty());
        let p = build(&st(true), 0, &DiscordOpts { artist: false, service: false, artwork: false, album: false, elapsed: false, ..all() }).unwrap();
        assert_eq!(p.state.trim(), "");
        assert!(p.large_image.is_none() && p.large_text.is_none() && p.start_unix.is_none());
    }

    #[test]
    fn paused_behaviour() {
        assert!(build(&st(false), 0, &DiscordOpts { paused_status: false, ..all() }).is_none());
        let p = build(&st(false), 0, &all()).unwrap();
        assert!(p.details.starts_with('⏸'));
        assert!(p.start_unix.is_none());
        assert!(build(&PlayerState::default(), 0, &all()).is_none());
    }
}
