//! End-to-end playback test: real decoder → DSP → cpal output → engine queue.
//! Needs an audio output device; on CI/Linux use an ALSA null/file device
//! (see README "Testing"). Skips (passes) when no device exists.

use aqua_glass::audio::engine::{wait_until, Cmd, Engine, EngineOpts};
use aqua_glass::audio::output::Output;
use aqua_glass::config::ProviderKind;
use aqua_glass::model::Track;
use std::path::Path;
use std::time::Duration;

fn wav(path: &Path, secs: f32, freq: f32) {
    let rate = 44100u32;
    let n = (rate as f32 * secs) as u32;
    let mut d = Vec::new();
    let data_len = n * 4;
    d.extend_from_slice(b"RIFF");
    d.extend_from_slice(&(36 + data_len).to_le_bytes());
    d.extend_from_slice(b"WAVEfmt ");
    d.extend_from_slice(&16u32.to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&2u16.to_le_bytes());
    d.extend_from_slice(&rate.to_le_bytes());
    d.extend_from_slice(&(rate * 4).to_le_bytes());
    d.extend_from_slice(&4u16.to_le_bytes());
    d.extend_from_slice(&16u16.to_le_bytes());
    d.extend_from_slice(b"data");
    d.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        let s = ((2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 8000.0) as i16;
        d.extend_from_slice(&s.to_le_bytes());
        d.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, d).unwrap();
}

fn local(path: &Path, ms: u64) -> Track {
    Track { provider: ProviderKind::Local, id: path.to_string_lossy().into(), title: path.file_stem().unwrap().to_string_lossy().into(), duration_ms: ms, ..Default::default() }
}

#[test]
fn local_playback_queue_seek_pause() {
    let out = Output::start("", 300);
    if out.s.device_error.lock().unwrap().is_some() {
        eprintln!("no audio device — skipping: {:?}", out.s.device_error.lock().unwrap());
        return;
    }
    let dir = std::env::temp_dir().join(format!("aqua-play-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    wav(&a, 1.5, 440.0);
    wav(&b, 3.0, 880.0);
    let engine = Engine::start(out.clone(), EngineOpts { gapless: true, ..Default::default() }, 0.5, false, Box::new(|| {}));
    engine.send(Cmd::Play { tracks: vec![local(&a, 1500), local(&b, 3000)], start: 0 });

    // playing, position advancing
    assert!(wait_until(Duration::from_secs(5), || engine.snapshot().playing), "should start playing");
    assert!(wait_until(Duration::from_secs(5), || engine.position_ms() > 300), "position should advance");
    let consumed0 = out.s.consumed.load(std::sync::atomic::Ordering::Relaxed);
    assert!(consumed0 > 0, "device consumed samples");

    // automatic advance to the second track (gapless boundary)
    assert!(
        wait_until(Duration::from_secs(10), || engine.snapshot().current.map(|t| t.title == "b").unwrap_or(false)),
        "should auto-advance to b"
    );
    assert!(engine.position_ms() < 2000);

    // seek
    engine.send(Cmd::Seek(2000));
    assert!(wait_until(Duration::from_secs(3), || engine.position_ms() >= 2000), "seek applies, got {}", engine.position_ms());

    // pause stops consumption
    engine.send(Cmd::Pause);
    assert!(wait_until(Duration::from_secs(2), || !engine.snapshot().playing));
    std::thread::sleep(Duration::from_millis(300));
    let p1 = engine.position_ms();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(p1, engine.position_ms(), "position frozen while paused");

    // resume then end of queue
    engine.send(Cmd::Resume);
    assert!(wait_until(Duration::from_secs(6), || !engine.snapshot().playing && engine.snapshot().message.as_deref() == Some("End of queue")), "queue should end");

    // a missing file reports an error instead of crashing, and the engine moves on
    engine.send(Cmd::Play { tracks: vec![local(&dir.join("missing.wav"), 1000), local(&a, 1500)], start: 0 });
    assert!(wait_until(Duration::from_secs(5), || engine.snapshot().error.is_some() || engine.snapshot().current.map(|t| t.title == "a").unwrap_or(false)));
    engine.send(Cmd::Shutdown);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn engine_handles_spotify_without_session() {
    let out = Output::start("", 300);
    let engine = Engine::start(out, EngineOpts::default(), 0.5, false, Box::new(|| {}));
    let t = Track { provider: ProviderKind::Spotify, id: "4uLU6hMCjMI75M1A2tKUQC".into(), uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(), title: "x".into(), ..Default::default() };
    engine.send(Cmd::Play { tracks: vec![t], start: 0 });
    assert!(wait_until(Duration::from_secs(3), || engine.snapshot().error.is_some()), "a clear error, not a crash");
    let e = engine.snapshot().error.unwrap();
    assert!(e.contains("Spotify"), "{e}");
    engine.send(Cmd::Shutdown);
}

/// Regression (0.6.1): connecting Spotify playback crashed the engine thread with
/// "there is no reactor running" because librespot's Session was created outside its
/// Tokio runtime. With a dummy token (and no route to Spotify in CI) the connection
/// must now *fail cleanly* and the engine must keep working.
#[test]
fn spotify_connect_fails_cleanly_instead_of_crashing() {
    aqua_glass::services::install_crypto();
    let out = Output::start("", 300);
    let engine = Engine::start(out, EngineOpts::default(), 0.5, false, Box::new(|| {}));
    engine.send(Cmd::SpotifyConnect(std::sync::Arc::new(|| Ok("dummy-token".to_string()))));
    let done = wait_until(Duration::from_secs(90), || matches!(engine.snapshot().spotify, aqua_glass::audio::engine::SpotifyStatus::Unavailable(_) | aqua_glass::audio::engine::SpotifyStatus::Ready));
    assert!(done, "connect attempt finished (engine thread alive), status: {:?}", engine.snapshot().spotify);
    // the engine still answers commands afterwards
    engine.send(Cmd::SetVolume(0.3));
    assert!(wait_until(Duration::from_secs(3), || (engine.snapshot().volume - 0.3).abs() < 0.01), "engine still responsive");
    engine.send(Cmd::Shutdown);
}

#[test]
fn plays_every_track_in_order() {
    sequence(true);
    sequence(false);
}

fn sequence(gapless: bool) {
    let out = Output::start("", 500);
    if out.s.device_error.lock().unwrap().is_some() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("aqua-seq-{}-{gapless}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut tracks = Vec::new();
    for (i, secs) in [1.2f32, 0.3, 1.0, 1.1].iter().enumerate() {
        let p = dir.join(format!("t{i}.wav"));
        wav(&p, *secs, 300.0 + i as f32 * 100.0);
        tracks.push(local(&p, (*secs * 1000.0) as u64));
    }
    let engine = Engine::start(out, EngineOpts { gapless, ..Default::default() }, 0.5, false, Box::new(|| {}));
    engine.send(Cmd::Play { tracks, start: 0 });
    let mut seen: Vec<String> = Vec::new();
    let end = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < end {
        let s = engine.snapshot();
        if let Some(t) = s.current {
            if seen.last() != Some(&t.title) {
                seen.push(t.title.clone());
            }
        }
        if s.message.as_deref() == Some("End of queue") {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(seen, vec!["t0", "t1", "t2", "t3"], "every track (incl. one shorter than the buffer) plays once, in order (gapless={gapless})");
    engine.send(Cmd::Shutdown);
    let _ = std::fs::remove_dir_all(dir);
}
