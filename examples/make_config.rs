//! Benchmark helper: print a settings.toml for a preset with extra toggles.
//! usage: make_config <Minimal|Balanced|Enhanced|FullyCustomized> [+feature|-feature]... [key=value]...
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
        } else if let Some(v) = a.strip_prefix("folder=") {
            c.services.local_folders = vec![v.into()];
        }
    }
    // no Windows-only integrations / network in the benchmark environment
    print!("{}", c.to_toml_string());
}
