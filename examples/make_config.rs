//! Benchmark helper: print a settings.toml for a preset with extra toggles.
//! usage: make_config <Minimal|Balanced|Enhanced|FullyCustomized> [+feature|-feature]... [key=value]...
//! keys: bg_image_path=, font=, repeat=all, window=WxH, layout=Auto|Landscape|Portrait, folder=,
//!       eq=g31,g62,…,g16k (dB), bass=dB, preamp=dB, volume=0..1
use aqua_glass::config::Config;
use aqua_glass::features::Preset;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut c = Config::default();
    let p = match args.first().map(|s| s.as_str()) {
        Some("Minimal") => Preset::Minimal,
        Some("Enhanced") => Preset::Enhanced,
        Some("FullyCustomized") => Preset::FullyCustomized,
        _ => Preset::Balanced,
    };
    c.apply_preset(p);
    for a in args.iter().skip(1) {
        if let Some(f) = a.strip_prefix('+') {
            c.features.insert(f.into(), true);
        } else if let Some(f) = a.strip_prefix('-') {
            c.features.insert(f.into(), false);
        } else if let Some(v) = a.strip_prefix("bg_image_path=") {
            c.background.image_path = v.into();
        } else if let Some(v) = a.strip_prefix("font=") {
            c.appearance.font_path = v.into();
        } else if a == "repeat=all" {
            c.playback.repeat = aqua_glass::config::RepeatMode::All;
        } else if let Some(v) = a.strip_prefix("window=") {
            // window=WIDTHxHEIGHT
            if let Some((w, h)) = v.split_once('x') {
                c.general.window_size = Some([w.parse().unwrap_or(1240.0), h.parse().unwrap_or(780.0)]);
            }
        } else if let Some(v) = a.strip_prefix("layout=") {
            c.general.layout_mode = match v {
                "Portrait" => aqua_glass::config::LayoutMode::Portrait,
                "Landscape" => aqua_glass::config::LayoutMode::Landscape,
                _ => aqua_glass::config::LayoutMode::Auto,
            };
        } else if let Some(v) = a.strip_prefix("eq=") {
            for (i, g) in v.split(',').take(10).enumerate() {
                c.audio.eq_gains_db[i] = g.parse().unwrap_or(0.0);
            }
            c.audio.eq_preset = "Custom".into();
        } else if let Some(v) = a.strip_prefix("bass=") {
            c.audio.bass_db = v.parse().unwrap_or(6.0);
        } else if let Some(v) = a.strip_prefix("preamp=") {
            c.audio.preamp_db = v.parse().unwrap_or(0.0);
        } else if let Some(v) = a.strip_prefix("volume=") {
            c.playback.volume = v.parse().unwrap_or(1.0);
        } else if let Some(v) = a.strip_prefix("folder=") {
            c.services.local_folders = vec![v.into()];
        }
    }
    // no Windows-only integrations / network in the benchmark environment
    print!("{}", c.to_toml_string());
}
