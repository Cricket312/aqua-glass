//! Aqua Glass — native Spotify and local-files music client for Windows.

pub mod audio;
pub mod config;
pub mod crash;
pub mod features;
pub mod history;
pub mod radio;
pub mod library;
pub mod matching;
pub mod model;
pub mod perf;
pub mod security;
pub mod services;
pub mod shortcuts;
pub mod syncprep;
pub mod theme;
pub mod transfer;
pub mod util;
pub mod artwork;
pub mod discord;
pub mod ui;
pub mod winint;

/// Start the application.
pub fn run() -> eframe::Result<()> {
    util::init_platform();
    crash::install();
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--radio-selftest") {
        std::process::exit(radio_selftest_cli(args.get(i + 1).map(|s| s.as_str()), args.get(i + 2).map(|s| s.as_str())));
    }
    if let Some(i) = args.iter().position(|a| a == "--export-sync-snapshot") {
        std::process::exit(syncprep::export_snapshot_cli(args.get(i + 1).map(|s| s.as_str())));
    }
    if args.iter().any(|a| a == "--crash-test") {
        // verifies crash reporting end to end (crash.log + message box)
        let _ = std::thread::Builder::new().name("crash-test".into()).spawn(|| panic!("deliberate crash test")).map(|h| h.join());
    }
    let start_hidden = args.iter().any(|a| a == "--minimized");
    // `aqua-glass.exe song.mp3 other.flac` plays the files (enables "Open with")
    let files: Vec<std::path::PathBuf> = args.iter().skip(1).map(std::path::PathBuf::from).filter(|p| p.is_file()).collect();
    let start_page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();
    let (cfg, warning) = config::Config::load();
    let debug_log = args.iter().any(|a| a == "--debug-log");
    let safe_mode = args.iter().any(|a| a == "--safe-mode");
    util::init_logging(debug_log || cfg.active("diagnostic_logging"));
    log::info!("Aqua Glass {} starting (safe mode: {safe_mode}, custom Spotify app: {})", env!("CARGO_PKG_VERSION"), !cfg.services.spotify_client_id.trim().is_empty());
    services::install_crypto();
    let size = cfg.general.window_size.unwrap_or([1240.0, 780.0]);
    let mut vp = eframe::egui::ViewportBuilder::default()
        .with_title("Aqua Glass")
        .with_inner_size(size)
        .with_min_inner_size([720.0, 480.0])
        .with_icon(std::sync::Arc::new(winint::app_icon()))
        .with_transparent(cfg.active("window_backdrop"));
    if let Some(p) = cfg.general.window_pos {
        // ignore positions that are far off-screen (e.g. a disconnected monitor)
        if p[0] > -2000.0 && p[1] > -200.0 && p[0] < 10000.0 && p[1] < 6000.0 {
            vp = vp.with_position(p);
        }
    }
    let opts = eframe::NativeOptions {
        viewport: vp,
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native(
        "Aqua Glass",
        opts,
        Box::new(move |cc| {
            let mut app = ui::AquaApp::new(cc, cfg, warning, start_hidden, safe_mode);
            app.open_cli(files, start_page.as_deref());
            Ok(Box::new(app))
        }),
    )
}

/// `aqua-glass.exe --radio-selftest ["Artist" "Title"]` — runs the Similarity Radio
/// self-test against the real Last.fm API, prints the report (when started from a
/// terminal) and saves it to `<data>\\radio-selftest-<time>.txt`. Exit code 0 = all
/// checks passed. The key comes from `AQUA_LASTFM_KEY` or the one saved in Settings.
fn radio_selftest_cli(artist: Option<&str>, title: Option<&str>) -> i32 {
    #[cfg(windows)]
    unsafe {
        // GUI-subsystem exe: attach to the calling terminal so println! is visible
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    services::install_crypto();
    let key = std::env::var("AQUA_LASTFM_KEY").ok().filter(|k| !k.trim().is_empty()).or_else(|| security::get(security::keys::LASTFM_API_KEY)).unwrap_or_default();
    let artist = artist.unwrap_or("Radiohead");
    let title = title.unwrap_or("Karma Police");
    let mut lib = library::LibraryStore::open(util::data_dir().join("library"));
    let local = lib.tracks(config::ProviderKind::Local, library::LOCAL_ALL_ID).cloned().unwrap_or_default();
    let t = radio::self_test(&key, artist, title, &local);
    println!("{}", t.report);
    let path = util::data_dir().join(format!("radio-selftest-{}.txt", util::now_unix()));
    match util::atomic_write(&path, t.report.as_bytes()) {
        Ok(()) => println!("Report saved to {}", path.display()),
        Err(e) => eprintln!("Could not save report: {e}"),
    }
    if t.ok() { 0 } else { 1 }
}
