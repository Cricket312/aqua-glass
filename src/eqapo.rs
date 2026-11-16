//! Equalizer APO integration: Aqua Glass' EQ / bass / preamp / balance settings
//! drive Equalizer APO (<https://sourceforge.net/projects/equalizerapo/>), a
//! system-wide equalizer, so they also affect audio Aqua Glass doesn't output
//! itself — Spotify songs played by Spotify Soloist.
//!
//! * Aqua Glass only writes its own file, `aqua-glass.txt`, in Equalizer APO's
//!   config folder. Equalizer APO watches that folder and reloads changes by itself.
//! * `config.txt` gets one line, `Include: aqua-glass.txt`, and only when the user
//!   clicks Connect (a backup `config.txt.before-aqua-glass` is made first).
//!   Disconnect removes exactly that line again.
//! * Equalizer APO works per output device for *all* audio on it. Aqua Glass
//!   turns its own DSP off while this is in use, so local files aren't processed twice.

use crate::audio::dsp::DspParams;
use crate::config::EQ_FREQS;
use std::path::{Path, PathBuf};

pub const INCLUDE_FILE: &str = "aqua-glass.txt";
const INCLUDE_LINE: &str = "Include: aqua-glass.txt";
const MARKER: &str = "# Added by Aqua Glass (Settings › Audio › Equalizer engine). Remove both lines to disconnect.";

/// Equalizer APO's config folder, if Equalizer APO is installed.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        for (root, key) in [(HKEY_LOCAL_MACHINE, "SOFTWARE\\EqualizerAPO"), (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\EqualizerAPO")] {
            if let Ok(k) = winreg::RegKey::predef(root).open_subkey(key) {
                if let Ok(p) = k.get_value::<String, _>("ConfigPath") {
                    let p = PathBuf::from(p);
                    if p.join("config.txt").is_file() {
                        return Some(p);
                    }
                }
            }
        }
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
        let p = PathBuf::from(pf).join("EqualizerAPO").join("config");
        if p.join("config.txt").is_file() {
            return Some(p);
        }
        None
    }
    #[cfg(not(windows))]
    None
}

/// Equalizer APO config text for these settings. `active = false` writes a flat
/// (pass-through) file.
pub fn render(p: &DspParams, active: bool) -> String {
    let mut s = String::from("# Aqua Glass — written automatically from Settings › Audio. Don't edit: changes are overwritten.\n");
    if !active {
        s.push_str("# Aqua Glass EQ is off (flat).\n");
        return s;
    }
    let mut filters = Vec::new();
    if p.eq_on {
        for (i, f) in EQ_FREQS.iter().enumerate() {
            let g = p.eq_gains_db[i];
            if g.abs() > 0.05 {
                filters.push(format!("Filter: ON PK Fc {f:.0} Hz Gain {g:.2} dB Q 1.41"));
            }
        }
    }
    if p.bass_on && p.bass_db.abs() > 0.05 {
        filters.push(format!("Filter: ON LSC Fc 120 Hz Gain {:.2} dB Q 0.71", p.bass_db));
    }
    // Equalizer APO clips hard: lower the level by the curve's real peak (headroom)
    let preamp = p.preamp_db - crate::audio::dsp::peak_gain_db(p).max(0.0);
    if preamp.abs() > 0.005 {
        s.push_str(&format!("Preamp: {preamp:.2} dB\n"));
    }
    for f in filters {
        s.push_str(&f);
        s.push('\n');
    }
    if p.balance.abs() > 0.001 {
        let (l, r) = ((1.0 - p.balance).min(1.0), (1.0 + p.balance).min(1.0));
        s.push_str(&format!("Copy: L={l:.3}*L R={r:.3}*R\n"));
    }
    s
}

fn is_include_line(l: &str) -> bool {
    let t = l.trim();
    !t.starts_with('#') && t.to_ascii_lowercase().replace(' ', "") == "include:aqua-glass.txt"
}

/// `config.txt` already includes our file.
pub fn is_linked(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("config.txt")).map(|s| s.lines().any(is_include_line)).unwrap_or(false)
}

/// Add `Include: aqua-glass.txt` to `config.txt` (after a one-time backup).
pub fn link(dir: &Path) -> std::io::Result<()> {
    let cfg = dir.join("config.txt");
    let text = std::fs::read_to_string(&cfg)?;
    if text.lines().any(is_include_line) {
        return Ok(());
    }
    let backup = dir.join("config.txt.before-aqua-glass");
    if !backup.exists() {
        std::fs::write(&backup, &text)?;
    }
    // our file must exist before Equalizer APO sees the include
    if !dir.join(INCLUDE_FILE).exists() {
        std::fs::write(dir.join(INCLUDE_FILE), render(&DspParams::default(), false))?;
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(MARKER);
    out.push('\n');
    out.push_str(INCLUDE_LINE);
    out.push('\n');
    std::fs::write(&cfg, out)
}

/// Remove our two lines from `config.txt` and leave a flat `aqua-glass.txt`.
pub fn unlink(dir: &Path) -> std::io::Result<()> {
    let cfg = dir.join("config.txt");
    let text = std::fs::read_to_string(&cfg)?;
    let kept: Vec<&str> = text.lines().filter(|l| !is_include_line(l) && l.trim() != MARKER).collect();
    let mut out = kept.join("\n");
    out.push('\n');
    std::fs::write(&cfg, out)?;
    write(dir, &render(&DspParams::default(), false))
}

/// Write our file in place (Equalizer APO reloads it automatically).
pub fn write(dir: &Path, content: &str) -> std::io::Result<()> {
    let p = dir.join(INCLUDE_FILE);
    if std::fs::read_to_string(&p).map(|old| old == content).unwrap_or(false) {
        return Ok(());
    }
    std::fs::write(p, content)
}

/// A friendlier message for the usual failure (no write access to Program Files).
pub fn explain(e: &std::io::Error, dir: &Path) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!("Windows didn't allow Aqua Glass to change files in {}. Equalizer APO normally lets its config folder be edited without admin rights; if yours doesn't, right-click that folder › Properties › Security › Edit, select “Users” and allow “Modify”, then try again.", dir.display())
    } else {
        format!("{e} ({})", dir.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_equalizer_apo_syntax_with_headroom() {
        let mut g = [0.0; 10];
        g[0] = 4.0; // 31 Hz
        g[6] = -3.0;
        let p = DspParams { eq_on: true, eq_gains_db: g, bass_on: true, bass_db: 6.0, preamp_db: 0.0, balance: 0.25 };
        let s = render(&p, true);
        assert!(s.contains("Filter: ON PK Fc 31 Hz Gain 4.00 dB Q 1.41"), "{s}");
        assert!(s.contains("Gain -3.00 dB"), "{s}");
        assert!(s.contains("Filter: ON LSC Fc 120 Hz Gain 6.00 dB Q 0.71"), "{s}");
        let pre: f32 = s.lines().find_map(|l| l.strip_prefix("Preamp: ")).unwrap().trim_end_matches(" dB").parse().unwrap();
        // +4 dB at 31 Hz sits under the fully-on +6 dB shelf → the curve really peaks at ≈ +10
        assert!((-10.3..-9.7).contains(&pre), "headroom from the real curve peak: {pre}");
        // "Vocal" (cuts low, boosts mids) + 14.5 dB bass: adding the largest boosts would
        // say 17.5 dB; the real curve needs less because the low cuts offset the shelf
        let vocal = [-2.0, -1.0, 0.0, 1.5, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0];
        let p2 = DspParams { eq_on: true, eq_gains_db: vocal, bass_on: true, bass_db: 14.5, preamp_db: 0.0, balance: 0.0 };
        let peak = crate::audio::dsp::peak_gain_db(&p2);
        assert!(peak < 16.0 && peak > 12.0, "{peak}");
        assert!(s.contains("Copy: L=0.750*L R=1.000*R"), "{s}");
        // decimal points, never commas (Equalizer APO's parser)
        assert!(!s.lines().filter(|l| !l.starts_with('#')).any(|l| l.contains(',')));
        // flat when off
        let flat = render(&p, false);
        assert!(!flat.contains("Filter") && !flat.contains("Preamp"));
        // EQ feature off but bass on → only the shelf
        let s = render(&DspParams { eq_on: false, ..p.clone() }, true);
        assert!(!s.contains(" PK ") && s.contains("LSC"));
    }

    #[test]
    fn link_and_unlink_touch_only_our_lines() {
        let dir = std::env::temp_dir().join(format!("aqua-apo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = "Preamp: -3 dB\nInclude: example.txt\n";
        std::fs::write(dir.join("config.txt"), original).unwrap();
        assert!(!is_linked(&dir));
        link(&dir).unwrap();
        assert!(is_linked(&dir));
        assert!(dir.join(INCLUDE_FILE).exists(), "our file exists before APO reads the include");
        assert_eq!(std::fs::read_to_string(dir.join("config.txt.before-aqua-glass")).unwrap(), original);
        link(&dir).unwrap(); // idempotent
        assert_eq!(std::fs::read_to_string(dir.join("config.txt")).unwrap().matches("aqua-glass.txt").count(), 1);
        write(&dir, "Preamp: -1 dB\n").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(INCLUDE_FILE)).unwrap(), "Preamp: -1 dB\n");
        unlink(&dir).unwrap();
        assert!(!is_linked(&dir));
        assert_eq!(std::fs::read_to_string(dir.join("config.txt")).unwrap(), original, "user's config restored exactly");
        assert!(!std::fs::read_to_string(dir.join(INCLUDE_FILE)).unwrap().contains("Filter"), "flat after disconnect");
        let _ = std::fs::remove_dir_all(dir);
    }
}
