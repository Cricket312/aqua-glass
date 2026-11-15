//! Playback engine + Spotify Soloist backend against a fake Soloist that speaks the
//! documented WebSocket protocol (auth_state, track_changed, playback_changed,
//! position_sync, error; commands play/pause/seek/set_volume/activate/get_*).

use aqua_glass::audio::engine::{wait_until, Cmd, Engine, EngineOpts, SpotifyStatus};
use aqua_glass::audio::output::Output;
use aqua_glass::audio::soloist::SoloistConfig;
use aqua_glass::config::{ProviderKind, SpotifyBackend};
use aqua_glass::model::Track;
use serde_json::{json, Value};
use std::net::TcpListener;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

struct Fake {
    dir: std::path::PathBuf,
    got: Arc<Mutex<Vec<Value>>>,
    push: mpsc::Sender<Value>,
}

/// One-connection fake Soloist. `logged_in` = paired. Every `play` with a URI is
/// answered like Soloist does: track_changed, playback_changed(playing), position_sync.
fn fake(logged_in: bool) -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!("aqua-fake-soloist-{}-{port}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ws.addr"), "127.0.0.1").unwrap();
    std::fs::write(dir.join("ws.port"), port.to_string()).unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let (push, pushed) = mpsc::channel::<Value>();
    let g = got.clone();
    std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut ws = tungstenite::accept(s).unwrap();
        ws.get_ref().set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: Value| {
            let _ = ws.send(tungstenite::Message::text(v.to_string()));
        };
        send(&mut ws, json!({"type":"auth_state","logged_in":logged_in,"is_active":logged_in,"device_name":"Aqua Glass"}));
        loop {
            while let Ok(v) = pushed.try_recv() {
                send(&mut ws, v);
            }
            match ws.read() {
                Ok(tungstenite::Message::Text(t)) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    assert_eq!(v["type"], "command", "every client message is a command");
                    g.lock().unwrap().push(v.clone());
                    match v["command"].as_str().unwrap() {
                        "get_auth_state" => send(&mut ws, json!({"type":"auth_state","logged_in":logged_in,"is_active":logged_in,"device_name":"Aqua Glass"})),
                        "play" if !logged_in => send(&mut ws, json!({"type":"error","message":"command requires authentication"})),
                        "play" => {
                            if let Some(uri) = v["uri"].as_str() {
                                send(&mut ws, json!({"type":"track_changed","item":{"uri":uri,"entity_type":"track","decorations":{"playback":{"duration_ms":180000}}}}));
                            }
                            send(&mut ws, json!({"type":"playback_changed","status":"playing"}));
                            send(&mut ws, json!({"type":"position_sync","position":{"position_ms":0,"timestamp_ms":now(),"speed":1.0}}));
                        }
                        "pause" => send(&mut ws, json!({"type":"playback_changed","status":"paused"})),
                        _ => {}
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return,
            }
        }
    });
    Fake { dir, got, push }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn sp(id: &str) -> Track {
    Track { provider: ProviderKind::Spotify, id: id.into(), uri: format!("spotify:track:{id}"), title: id.into(), duration_ms: 180_000, ..Default::default() }
}

fn plays(f: &Fake) -> Vec<String> {
    f.got.lock().unwrap().iter().filter(|v| v["command"] == "play").filter_map(|v| v["uri"].as_str().map(|s| s.to_string())).collect()
}

fn engine(f: &Fake) -> Engine {
    let out = Output::start("", 300);
    let e = Engine::start(out, EngineOpts { spotify_backend: SpotifyBackend::Soloist, ..Default::default() }, 0.5, false, Box::new(|| {}));
    e.send(Cmd::SoloistStart(SoloistConfig { exe: "unused".into(), api_key: String::new(), device_name: "Aqua Glass".into(), data_dir: f.dir.clone(), manage: false }));
    e
}

#[test]
fn soloist_backend_plays_our_queue_in_order() {
    let f = fake(true);
    let e = engine(&f);
    assert!(wait_until(Duration::from_secs(10), || e.snapshot().spotify == SpotifyStatus::Ready), "paired Soloist → Ready: {:?}", e.snapshot().spotify);

    e.send(Cmd::Play { tracks: vec![sp("A"), sp("B")], start: 0 });
    assert!(wait_until(Duration::from_secs(5), || plays(&f) == ["spotify:track:A"]), "{:?}", plays(&f));
    assert!(wait_until(Duration::from_secs(5), || e.snapshot().playing), "playing after Soloist confirms");
    // radio recommendation + manual pick: the manual one must play first
    e.send(Cmd::RadioAppend(vec![sp("R1")]));
    e.send(Cmd::Enqueue(vec![sp("M")]));
    // Soloist reaches the end of A and autoplay moves on to something else
    f.push.send(json!({"type":"track_changed","item":{"uri":"spotify:track:AUTOPLAY","entity_type":"track","decorations":{}}})).unwrap();
    assert!(wait_until(Duration::from_secs(5), || plays(&f).len() == 2), "{:?}", plays(&f));
    assert_eq!(plays(&f)[1], "spotify:track:B", "manual picks queue behind the rest of what you started, ahead of radio");
    // end of B → M (manual) before R1 (radio)
    f.push.send(json!({"type":"playback_changed","status":"idle"})).unwrap();
    assert!(wait_until(Duration::from_secs(5), || plays(&f).len() == 3), "{:?}", plays(&f));
    assert_eq!(plays(&f)[2], "spotify:track:M", "manual pick before radio");
    f.push.send(json!({"type":"playback_changed","status":"idle"})).unwrap();
    assert!(wait_until(Duration::from_secs(5), || plays(&f).len() == 4), "{:?}", plays(&f));
    assert_eq!(plays(&f)[3], "spotify:track:R1");
    assert_eq!(e.snapshot().current.unwrap().id, "R1");

    // controls go to Soloist
    e.send(Cmd::Pause);
    e.send(Cmd::Seek(42_000));
    e.send(Cmd::SetVolume(0.25));
    assert!(
        wait_until(Duration::from_secs(5), || {
            let g = f.got.lock().unwrap();
            g.iter().any(|v| v["command"] == "pause") && g.iter().any(|v| v["command"] == "seek" && v["position_ms"] == 42_000) && g.iter().any(|v| v["command"] == "set_volume" && v["volume"] == 25)
        }),
        "{:?}",
        f.got.lock().unwrap()
    );
    assert!(wait_until(Duration::from_secs(3), || !e.snapshot().playing));
    // position follows Soloist's anchor
    f.push.send(json!({"type":"position_sync","position":{"position_ms":90000,"timestamp_ms":now(),"speed":0.0}})).unwrap();
    assert!(wait_until(Duration::from_secs(3), || (89_000..=91_000).contains(&e.position_ms())), "{}", e.position_ms());
    // a mid-song pause from the Spotify app is a pause, not "track ended"
    let n = plays(&f).len();
    let bare = || f.got.lock().unwrap().iter().filter(|v| v["command"] == "play" && v.get("uri").is_none()).count();
    e.send(Cmd::Resume);
    assert!(wait_until(Duration::from_secs(3), || bare() == 1), "resume = a bare play");
    f.push.send(json!({"type":"position_sync","position":{"position_ms":95000,"timestamp_ms":now(),"speed":1.0}})).unwrap();
    f.push.send(json!({"type":"playback_changed","status":"paused"})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(plays(&f).len(), n, "a mid-song pause doesn't start another song");
    assert_eq!(e.snapshot().current.unwrap().id, "R1");
    e.send(Cmd::Shutdown);
}

#[test]
fn unpaired_soloist_explains_and_does_not_skip() {
    let f = fake(false);
    let e = engine(&f);
    assert!(wait_until(Duration::from_secs(10), || matches!(e.snapshot().spotify, SpotifyStatus::Unavailable(ref m) if m.contains("paired"))), "{:?}", e.snapshot().spotify);
    e.send(Cmd::Play { tracks: vec![sp("A"), sp("B")], start: 0 });
    assert!(wait_until(Duration::from_secs(5), || e.snapshot().error.is_some_and(|m| m.contains("devices icon"))), "{:?}", e.snapshot().error);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(e.snapshot().current.unwrap().id, "A", "no skipping through the queue");
    assert!(plays(&f).is_empty(), "nothing sent while unpaired");
    e.send(Cmd::Shutdown);
}
