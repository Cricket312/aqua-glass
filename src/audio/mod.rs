//! Audio: one output pipeline shared by all in-app backends.

pub mod dsp;
pub mod engine;
#[cfg(feature = "local-files")]
pub mod local;
pub mod output;
pub mod soloist;
#[cfg(feature = "spotify-playback")]
pub mod spotify;
pub mod visualizer;
