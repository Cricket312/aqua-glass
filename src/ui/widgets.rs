//! Small reusable widgets: icons, provider badges, RAM impact badges, rows.

use crate::config::ProviderKind;
use crate::features::{FeatureDef, Impact};
use crate::perf::Measurement;
use eframe::egui::{self, Color32, RichText, Ui, Vec2};

pub mod icon {
    pub const HOME: &str = "🏠";
    pub const SEARCH: &str = "🔍";
    pub const LIBRARY: &str = "📚";
    pub const PLAYLIST: &str = "☰";
    pub const ARTIST: &str = "👤";
    pub const ALBUM: &str = "💿";
    pub const NOW: &str = "🎵";
    pub const QUEUE: &str = "⏏";
    pub const TRANSFER: &str = "⇄";
    pub const SETTINGS: &str = "⚙";
    pub const PLAY: &str = "▶";
    pub const PAUSE: &str = "⏸";
    pub const NEXT: &str = "⏭";
    pub const PREV: &str = "⏮";
    pub const SHUFFLE: &str = "🔀";
    pub const REPEAT: &str = "🔁";
    pub const REPEAT_ONE: &str = "🔂";
    pub const VOL: &str = "🔊";
    pub const MUTE: &str = "🔇";
    pub const LYRICS: &str = "🗒";
    pub const MINI: &str = "🗗";
    pub const HEART: &str = "♥";
    pub const MORE: &str = "⋯";
    pub const QUICK: &str = "⚡";
    pub const SYNC: &str = "🔄";
    pub const EXTERNAL: &str = "↗";
}

pub fn provider_color(p: ProviderKind) -> Color32 {
    match p {
        ProviderKind::Spotify => Color32::from_rgb(30, 215, 96),
        ProviderKind::YouTube => Color32::from_rgb(255, 64, 64),
        ProviderKind::Local => Color32::from_rgb(160, 170, 190),
    }
}

/// Small coloured provider tag ("SP", "YT", "LO").
pub fn provider_badge(ui: &mut Ui, p: ProviderKind) -> egui::Response {
    let c = provider_color(p);
    let r = egui::Frame::new()
        .fill(c.gamma_multiply(0.18))
        .stroke(egui::Stroke::new(1.0, c.gamma_multiply(0.7)))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| ui.label(RichText::new(p.short()).small().color(c).strong()))
        .response;
    r.on_hover_text(p.label())
}

pub fn impact_color(i: Impact) -> Color32 {
    match i {
        Impact::Low => Color32::from_rgb(52, 211, 153),
        Impact::Moderate => Color32::from_rgb(250, 204, 21),
        Impact::High => Color32::from_rgb(251, 146, 60),
        Impact::VeryHigh => Color32::from_rgb(248, 113, 113),
    }
}

/// RAM impact badge with estimate vs. measurement made explicit.
pub fn ram_badge(ui: &mut Ui, f: &FeatureDef, m: Option<&Measurement>) {
    let (impact, text, tip) = match m {
        Some(m) if m.samples > 0 => (
            Impact::classify(m.delta_mb),
            format!("Measured: +{:.1} MB", m.delta_mb),
            format!(
                "Measured on this PC: mean increase of the private working set {:.1} MB over {} sample(s), last {}. Estimate was +{}–{} MB.",
                m.delta_mb,
                m.samples,
                crate::util::fmt_unix(m.measured_at),
                f.est_mb.0,
                f.est_mb.1
            ),
        ),
        _ => (
            f.impact,
            if f.est_mb.1 <= 0.05 { "Estimated: negligible".to_string() } else { format!("Estimated: +{}–{} MB", trim(f.est_mb.0), trim(f.est_mb.1)) },
            "Engineering estimate — not yet measured on this PC. Aqua Glass measures the real increase a few seconds after you enable the feature.".to_string(),
        ),
    };
    let c = impact_color(impact);
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(c.gamma_multiply(0.15))
            .corner_radius(4)
            .inner_margin(egui::Margin::symmetric(5, 1))
            .show(ui, |ui| ui.label(RichText::new(format!("RAM: {}", impact.label())).small().strong().color(c)))
            .response
            .on_hover_text(tip.clone());
        ui.label(RichText::new(text).small().weak()).on_hover_text(tip);
    });
}

fn trim(v: f32) -> String {
    if v.fract() == 0.0 { format!("{}", v as i32) } else { format!("{v:.1}") }
}

/// A square artwork image or placeholder.
pub fn artwork(ui: &mut Ui, tex: Option<&egui::TextureHandle>, size: f32, rounding: u8) -> egui::Response {
    let sz = Vec2::splat(size);
    match tex {
        Some(t) => {
            let ts = t.size_vec2();
            let scale = size / ts.x.max(ts.y).max(1.0);
            let img_sz = ts * scale;
            let (rect, resp) = ui.allocate_exact_size(sz, egui::Sense::click());
            let r = egui::Rect::from_center_size(rect.center(), img_sz);
            egui::Image::new(egui::load::SizedTexture::new(t.id(), img_sz)).corner_radius(rounding).paint_at(ui, r);
            resp
        }
        None => {
            let (rect, resp) = ui.allocate_exact_size(sz, egui::Sense::click());
            let v = ui.visuals();
            ui.painter().rect_filled(rect, rounding, v.faint_bg_color);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::NOW, egui::FontId::proportional(size * 0.4), v.weak_text_color());
            resp
        }
    }
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).heading().strong());
    ui.add_space(2.0);
}

pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).strong().size(16.0));
    ui.separator();
}

pub fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).small().weak());
}

pub fn warn(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(format!("⚠ {text}")).color(Color32::from_rgb(251, 191, 36)));
}

pub fn error(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(format!("✖ {text}")).color(Color32::from_rgb(248, 113, 113)));
}

pub fn ok(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(format!("✔ {text}")).color(Color32::from_rgb(52, 211, 153)));
}

/// Horizontal memory budget bar.
pub fn budget_bar(ui: &mut Ui, used: f32, target: f32, accent: Color32) {
    let w = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6, ui.visuals().faint_bg_color);
    let frac = (used / target.max(1.0)).clamp(0.0, 1.0);
    let c = if used > target {
        impact_color(Impact::VeryHigh)
    } else if used > target * 0.85 {
        impact_color(Impact::High)
    } else {
        accent
    };
    let mut fill = rect;
    fill.set_width(rect.width() * frac);
    p.rect_filled(fill, 6, c);
}

/// Sparkline of recent values.
pub fn sparkline(ui: &mut Ui, values: &[f32], target: f32, color: Color32) {
    let w = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 60.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4, ui.visuals().faint_bg_color.gamma_multiply(0.6));
    if values.len() < 2 {
        return;
    }
    let max = values.iter().cloned().fold(target, f32::max) * 1.1;
    let pts: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| egui::pos2(rect.left() + rect.width() * i as f32 / (values.len() - 1) as f32, rect.bottom() - rect.height() * v / max))
        .collect();
    let ty = rect.bottom() - rect.height() * target / max;
    p.hline(rect.x_range(), ty, egui::Stroke::new(1.0, impact_color(Impact::High).gamma_multiply(0.6)));
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
}

pub fn small_button(ui: &mut Ui, text: &str, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame(false)).on_hover_text(tip)
}
