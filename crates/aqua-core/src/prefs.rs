//! Portable user preferences that are worth sharing between devices
//! (Similarity Radio). Device-specific settings (audio device, window layout,
//! Windows integrations) are *not* here; see `crate::sync::SettingsState`.

use serde::{Deserialize, Serialize};

/// Similarity Radio discovery strength.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Strength {
    Familiar,
    #[default]
    Balanced,
    Adventurous,
    DeepDiscovery,
}

impl Strength {
    pub const ALL: [Strength; 4] = [Strength::Familiar, Strength::Balanced, Strength::Adventurous, Strength::DeepDiscovery];
    pub fn label(self) -> &'static str {
        match self {
            Strength::Familiar => "Familiar",
            Strength::Balanced => "Balanced",
            Strength::Adventurous => "Adventurous",
            Strength::DeepDiscovery => "Deep Discovery",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Strength::Familiar => "Very similar music — the closest matches to the seed track.",
            Strength::Balanced => "Similar music with some new artists from closely related acts.",
            Strength::Adventurous => "More variety from a wider circle of related artists, same general style.",
            Strength::DeepDiscovery => "Favors lesser-known tracks and artists you haven't heard.",
        }
    }
    /// Random variation applied when ranking candidates.
    pub fn jitter(self) -> f32 {
        match self {
            Strength::Familiar => 0.05,
            Strength::Balanced => 0.15,
            Strength::Adventurous => 0.25,
            Strength::DeepDiscovery => 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RadioProvider {
    #[default]
    Automatic,
    Spotify,
    Local,
    YouTube,
}

impl RadioProvider {
    pub fn label(self) -> &'static str {
        match self {
            RadioProvider::Automatic => "Automatic",
            RadioProvider::Spotify => "Spotify",
            RadioProvider::Local => "Local files",
            RadioProvider::YouTube => "YouTube Music (opens in browser)",
        }
    }
}

/// Similarity Radio preferences as the user sets them (no secrets: the
/// Last.fm API key stays in the device's secret store).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RadioPrefs {
    pub enabled: bool,
    /// Playing a song from search starts radio automatically.
    pub start_from_search: bool,
    pub start_from_artist: bool,
    pub start_from_album: bool,
    pub strength: Strength,
    /// Recommendations to keep queued (default 15).
    pub target: usize,
    /// Refill when this many remain (default 5).
    pub threshold: usize,
    pub provider: RadioProvider,
    pub endless: bool,
    pub prefer_unheard: bool,
    pub avoid_recent: bool,
    pub avoid_duplicates: bool,
    pub allow_repeat_artists: bool,
    pub learn: bool,
    pub stay_close: bool,
}

impl Default for RadioPrefs {
    fn default() -> Self {
        RadioPrefs {
            enabled: true,
            start_from_search: true,
            start_from_artist: false,
            start_from_album: false,
            strength: Strength::Balanced,
            target: 15,
            threshold: 5,
            provider: RadioProvider::Automatic,
            endless: true,
            prefer_unheard: true,
            avoid_recent: true,
            avoid_duplicates: true,
            allow_repeat_artists: false,
            learn: true,
            stay_close: false,
        }
    }
}
