//! Aqua Glass — native Spotify + YouTube Music client.

pub mod audio;
pub mod config;
pub mod features;
pub mod library;
pub mod matching;
pub mod model;
pub mod perf;
pub mod security;
pub mod services;
pub mod shortcuts;
pub mod theme;
pub mod transfer;
pub mod util;
pub mod artwork;
pub mod discord;
pub mod ui;
pub mod winint;

/// Start the application.
pub fn run() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let start_hidden = args.iter().any(|a| a == "--minimized");
    // `aqua-glass.exe song.mp3 other.flac` plays the files (enables "Open with")
    let files: Vec<std::path::PathBuf> = args.iter().skip(1).map(std::path::PathBuf::from).filter(|p| p.is_file()).collect();
    let start_page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();
    let (cfg, warning) = config::Config::load();
    util::init_logging(cfg.active("diagnostic_logging"));
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
            let mut app = ui::AquaApp::new(cc, cfg, warning, start_hidden);
            app.open_cli(files, start_page.as_deref());
            Ok(Box::new(app))
        }),
    )
}
