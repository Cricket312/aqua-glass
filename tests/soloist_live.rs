//! Runs Aqua Glass's Soloist integration against the real Spotify Soloist binary.
//! Needs `SOLOIST_EXE` (path to a downloaded soloist / soloist.exe); skipped otherwise.
//! Uses a placeholder API key, so Soloist stays unpaired: this checks process
//! management, endpoint discovery, the WebSocket protocol and shutdown — not audio.

use aqua_glass::audio::soloist::{version, Command, Event, Soloist, SoloistConfig};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn wait(events: &Arc<Mutex<Vec<Event>>>, secs: u64, f: impl Fn(&Event) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if events.lock().unwrap().iter().any(&f) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn real_soloist_process_and_protocol() {
    let Ok(exe) = std::env::var("SOLOIST_EXE") else {
        eprintln!("SOLOIST_EXE not set — skipping");
        return;
    };
    let (text, built) = version(exe.as_ref()).expect("--version works");
    eprintln!("{text} (build time {built})");
    let dir = std::env::temp_dir().join(format!("aqua-soloist-{}", std::process::id()));
    let cfg = SoloistConfig { exe: exe.into(), api_key: "placeholder-key".into(), device_name: "Aqua Glass test".into(), data_dir: dir.clone(), manage: true };
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev2 = events.clone();
    let s = Soloist::start(&cfg, move |e| ev2.lock().unwrap().push(e)).expect("Soloist starts and opens its API");
    let pid = s.pid.expect("managed process");
    assert!(wait(&events, 10, |e| *e == Event::Connected), "WebSocket connected");
    assert!(wait(&events, 10, |e| matches!(e, Event::Auth { logged_in: false, device_name, .. } if device_name == "Aqua Glass test")), "auth_state: not paired, our device name: {:?}", events.lock().unwrap());
    // a control command before pairing is refused with a documented error
    s.send(Command::Play(Some("spotify:track:2JRo0gjbX4GrCqBYdRohoo".into())));
    assert!(wait(&events, 10, |e| matches!(e, Event::Error(m) if m.contains("auth"))), "play before pairing → error: {:?}", events.lock().unwrap());
    // the WebSocket only listens on loopback
    let addr = std::fs::read_to_string(dir.join("ws.addr")).unwrap();
    assert_eq!(addr.trim(), "127.0.0.1");
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        let rss = status.lines().find(|l| l.starts_with("VmRSS")).unwrap_or_default().to_string();
        eprintln!("Soloist (unpaired, idle) {rss}");
    }
    drop(s);
    std::thread::sleep(Duration::from_millis(500));
    #[cfg(unix)]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists() || std::fs::read_to_string(format!("/proc/{pid}/status")).map(|s| s.contains("zombie")).unwrap_or(true), "Soloist stopped with Aqua Glass");
    let _ = std::fs::remove_dir_all(dir);
}
