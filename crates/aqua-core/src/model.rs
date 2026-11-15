//! Provider-neutral music metadata.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default, Hash, PartialOrd, Ord)]
pub enum ProviderKind {
    #[default]
    Spotify,
    /// Legacy: YouTube Music support was removed in 0.6. Kept so libraries,
    /// history and sync data written by older versions still load.
    YouTube,
    Local,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Spotify => "Spotify",
            ProviderKind::YouTube => "YouTube Music",
            ProviderKind::Local => "Local Files",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            ProviderKind::Spotify => "SP",
            ProviderKind::YouTube => "YT",
            ProviderKind::Local => "LO",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}



#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Track {
    pub provider: ProviderKind,
    /// Provider id: Spotify track id or absolute file path (YouTube video id in pre-0.6 data).
    pub id: String,
    /// Spotify URI (`spotify:track:...`) when applicable.
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_id: String,
    pub duration_ms: u64,
    pub explicit: bool,
    pub isrc: String,
    pub artwork_url: String,
    pub track_number: u32,
    /// Track is no longer playable/available upstream.
    pub unavailable: bool,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }
    pub fn key(&self) -> String {
        format!("{:?}:{}", self.provider, self.id)
    }
    /// A URL a user can open in a browser for this track, if any.
    pub fn web_url(&self) -> Option<String> {
        match self.provider {
            ProviderKind::Spotify => Some(format!("https://open.spotify.com/track/{}", self.id)),
            ProviderKind::YouTube | ProviderKind::Local => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Playlist {
    pub provider: ProviderKind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub owned_by_user: bool,
    pub collaborative: bool,
    pub public: Option<bool>,
    pub artwork_url: String,
    pub track_count: u32,
    /// Spotify snapshot_id — used to skip unchanged playlists on sync.
    pub snapshot: String,
    /// Special "Liked Songs"/"Liked Music" collection.
    pub is_liked: bool,
    /// Set when the provider refused to return items (e.g. API restrictions).
    pub items_error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Album {
    pub provider: ProviderKind,
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub artwork_url: String,
    pub release_date: String,
    pub total_tracks: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Artist {
    pub provider: ProviderKind,
    pub id: String,
    pub name: String,
    pub artwork_url: String,
    pub genres: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
    pub errors: Vec<String>,
}
