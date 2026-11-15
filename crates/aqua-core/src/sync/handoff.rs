//! Playback handoff ("Continue on this device") — planning only.
//!
//! A handoff is *not* remote control of a streaming service. The source
//! device publishes a [`HandoffOffer`] (its listening state and queue); the
//! target decides locally, with [`plan_handoff`], what it can actually do
//! with the providers it is signed in to. Pausing the source is a request to
//! the source *Aqua Glass instance* (which pauses its own player) — never a
//! Spotify Connect remote command, which those services don't
//! grant to third-party apps in general.

use super::state::{ListeningState, QueueEntry, QueueState, TrackRef};
use super::DeviceInfo;
use crate::model::ProviderKind;
use serde::{Deserialize, Serialize};

/// Offers older than this are shown without a resume position.
pub const POSITION_TRUST_MS: i64 = 6 * 3600 * 1000;
/// Starting this close to the end isn't useful; start the track over instead.
const NEAR_END_MS: u64 = 5000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandoffOffer {
    pub from: DeviceInfo,
    pub listening: ListeningState,
    pub queue: Option<QueueState>,
}

/// What the *target* device can do. Filled in by the target's platform layer
/// from its own sign-in state — never assumed from the source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Capabilities {
    /// Providers this device can play in-app right now (signed in, Premium where required).
    pub playable: Vec<ProviderKind>,
    /// Local files are available on this device (matched by artist/title).
    pub local_library: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HandoffAction {
    /// Same track, same provider, played in Aqua Glass.
    Resume { track: TrackRef, position_ms: u64 },
    /// The source's provider isn't available here: look the song up on these
    /// providers (by ISRC, then strict artist/title/duration matching) and play
    /// the match — or nothing if no confident match exists.
    ResumeMatched { original: TrackRef, search: Vec<ProviderKind>, position_ms: u64 },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandoffPlan {
    pub action: HandoffAction,
    /// Upcoming entries to continue with (origins preserved), when playing in-app.
    pub upcoming: Vec<QueueEntry>,
    /// Ask the source Aqua Glass to pause once playback starts here.
    pub pause_source: bool,
}

/// Decide what "Continue on this device" does. Pure function of its inputs.
pub fn plan_handoff(offer: &HandoffOffer, caps: &Capabilities, now_ms: i64) -> HandoffPlan {
    let unavailable = |reason: &str| HandoffPlan { action: HandoffAction::Unavailable { reason: reason.into() }, upcoming: vec![], pause_source: false };
    let Some(track) = offer.listening.track.clone() else {
        return unavailable("Nothing is playing on the other device.");
    };
    let fresh = now_ms - offer.listening.updated_ms <= POSITION_TRUST_MS;
    let mut position_ms = if fresh { offer.listening.estimated_position_ms(now_ms) } else { 0 };
    if offer.listening.duration_ms > 0 && position_ms + NEAR_END_MS >= offer.listening.duration_ms {
        position_ms = 0;
    }
    let upcoming = offer.queue.as_ref().map(|q| q.upcoming().to_vec()).unwrap_or_default();
    let in_app = |action| HandoffPlan { action, upcoming: upcoming.clone(), pause_source: true };

    if track.provider != ProviderKind::Local && caps.playable.contains(&track.provider) {
        return in_app(HandoffAction::Resume { track, position_ms });
    }
    let mut search: Vec<ProviderKind> = caps.playable.iter().copied().filter(|p| *p != ProviderKind::Local).collect();
    if caps.local_library {
        search.push(ProviderKind::Local);
    }
    if search.is_empty() {
        return unavailable("This device isn't signed in to a service that can play this song.");
    }
    in_app(HandoffAction::ResumeMatched { original: track, search, position_ms })
}
