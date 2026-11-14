//! Local audio files (MP3, FLAC, Ogg Vorbis, WAV, AAC/M4A) decoded with symphonia.

use super::output::{Feeder, Output};
use crate::config::ProviderKind;
use crate::model::Track;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Value};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub const EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "wav", "m4a", "aac", "mp4"];

pub struct Decoding {
    pub stop: Arc<AtomicBool>,
}

impl Drop for Decoding {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn open(path: &Path) -> anyhow::Result<symphonia::core::probe::ProbeResult> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    Ok(symphonia::default::get_probe().format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())?)
}

fn parse_gain(s: &str) -> Option<f32> {
    s.trim().trim_end_matches("dB").trim_end_matches("db").trim().parse::<f32>().ok()
}

/// Start decoding `path` from `start_ms`. `on_end(ok, error)` is called when the
/// file has been fully pushed (or failed). Decoding stops when `Decoding` is dropped
/// or the output is flushed.
pub fn spawn(
    path: PathBuf,
    start_ms: u64,
    out: Output,
    gen: u64,
    replaygain: bool,
    on_end: impl FnOnce(Result<(), String>) + Send + 'static,
) -> Decoding {
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    std::thread::Builder::new()
        .name("local-decoder".into())
        .spawn(move || {
            let r = decode(&path, start_ms, &out, gen, replaygain, &st);
            if !st.load(Ordering::Relaxed) && out.generation() == gen {
                on_end(r.map_err(|e| e.to_string()));
            }
        })
        .expect("spawn decoder");
    Decoding { stop }
}

fn decode(path: &Path, start_ms: u64, out: &Output, gen: u64, replaygain: bool, stop: &AtomicBool) -> anyhow::Result<()> {
    let mut probed = open(path)?;
    let mut gain_db: Option<f32> = None;
    if replaygain {
        let mut scan = |rev: &symphonia::core::meta::MetadataRevision| {
            for t in rev.tags() {
                if matches!(t.std_key, Some(StandardTagKey::ReplayGainTrackGain)) || t.key.eq_ignore_ascii_case("REPLAYGAIN_TRACK_GAIN") {
                    if let Value::String(s) = &t.value {
                        gain_db = parse_gain(s);
                    }
                }
            }
        };
        if let Some(m) = probed.metadata.get() {
            if let Some(rev) = m.current() {
                scan(rev);
            }
        }
        if let Some(rev) = probed.format.metadata().current() {
            scan(rev);
        }
    }
    let format = &mut probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("no audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let rate = track.codec_params.sample_rate.unwrap_or(44100);
    if start_ms > 0 {
        let _ = format.seek(SeekMode::Coarse, SeekTo::Time { time: Time::from(start_ms as f64 / 1000.0), track_id: Some(track.id) });
    }
    let mut feeder = Feeder::new(out, rate, gen);
    if let Some(g) = gain_db {
        feeder.set_track_gain(10f32.powf(g.clamp(-20.0, 12.0) / 20.0));
    }
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(symphonia::core::errors::Error::ResetRequired) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue, // skip corrupt frame
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        if !feeder.write(b.samples(), spec.channels.count()) {
            return Ok(()); // flushed
        }
    }
}

/// Read tags and duration for the library.
pub fn probe_track(path: &Path) -> Option<Track> {
    let mut probed = open(path).ok()?;
    let mut t = Track {
        provider: ProviderKind::Local,
        id: path.to_string_lossy().to_string(),
        title: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        ..Default::default()
    };
    let mut cover: Option<Vec<u8>> = None;
    let mut apply = |rev: &symphonia::core::meta::MetadataRevision| {
        if cover.is_none() {
            if let Some(v) = rev.visuals().first() {
                if v.data.len() < 8 * 1024 * 1024 {
                    cover = Some(v.data.to_vec());
                }
            }
        }
        for tag in rev.tags() {
            let v = tag.value.to_string();
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => t.title = v,
                Some(StandardTagKey::Artist) => t.artists = v.split(['/', ';']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                Some(StandardTagKey::Album) => t.album = v,
                Some(StandardTagKey::TrackNumber) => t.track_number = v.split('/').next().and_then(|n| n.trim().parse().ok()).unwrap_or(0),
                Some(StandardTagKey::IdentIsrc) => t.isrc = v,
                _ => {}
            }
        }
    };
    if let Some(m) = probed.metadata.get() {
        if let Some(rev) = m.current() {
            apply(rev);
        }
    }
    if let Some(rev) = probed.format.metadata().current() {
        apply(rev);
    }
    if let Some(tr) = probed.format.default_track() {
        let p = &tr.codec_params;
        if let (Some(n), Some(r)) = (p.n_frames, p.sample_rate) {
            t.duration_ms = n * 1000 / r as u64;
        }
    }
    t.album_id = format!("local-album:{}", t.album);
    if let Some(bytes) = cover {
        // embedded cover art › one cached file per album (the artwork loader downsizes it)
        use sha2::Digest;
        let key = if t.album.is_empty() { t.id.clone() } else { format!("{}|{}", t.artists.first().cloned().unwrap_or_default(), t.album) };
        let h = sha2::Sha256::digest(key.as_bytes());
        let name: String = h.iter().take(12).map(|b| format!("{b:02x}")).collect();
        let path = crate::util::cache_dir().join("local-art").join(format!("{name}.img"));
        if !path.exists() {
            let _ = crate::util::atomic_write(&path, &bytes);
        }
        t.artwork_url = path.to_string_lossy().to_string();
    }
    Some(t)
}

/// Recursively scan folders for audio files (bounded depth, skips hidden dirs).
pub fn scan(folders: &[String], progress: &dyn Fn(usize)) -> Vec<Track> {
    let mut files = Vec::new();
    fn walk(dir: &Path, depth: u32, files: &mut Vec<PathBuf>) {
        if depth > 12 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                walk(&p, depth + 1, files);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| EXTENSIONS.contains(&x.to_ascii_lowercase().as_str())) {
                files.push(p);
            }
        }
    }
    for f in folders {
        walk(Path::new(f), 0, &mut files);
    }
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        if let Some(t) = probe_track(f) {
            out.push(t);
        }
        if i % 25 == 0 {
            progress(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaygain_parse() {
        assert_eq!(parse_gain("-6.20 dB"), Some(-6.2));
        assert_eq!(parse_gain("+1.5dB"), Some(1.5));
        assert_eq!(parse_gain("x"), None);
    }

    pub fn write_test_wav(path: &Path, secs: f32, freq: f32) {
        let rate = 22050u32;
        let n = (rate as f32 * secs) as u32;
        let mut d = Vec::new();
        let data_len = n * 2;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + data_len).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&(rate * 2).to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..n {
            let s = ((2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 12000.0) as i16;
            d.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, d).unwrap();
    }

    #[test]
    fn probes_wav() {
        let dir = std::env::temp_dir().join(format!("aqua-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("My Song.wav");
        write_test_wav(&p, 2.0, 440.0);
        let t = probe_track(&p).unwrap();
        assert_eq!(t.title, "My Song");
        assert!((t.duration_ms as i64 - 2000).abs() < 20, "{}", t.duration_ms);
        let found = scan(&[dir.to_string_lossy().to_string()], &|_| {});
        assert_eq!(found.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
