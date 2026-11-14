//! Declarative theme engine. Themes are plain TOML documents (no scripts),
//! stored in `<data>/themes/*.toml`, and applied to egui's style at runtime
//! without recompiling.

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ButtonStyle {
    #[default]
    Filled,
    Outline,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IconStyle {
    #[default]
    Solid,
    Outline,
    Text,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub accent: [u8; 3],
    pub accent2: [u8; 3],
    /// Hover/highlight accent.
    pub accent_hover: [u8; 3],
    pub background: [u8; 3],
    /// Sidebar background.
    pub sidebar: [u8; 3],
    pub panel: [u8; 3],
    pub surface: [u8; 3],
    pub text: [u8; 3],
    pub text_dim: [u8; 3],
    pub border: [u8; 3],
    /// 0..1 — panel opacity when "Translucent Panels" is on.
    pub panel_alpha: f32,
    pub rounding: f32,
    pub spacing: f32,
    pub font_size: f32,
    pub heading_size: f32,
    pub border_width: f32,
    pub shadow_intensity: f32,
    pub hover_strength: f32,
    pub animation_speed: f32,
    pub blur_intensity: f32,
    pub button_style: ButtonStyle,
    pub icon_style: IconStyle,
}

impl Default for Theme {
    fn default() -> Self {
        aqua_default()
    }
}

fn aqua_default() -> Theme {
    Theme {
        name: "Aqua Glass Default".into(),
        dark: true,
        accent: [0x62, 0xD6, 0xB2],      // seafoam
        accent2: [0x36, 0xB8, 0xAC],     // aqua
        accent_hover: [0x85, 0xE5, 0xC8],
        background: [0x0B, 0x15, 0x17],
        sidebar: [0x10, 0x1D, 0x1F],
        panel: [0x17, 0x27, 0x29],       // cards and panels
        surface: [0x20, 0x34, 0x36],     // elevated surfaces
        text: [0xF0, 0xF7, 0xF5],
        text_dim: [0x9B, 0xAE, 0xAA],
        border: [0x29, 0x40, 0x3F],
        panel_alpha: 0.92,
        rounding: 8.0,
        spacing: 8.0,
        font_size: 14.0,
        heading_size: 22.0,
        border_width: 1.0,
        shadow_intensity: 0.35,
        hover_strength: 0.6,
        animation_speed: 1.0,
        blur_intensity: 1.0,
        button_style: ButtonStyle::Filled,
        icon_style: IconStyle::Solid,
    }
}

pub fn builtin_themes() -> Vec<Theme> {
    let base = aqua_default();
    let mut v = vec![
        base.clone(),
        Theme {
            name: "Midnight Aqua".into(),
            accent: [45, 212, 191],
            accent2: [14, 165, 233],
            background: [3, 7, 18],
            panel: [8, 15, 30],
            surface: [15, 25, 45],
            border: [25, 45, 70],
            ..base.clone()
        },
        Theme {
            name: "AMOLED Black".into(),
            accent: [0, 229, 255],
            accent2: [0, 140, 255],
            background: [0, 0, 0],
            panel: [0, 0, 0],
            surface: [14, 14, 16],
            text: [235, 235, 240],
            text_dim: [130, 130, 140],
            border: [35, 35, 40],
            panel_alpha: 1.0,
            shadow_intensity: 0.0,
            ..base.clone()
        },
        Theme {
            name: "Minimal Dark".into(),
            accent: [148, 163, 184],
            accent2: [100, 116, 139],
            background: [24, 24, 27],
            panel: [30, 30, 34],
            surface: [39, 39, 44],
            text: [228, 228, 231],
            text_dim: [150, 150, 160],
            border: [55, 55, 62],
            rounding: 4.0,
            shadow_intensity: 0.0,
            button_style: ButtonStyle::Flat,
            ..base.clone()
        },
        Theme {
            name: "Minimal Light".into(),
            dark: false,
            accent: [8, 145, 178],
            accent2: [37, 99, 235],
            background: [248, 250, 252],
            panel: [241, 245, 249],
            surface: [226, 232, 240],
            text: [15, 23, 42],
            text_dim: [71, 85, 105],
            border: [203, 213, 225],
            rounding: 6.0,
            shadow_intensity: 0.2,
            button_style: ButtonStyle::Outline,
            ..base.clone()
        },
        // Original palette inspired by soft muted "pine" tones (not a copy of any theme file).
        Theme {
            name: "Rose Pine".into(),
            accent: [235, 188, 186],
            accent2: [156, 207, 216],
            background: [25, 23, 36],
            panel: [31, 29, 46],
            surface: [38, 35, 58],
            text: [224, 222, 244],
            text_dim: [144, 140, 170],
            border: [64, 61, 82],
            ..base.clone()
        },
        // Warm, coffee-toned original palette.
        Theme {
            name: "Mocha-inspired".into(),
            accent: [203, 166, 247],
            accent2: [137, 180, 250],
            background: [30, 30, 46],
            panel: [24, 24, 37],
            surface: [49, 50, 68],
            text: [205, 214, 244],
            text_dim: [147, 153, 178],
            border: [69, 71, 90],
            ..base.clone()
        },
        Theme { name: "Custom".into(), ..base },
    ];
    // derived colours for palettes that don't define them
    for t in v.iter_mut().skip(1) {
        if t.name != "Custom" {
            t.sidebar = lerp_rgb(t.background, t.panel, 0.6);
            t.accent_hover = lerp_rgb(t.accent, [255, 255, 255], 0.3);
        }
    }
    v
}

pub fn c(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn ca(rgb: [u8; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8)
}

/// Options from feature toggles that influence how a theme is applied.
pub struct ApplyOpts {
    pub custom_theme: bool,
    pub animations: bool,
    pub hover: bool,
    pub shadows: bool,
    pub translucent: bool,
    pub ui_scale: f32,
    /// Overrides accent (dynamic artwork colors).
    pub accent_override: Option<[u8; 3]>,
}

pub fn apply(ctx: &egui::Context, theme: &Theme, o: &ApplyOpts) {
    let fallback = Theme { name: "plain".into(), ..aqua_default() };
    let t = if o.custom_theme { theme } else { &fallback };
    let accent = o.accent_override.unwrap_or(t.accent);
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(if o.custom_theme { t.rounding.clamp(0.0, 30.0) as u8 } else { 4 });
    let panel_alpha = if o.translucent { t.panel_alpha } else { 1.0 };
    v.panel_fill = ca(t.panel, panel_alpha);
    v.window_fill = c(t.panel);
    v.extreme_bg_color = c(t.background);
    v.faint_bg_color = c(t.surface);
    v.override_text_color = Some(c(t.text));
    v.hyperlink_color = c(accent);
    // seafoam for selected controls, progress and toggles; dark text on it for contrast
    v.selection.bg_fill = c(accent);
    v.selection.stroke = Stroke::new(1.0, c(t.background));
    v.window_corner_radius = r;
    v.menu_corner_radius = r;
    v.window_stroke = Stroke::new(t.border_width, c(t.border));
    let shadow_a = if o.shadows { (t.shadow_intensity * 120.0) as u8 } else { 0 };
    v.window_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(shadow_a) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(shadow_a) };
    let (inactive_fill, stroke_w) = match t.button_style {
        ButtonStyle::Filled => (c(t.surface), 0.0),
        ButtonStyle::Outline => (Color32::TRANSPARENT, t.border_width.max(1.0)),
        ButtonStyle::Flat => (Color32::TRANSPARENT, 0.0),
    };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = c(t.panel);
    w.noninteractive.bg_stroke = Stroke::new(t.border_width, c(t.border));
    w.noninteractive.fg_stroke = Stroke::new(1.0, c(t.text_dim));
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = inactive_fill;
    w.inactive.weak_bg_fill = inactive_fill;
    w.inactive.bg_stroke = Stroke::new(stroke_w, c(t.border));
    w.inactive.fg_stroke = Stroke::new(1.0, c(t.text));
    w.inactive.corner_radius = r;
    let hover_fill = if o.hover {
        c(lerp_rgb(lerp_rgb(t.surface, t.border, 0.5), t.accent_hover, 0.12 * t.hover_strength.clamp(0.0, 1.0)))
    } else {
        inactive_fill
    };
    w.hovered.bg_fill = hover_fill;
    w.hovered.weak_bg_fill = hover_fill;
    w.hovered.bg_stroke = Stroke::new(stroke_w, c(t.border));
    w.hovered.fg_stroke = Stroke::new(1.5, c(t.text));
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = c(lerp_rgb(t.surface, accent, 0.3));
    w.active.weak_bg_fill = w.active.bg_fill;
    w.active.bg_stroke = Stroke::new(stroke_w, c(t.border));
    w.active.fg_stroke = Stroke::new(2.0, c(t.text));
    w.active.corner_radius = r;
    w.open = w.active;
    ctx.set_visuals(v);
    crate::ui::widgets::icon::STYLE.store(match t.icon_style { IconStyle::Solid => 0, IconStyle::Outline => 1, IconStyle::Text => 2 }, std::sync::atomic::Ordering::Relaxed);

    ctx.global_style_mut(|s| {
        let sp = if o.custom_theme { t.spacing.clamp(2.0, 24.0) } else { 6.0 };
        s.spacing.item_spacing = egui::vec2(sp, sp * 0.75);
        s.spacing.button_padding = egui::vec2(sp * 1.2, sp * 0.6);
        s.spacing.window_margin = egui::Margin::same(sp as i8 + 4);
        s.animation_time = if o.animations { 0.12 / t.animation_speed.clamp(0.2, 5.0) } else { 0.0 };
        let fs = if o.custom_theme { t.font_size.clamp(9.0, 28.0) } else { 14.0 };
        let hs = if o.custom_theme { t.heading_size.clamp(12.0, 48.0) } else { 22.0 };
        use egui::{FontFamily, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Small, FontId::new(fs * 0.8, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(fs, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(fs, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(hs, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(fs * 0.9, FontFamily::Monospace)),
        ]
        .into();
    });
    ctx.set_zoom_factor(o.ui_scale);
}

pub fn themes_dir() -> PathBuf {
    crate::util::data_dir().join("themes")
}

fn sanitize_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "theme".into() } else { s }
}

pub fn parse_theme(s: &str) -> anyhow::Result<Theme> {
    let mut t: Theme = toml::from_str(s)?;
    t.validate();
    if t.name.trim().is_empty() {
        anyhow::bail!("theme has no name");
    }
    Ok(t)
}

impl Theme {
    /// Clamp numeric fields; themes are untrusted input.
    pub fn validate(&mut self) {
        self.panel_alpha = self.panel_alpha.clamp(0.0, 1.0);
        self.rounding = self.rounding.clamp(0.0, 30.0);
        self.spacing = self.spacing.clamp(2.0, 24.0);
        self.font_size = self.font_size.clamp(9.0, 28.0);
        self.heading_size = self.heading_size.clamp(12.0, 48.0);
        self.border_width = self.border_width.clamp(0.0, 4.0);
        self.shadow_intensity = self.shadow_intensity.clamp(0.0, 1.0);
        self.hover_strength = self.hover_strength.clamp(0.0, 1.0);
        self.animation_speed = self.animation_speed.clamp(0.2, 5.0);
        self.blur_intensity = self.blur_intensity.clamp(0.0, 2.0);
        self.name = self.name.chars().take(64).collect();
    }
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

pub fn load_user_themes() -> (Vec<Theme>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    if let Ok(rd) = std::fs::read_dir(themes_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|s| parse_theme(&s)) {
                Ok(t) => out.push(t),
                Err(err) => errors.push(format!("{}: {err}", p.display())),
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, errors)
}

pub fn save_user_theme(t: &Theme) -> std::io::Result<PathBuf> {
    let p = themes_dir().join(format!("{}.toml", sanitize_filename(&t.name)));
    crate::util::atomic_write(&p, t.to_toml().as_bytes())?;
    Ok(p)
}

pub fn delete_user_theme(name: &str) -> std::io::Result<()> {
    std::fs::remove_file(themes_dir().join(format!("{}.toml", sanitize_filename(name))))
}

pub fn import_theme(path: &Path) -> anyhow::Result<Theme> {
    let s = std::fs::read_to_string(path)?;
    if s.len() > 64 * 1024 {
        anyhow::bail!("theme file too large");
    }
    let t = parse_theme(&s)?;
    save_user_theme(&t)?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_roundtrip() {
        let names: Vec<_> = builtin_themes().into_iter().map(|t| t.name).collect();
        for n in ["Aqua Glass Default", "Midnight Aqua", "AMOLED Black", "Minimal Dark", "Minimal Light", "Rose Pine", "Mocha-inspired", "Custom"] {
            assert!(names.iter().any(|x| x == n), "missing {n}");
        }
        for t in builtin_themes() {
            let back = parse_theme(&t.to_toml()).unwrap();
            assert_eq!(back, t);
        }
    }
    #[test]
    fn untrusted_values_clamped() {
        let t = parse_theme("name='x'\nrounding=999\nfont_size=-4\npanel_alpha=7").unwrap();
        assert_eq!(t.rounding, 30.0);
        assert_eq!(t.font_size, 9.0);
        assert_eq!(t.panel_alpha, 1.0);
    }
    #[test]
    fn invalid_theme_rejected() {
        assert!(parse_theme("name=''").is_err());
        assert!(parse_theme("accent = 'red'").is_err());
    }
    #[test]
    fn partial_theme_uses_defaults() {
        let t = parse_theme("name='Mine'\naccent=[255,0,0]").unwrap();
        assert_eq!(t.accent, [255, 0, 0]);
        assert_eq!(t.background, aqua_default().background);
    }
}
