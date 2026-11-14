//! Aqua Glass portable core.
//!
//! Everything in this crate is platform-independent Rust (no UI toolkit, no
//! audio backend, no OS APIs) so it can be shared by the Windows desktop app
//! and a future Android client (via UniFFI/JNI), and by an optional
//! self-hosted sync server.
//!
//! * [`model`] — tracks, playlists, albums, artists, provider ids.
//! * [`matching`] — cross-provider track matching and title normalisation.
//! * [`history`] — bounded local listening history.
//! * [`library`] — imported playlists / saved collections on disk.
//! * [`queue`] — play queue logic (manual vs. radio entries, shuffle, repeat).
//! * [`playback`] — the [`playback::PlaybackControl`] interface and player state.
//! * [`prefs`] — portable Similarity Radio preferences.
//! * [`sync`] — versioned, serialisable cross-device sync and handoff models
//!   (data only: no networking is implemented).
//! * [`secrets`], [`storage`], [`platform`] — hooks the platform layer fills in.

pub mod history;
pub mod library;
pub mod matching;
pub mod model;
pub mod platform;
pub mod playback;
pub mod prefs;
pub mod queue;
pub mod secrets;
pub mod session;
pub mod storage;
pub mod sync;
