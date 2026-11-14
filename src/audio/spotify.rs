//! Spotify Premium playback through librespot (open-source Spotify Connect
//! client library). Requires a Spotify Premium account. Audio is decoded by
//! librespot and delivered to Aqua Glass' own output pipeline through a custom
//! `Sink`, so EQ, visualizer and volume work uniformly.

use super::engine::EngineEvent;
use super::output::{Feeder, Output};
use crate::config::Quality;
use librespot_core::authentication::Credentials;
use librespot_core::session::Session;
use librespot_core::{SessionConfig, SpotifyUri};
use librespot_playback::audio_backend::{Sink, SinkResult};
use librespot_playback::config::{Bitrate, PlayerConfig};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::NoOpVolume;
use librespot_playback::player::{Player, PlayerEvent};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

pub const SAMPLE_RATE: u32 = 44_100;

struct AquaSink {
    feeder: Feeder,
    gen: Arc<AtomicU64>,
    buf: Vec<f32>,
}

impl Sink for AquaSink {
    fn write(&mut self, packet: AudioPacket, _c: &mut Converter) -> SinkResult<()> {
        if let AudioPacket::Samples(s) = packet {
            self.buf.clear();
            self.buf.extend(s.iter().map(|x| *x as f32));
            self.feeder.set_generation(self.gen.load(Ordering::Acquire));
            self.feeder.write(&self.buf, 2);
        }
        Ok(())
    }
}

pub struct SpotifyPlayer {
    rt: tokio::runtime::Runtime,
    session: Session,
    player: Arc<Player>,
    /// Generation the sink should tag its data with (updated after flushes).
    pub gen: Arc<AtomicU64>,
}

pub struct PlayerOpts {
    pub quality: Quality,
    pub normalisation: bool,
    pub gapless: bool,
}

impl SpotifyPlayer {
    pub fn connect(token: &str, out: &Output, opts: &PlayerOpts, events: mpsc::Sender<EngineEvent>) -> anyhow::Result<SpotifyPlayer> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("spotify").enable_all().build()?;
        let session = Session::new(SessionConfig::default(), None);
        rt.block_on(session.connect(Credentials::with_access_token(token), false))
            .map_err(|e| anyhow::anyhow!("Spotify playback sign-in failed: {e}. Playback requires Spotify Premium."))?;
        let gen = Arc::new(AtomicU64::new(out.generation()));
        let player = {
            let _g = rt.enter();
            let config = PlayerConfig {
                bitrate: match opts.quality {
                    Quality::Low96 => Bitrate::Bitrate96,
                    Quality::Normal160 => Bitrate::Bitrate160,
                    Quality::High320 => Bitrate::Bitrate320,
                },
                gapless: opts.gapless,
                normalisation: opts.normalisation,
                ..Default::default()
            };
            let feeder = Feeder::new(out, SAMPLE_RATE, gen.load(Ordering::Acquire));
            let g2 = gen.clone();
            Player::new(config, session.clone(), Box::new(NoOpVolume), move || {
                Box::new(AquaSink { feeder, gen: g2, buf: Vec::with_capacity(8192) }) as Box<dyn Sink>
            })
        };
        let mut ch = player.get_player_event_channel();
        rt.spawn(async move {
            while let Some(ev) = ch.recv().await {
                let mapped = match ev {
                    PlayerEvent::EndOfTrack { play_request_id, .. } => Some(EngineEvent::SpotifyEnded(play_request_id)),
                    PlayerEvent::Unavailable { play_request_id, .. } => Some(EngineEvent::SpotifyUnavailable(play_request_id)),
                    PlayerEvent::TimeToPreloadNextTrack { .. } => Some(EngineEvent::SpotifyPreload),
                    PlayerEvent::Playing { play_request_id, position_ms, .. } => Some(EngineEvent::SpotifyPlaying(play_request_id, position_ms)),
                    PlayerEvent::PlayRequestIdChanged { play_request_id } => Some(EngineEvent::SpotifyRequestId(play_request_id)),
                    PlayerEvent::SessionDisconnected { .. } => Some(EngineEvent::SpotifyDisconnected),
                    _ => None,
                };
                if let Some(m) = mapped {
                    if events.send(m).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(SpotifyPlayer { rt, session, player, gen })
    }

    pub fn is_valid(&self) -> bool {
        !self.session.is_invalid()
    }

    pub fn load(&self, uri: &str, start_ms: u32) -> anyhow::Result<()> {
        let u = SpotifyUri::from_uri(uri).map_err(|e| anyhow::anyhow!("bad Spotify URI {uri}: {e}"))?;
        let _g = self.rt.enter();
        self.player.load(u, true, start_ms);
        Ok(())
    }

    pub fn preload(&self, uri: &str) {
        if let Ok(u) = SpotifyUri::from_uri(uri) {
            let _g = self.rt.enter();
            self.player.preload(u);
        }
    }

    pub fn play(&self) {
        self.player.play();
    }
    pub fn pause(&self) {
        self.player.pause();
    }
    pub fn stop(&self) {
        self.player.stop();
    }
    pub fn seek(&self, ms: u32) {
        self.player.seek(ms);
    }
}

impl Drop for SpotifyPlayer {
    fn drop(&mut self) {
        self.player.stop();
        self.session.shutdown();
    }
}
