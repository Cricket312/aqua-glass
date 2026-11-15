//! Spotify playback through **Spotify Soloist**, Spotify's official headless
//! client (<https://developer.spotify.com/documentation/soloist>).
//!
//! Soloist runs as a separate process and plays the audio itself (Windows:
//! WASAPI). Aqua Glass keeps its own queue and Similarity Radio and tells
//! Soloist which track to play through Soloist's local WebSocket API.
//!
//! * Soloist is **not bundled**: the user downloads it from Spotify and points
//!   Aqua Glass at `soloist.exe`. Builds expire 90 days after their build date
//!   (exit code 10); [`version`] reads the build date so the UI can warn early.
//! * The API key is read from the secret store and passed as a command-line
//!   argument — the only way Soloist accepts it — so local users with access to
//!   the process list can see it while Soloist runs (a Soloist limitation).
//! * The WebSocket is always bound to `127.0.0.1` (`--ws 127.0.0.1:0`) and Aqua
//!   Glass refuses to connect to a non-loopback endpoint. It has no
//!   authentication of its own, so it must never be exposed.
//! * With "manage" on, Aqua Glass starts Soloist with a hidden window and ties
//!   it to a Windows job object, so Soloist also stops if Aqua Glass crashes.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Spotify Soloist builds stop working this many days after their build date.
pub const BUILD_LIFETIME_DAYS: i64 = 90;

#[derive(Clone, Debug)]
pub struct SoloistConfig {
    pub exe: PathBuf,
    pub api_key: String,
    pub device_name: String,
    pub data_dir: PathBuf,
    /// Start/stop Soloist ourselves (true) or attach to one already running (false).
    pub manage: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Playing,
    Paused,
    Buffering,
}

impl Status {
    fn parse(s: &str) -> Status {
        match s {
            "playing" => Status::Playing,
            "paused" => Status::Paused,
            "buffering" => Status::Buffering,
            _ => Status::Idle,
        }
    }
}

/// Position at a wall-clock instant; `speed` 0 = not advancing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PosAnchor {
    pub position_ms: u64,
    pub timestamp_ms: i64,
    pub speed: f64,
}

impl PosAnchor {
    pub fn estimate(&self, now_ms: i64) -> u64 {
        let elapsed = (now_ms - self.timestamp_ms).max(0) as f64;
        (self.position_ms as f64 + elapsed * self.speed).max(0.0) as u64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// WebSocket connected.
    Connected,
    Auth { logged_in: bool, active: bool, device_name: String },
    State { status: Status, uri: String, duration_ms: u64, position: PosAnchor, volume: Option<u8> },
    Track { uri: String, duration_ms: u64 },
    Playback(Status),
    Position(PosAnchor),
    Active(bool),
    /// Soloist rejected a command (e.g. "command requires authentication").
    Error(String),
    /// Connection lost or the process exited.
    Down(String),
}

fn entity(v: &serde_json::Value) -> (String, u64) {
    (v["uri"].as_str().unwrap_or_default().to_string(), v["decorations"]["playback"]["duration_ms"].as_u64().unwrap_or(0))
}

fn anchor(v: &serde_json::Value) -> PosAnchor {
    PosAnchor { position_ms: v["position_ms"].as_u64().unwrap_or(0), timestamp_ms: v["timestamp_ms"].as_i64().unwrap_or(0), speed: v["speed"].as_f64().unwrap_or(0.0) }
}

/// Parse one server → client frame (documented event types only; others are ignored).
pub fn parse_event(text: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(match v["type"].as_str()? {
        "auth_state" => Event::Auth { logged_in: v["logged_in"].as_bool().unwrap_or(false), active: v["is_active"].as_bool().unwrap_or(false), device_name: v["device_name"].as_str().unwrap_or_default().into() },
        "playback_state" => {
            let (uri, duration_ms) = entity(&v["item"]);
            Event::State { status: Status::parse(v["status"].as_str().unwrap_or("")), uri, duration_ms, position: anchor(&v["position"]), volume: v["volume"].as_u64().map(|x| x.min(100) as u8) }
        }
        "track_changed" => {
            let (uri, duration_ms) = entity(&v["item"]);
            Event::Track { uri, duration_ms }
        }
        "playback_changed" => Event::Playback(Status::parse(v["status"].as_str().unwrap_or(""))),
        "position_sync" => Event::Position(anchor(&v["position"])),
        "device_changed" => Event::Active(v["is_active"].as_bool().unwrap_or(false)),
        "error" => Event::Error(v["message"].as_str().unwrap_or("error").to_string()),
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Start playing a Spotify URI (or resume when None).
    Play(Option<String>),
    Pause,
    Seek(u64),
    SetVolume(u8),
    Activate,
    GetState,
    GetAuthState,
}

pub fn command_json(c: &Command) -> String {
    let v = match c {
        Command::Play(Some(uri)) => serde_json::json!({"type": "command", "command": "play", "uri": uri}),
        Command::Play(None) => serde_json::json!({"type": "command", "command": "play"}),
        Command::Pause => serde_json::json!({"type": "command", "command": "pause"}),
        Command::Seek(ms) => serde_json::json!({"type": "command", "command": "seek", "position_ms": ms}),
        Command::SetVolume(v) => serde_json::json!({"type": "command", "command": "set_volume", "volume": (*v).min(100)}),
        Command::Activate => serde_json::json!({"type": "command", "command": "activate"}),
        Command::GetState => serde_json::json!({"type": "command", "command": "get_state"}),
        Command::GetAuthState => serde_json::json!({"type": "command", "command": "get_auth_state"}),
    };
    v.to_string()
}

/// `soloist --version` → (version text, build date as unix seconds).
/// Output looks like `soloist 1.3.10.71 build 1794592912 (20261113) (g19412df6ea) (windows/x86_64)`.
pub fn version(exe: &Path) -> Result<(String, i64), String> {
    let mut cmd = Proc::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stderr(Stdio::null());
    hide_window(&mut cmd);
    let out = cmd.output().map_err(|e| format!("could not run {}: {e}", exe.display()))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.code() == Some(10) {
        return Err("this Soloist build has expired — download a new one".into());
    }
    let built = parse_build_time(&text).ok_or_else(|| format!("unexpected --version output: {text}"))?;
    Ok((text, built))
}

/// Build time from the `build <unix seconds>` field (falls back to the `(YYYYMMDD)` date).
pub fn parse_build_time(version_text: &str) -> Option<i64> {
    let mut words = version_text.split_whitespace();
    while let Some(w) = words.next() {
        if w == "build" {
            if let Some(ts) = words.next().and_then(|t| t.parse::<i64>().ok()) {
                return Some(ts);
            }
        }
    }
    let d = version_text.split(['(', ')']).find(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))?;
    let (y, m, day) = (d[0..4].parse().ok()?, d[4..6].parse::<u8>().ok()?, d[6..8].parse().ok()?);
    let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, day).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp())
}

/// Days until a build made at `built` expires (negative = expired).
pub fn days_left(built: i64, now: i64) -> i64 {
    (built + BUILD_LIFETIME_DAYS * 86_400 - now).div_euclid(86_400)
}

fn hide_window(cmd: &mut Proc) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Kills the child when Aqua Glass exits — even if it crashes (Windows job object).
#[cfg(windows)]
struct KillOnClose(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl KillOnClose {
    fn attach(child: &Child) -> Option<KillOnClose> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).ok()?;
            AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).ok()?;
            Some(KillOnClose(job))
        }
    }
}

#[cfg(windows)]
impl Drop for KillOnClose {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
unsafe impl Send for KillOnClose {}

pub struct Soloist {
    tx: mpsc::Sender<Command>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    pub pid: Option<u32>,
    #[cfg(windows)]
    _job: Option<KillOnClose>,
}

fn is_loopback(addr: &str) -> bool {
    addr == "127.0.0.1" || addr == "::1" || addr == "localhost"
}

/// Read the endpoint Soloist wrote into its data directory.
pub fn endpoint(data_dir: &Path) -> Option<(String, u16)> {
    let addr = std::fs::read_to_string(data_dir.join("ws.addr")).ok()?.trim().to_string();
    let port = std::fs::read_to_string(data_dir.join("ws.port")).ok()?.trim().parse().ok()?;
    Some((addr, port))
}

impl Soloist {
    /// Start (or attach to) Soloist and connect to its WebSocket. `on_event` runs on
    /// the client thread.
    pub fn start(cfg: &SoloistConfig, on_event: impl Fn(Event) + Send + 'static) -> Result<Soloist, String> {
        let _ = std::fs::create_dir_all(&cfg.data_dir);
        let mut child = None;
        #[cfg(windows)]
        let mut job = None;
        if cfg.manage {
            if !cfg.exe.is_file() {
                return Err(format!("soloist.exe not found at {}", cfg.exe.display()));
            }
            if cfg.api_key.trim().is_empty() {
                return Err("no Soloist API key stored — add it in Settings › Music Services".into());
            }
            for f in ["ws.addr", "ws.port"] {
                let _ = std::fs::remove_file(cfg.data_dir.join(f));
            }
            let log = std::fs::File::create(cfg.data_dir.join("soloist-output.log")).ok();
            let mut cmd = Proc::new(&cfg.exe);
            cmd.arg("--device-name")
                .arg(&cfg.device_name)
                .arg("--api-key")
                .arg(cfg.api_key.trim())
                .arg("--ws")
                .arg("127.0.0.1:0")
                .arg("--data-dir")
                .arg(&cfg.data_dir)
                .arg("--cache-dir")
                .arg(cfg.data_dir.join("cache"))
                .arg("--cache-size")
                .arg("500")
                .stdin(Stdio::null());
            match log.and_then(|f| f.try_clone().ok().map(|g| (f, g))) {
                Some((a, b)) => {
                    cmd.stdout(a).stderr(b);
                }
                None => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
            }
            hide_window(&mut cmd);
            let c = cmd.spawn().map_err(|e| format!("could not start Soloist: {e}"))?;
            #[cfg(windows)]
            {
                job = KillOnClose::attach(&c);
            }
            child = Some(c);
        }
        // wait for the endpoint files (Soloist writes them once the API listens)
        let deadline = Instant::now() + Duration::from_secs(20);
        let (addr, port) = loop {
            if let Some(c) = child.as_mut() {
                if let Ok(Some(st)) = c.try_wait() {
                    return Err(match st.code() {
                        Some(10) => "this Soloist build has expired (Soloist builds stop working 90 days after release) — download the current build from Spotify".into(),
                        Some(code) => format!("Soloist exited with code {code} — see soloist-output.log in {}", cfg.data_dir.display()),
                        None => "Soloist exited".into(),
                    });
                }
            }
            if let Some(e) = endpoint(&cfg.data_dir) {
                break e;
            }
            if Instant::now() > deadline {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                }
                return Err(if cfg.manage { "Soloist did not open its local API within 20 s".into() } else { format!("no running Soloist found (no ws.port in {})", cfg.data_dir.display()) });
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if !is_loopback(&addr) {
            if let Some(mut c) = child.take() {
                let _ = c.kill();
            }
            return Err(format!("refusing to connect: Soloist's WebSocket is bound to {addr}, not localhost — restart it with --ws 127.0.0.1:0"));
        }
        let pid = child.as_ref().map(|c| c.id());
        let child = Arc::new(Mutex::new(child));
        let (tx, rx) = mpsc::channel::<Command>();
        let stop = Arc::new(AtomicBool::new(false));
        let (st, ch, data_dir) = (stop.clone(), child.clone(), cfg.data_dir.clone());
        std::thread::Builder::new()
            .name("soloist-ws".into())
            .spawn(move || client_loop(addr, port, data_dir, rx, st, ch, on_event))
            .map_err(|e| e.to_string())?;
        Ok(Soloist {
            tx,
            stop,
            child,
            pid,
            #[cfg(windows)]
            _job: job,
        })
    }

    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}

impl Drop for Soloist {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn client_loop(mut addr: String, mut port: u16, data_dir: PathBuf, rx: mpsc::Receiver<Command>, stop: Arc<AtomicBool>, child: Arc<Mutex<Option<Child>>>, on_event: impl Fn(Event)) {
    let mut announced_down = false;
    while !stop.load(Ordering::Relaxed) {
        // the process may have exited
        if let Some(c) = child.lock().unwrap().as_mut() {
            if let Ok(Some(st)) = c.try_wait() {
                on_event(Event::Down(if st.code() == Some(10) { "Soloist build expired — download a new one".into() } else { format!("Soloist stopped (exit code {:?})", st.code()) }));
                return;
            }
        }
        let url = format!("ws://{addr}:{port}");
        let ws = TcpStream::connect((addr.as_str(), port)).map_err(|e| e.to_string()).and_then(|s| {
            s.set_nodelay(true).ok();
            tungstenite::client(url.as_str(), s).map(|(ws, _)| ws).map_err(|e| e.to_string())
        });
        let mut ws = match ws {
            Ok(ws) => ws,
            Err(e) => {
                if !announced_down {
                    on_event(Event::Down(format!("can't reach Soloist: {e}")));
                    announced_down = true;
                }
                std::thread::sleep(Duration::from_secs(2));
                // an external Soloist may have restarted on a new port
                if let Some((a, p)) = endpoint(&data_dir).filter(|(a, _)| is_loopback(a)) {
                    addr = a;
                    port = p;
                }
                continue;
            }
        };
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(100)));
        announced_down = false;
        on_event(Event::Connected);
        // drop commands queued while disconnected except the latest state queries
        while rx.try_recv().is_ok() {}
        let _ = ws.send(tungstenite::Message::text(command_json(&Command::GetAuthState)));
        let _ = ws.send(tungstenite::Message::text(command_json(&Command::GetState)));
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = ws.close(None);
                return;
            }
            while let Ok(c) = rx.try_recv() {
                if ws.send(tungstenite::Message::text(command_json(&c))).is_err() {
                    break;
                }
            }
            match ws.read() {
                Ok(tungstenite::Message::Text(t)) => {
                    if let Some(ev) = parse_event(t.as_str()) {
                        on_event(ev);
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => {
                    on_event(Event::Down(format!("connection to Soloist lost: {e}")));
                    announced_down = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Payloads copied from the Soloist WebSocket API reference.
    #[test]
    fn parses_documented_events() {
        assert_eq!(parse_event(r#"{"type":"auth_state","logged_in":true,"is_active":true,"device_name":"Kitchen speaker"}"#), Some(Event::Auth { logged_in: true, active: true, device_name: "Kitchen speaker".into() }));
        let st = r#"{"type":"playback_state","status":"playing","item":{"uri":"spotify:track:2JRo0gjbX4GrCqBYdRohoo","entity_type":"track","decorations":{"identity":{"name":"My Song"},"playback":{"duration_ms":210000,"content_ratings":[]}}},"position":{"position_ms":45000,"timestamp_ms":1747654321000,"speed":1.0},"volume":65,"is_active":true}"#;
        match parse_event(st) {
            Some(Event::State { status, uri, duration_ms, position, volume }) => {
                assert_eq!(status, Status::Playing);
                assert_eq!(uri, "spotify:track:2JRo0gjbX4GrCqBYdRohoo");
                assert_eq!(duration_ms, 210_000);
                assert_eq!(position.estimate(1747654321000 + 1500), 46_500);
                assert_eq!(volume, Some(65));
            }
            e => panic!("{e:?}"),
        }
        assert_eq!(parse_event(r#"{"type":"playback_changed","status":"paused"}"#), Some(Event::Playback(Status::Paused)));
        assert_eq!(parse_event(r#"{"type":"error","message":"command requires authentication"}"#), Some(Event::Error("command requires authentication".into())));
        let p = parse_event(r#"{"type":"position_sync","position":{"position_ms":45000,"timestamp_ms":1000,"speed":0.0}}"#);
        assert!(matches!(p, Some(Event::Position(a)) if a.estimate(99_999) == 45_000), "paused position doesn't advance");
        assert_eq!(parse_event(r#"{"type":"command_result","command":"pause"}"#), None);
        assert_eq!(parse_event("not json"), None);
    }

    #[test]
    fn commands_match_the_documented_shape() {
        let v: serde_json::Value = serde_json::from_str(&command_json(&Command::Play(Some("spotify:track:x".into())))).unwrap();
        assert_eq!(v, serde_json::json!({"type":"command","command":"play","uri":"spotify:track:x"}));
        let v: serde_json::Value = serde_json::from_str(&command_json(&Command::SetVolume(150))).unwrap();
        assert_eq!(v["volume"], 100);
        let v: serde_json::Value = serde_json::from_str(&command_json(&Command::Seek(30_000))).unwrap();
        assert_eq!(v, serde_json::json!({"type":"command","command":"seek","position_ms":30000}));
    }

    #[test]
    fn build_expiry() {
        let v = "soloist 1.3.10.71 build 1794592912 (20261113) (g19412df6ea) (windows/x86_64)";
        let b = parse_build_time(v).unwrap();
        assert_eq!(b, 1794592912);
        assert_eq!(days_left(b, b + 2 * 86_400), 88);
        assert!(days_left(b, b + 91 * 86_400) < 0);
        // date fallback
        let d = parse_build_time("soloist 1.0 (20261113) (x)").unwrap();
        assert_eq!(days_left(d, d), 90);
    }

    #[test]
    fn loopback_only() {
        assert!(is_loopback("127.0.0.1"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.5"));
    }
}
