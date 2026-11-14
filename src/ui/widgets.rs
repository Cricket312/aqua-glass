//! Small reusable widgets: icons, provider badges, RAM impact badges, rows.

use crate::config::ProviderKind;
use crate::features::{FeatureDef, Impact};
use crate::perf::Measurement;
use eframe::egui::{self, Color32, RichText, Ui, Vec2};

/// Icon glyphs, following the theme's icon style (solid / outline / text).
#[allow(non_snake_case)]
pub mod icon {
    use std::sync::atomic::{AtomicU8, Ordering};
    /// 0 = solid, 1 = outline, 2 = text labels. Set by the theme engine.
    pub static STYLE: AtomicU8 = AtomicU8::new(0);
    fn pick(solid: &'static str, outline: &'static str, text: &'static str) -> &'static str {
        match STYLE.load(Ordering::Relaxed) {
            1 => outline,
            2 => text,
            _ => solid,
        }
    }
    pub fn HOME() -> &'static str { pick("🏠", "☆", "Home") }
    pub fn SEARCH() -> &'static str { pick("🔍", "🔎", "Find") }
    pub fn LIBRARY() -> &'static str { pick("📚", "🗀", "Lib") }
    pub fn PLAYLIST() -> &'static str { pick("☰", "⎘", "List") }
    pub fn ARTIST() -> &'static str { pick("👤", "○", "Art") }
    pub fn ALBUM() -> &'static str { pick("💿", "○", "Alb") }
    pub fn NOW() -> &'static str { pick("🎵", "🎼", "Now") }
    pub fn QUEUE() -> &'static str { pick("⏏", "⎘", "Queue") }
    pub fn TRANSFER() -> &'static str { pick("🔃", "⟲", "Xfer") }
    pub fn SETTINGS() -> &'static str { pick("⚙", "⛭", "Settings") }
    pub fn PLAY() -> &'static str { pick("▶", "⏵", "Play") }
    pub fn PAUSE() -> &'static str { pick("⏸", "⏸", "Pause") }
    pub fn NEXT() -> &'static str { pick("⏭", "⏩", "Next") }
    pub fn PREV() -> &'static str { pick("⏮", "⏪", "Prev") }
    pub fn SHUFFLE() -> &'static str { pick("🔀", "🔀", "Shuffle") }
    pub fn REPEAT() -> &'static str { pick("🔁", "⟲", "Repeat") }
    pub fn REPEAT_ONE() -> &'static str { pick("🔂", "🔂", "Repeat 1") }
    pub fn VOL() -> &'static str { pick("🔊", "🔉", "Vol") }
    pub fn MUTE() -> &'static str { pick("🔇", "🔈", "Muted") }
    pub fn LYRICS() -> &'static str { pick("🎤", "🎼", "Lyrics") }
    pub fn MINI() -> &'static str { pick("🗗", "🖼", "Mini") }
    pub fn HEART() -> &'static str { pick("♥", "♡", "Like") }
    pub fn MORE() -> &'static str { pick("…", "…", "More") }
    pub fn QUICK() -> &'static str { pick("⚡", "⚡", "Quick") }
    pub fn SYNC() -> &'static str { pick("🔄", "⟲", "Sync") }
    pub fn EXTERNAL() -> &'static str { pick("↗", "↗", "Open") }
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
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::NOW(), egui::FontId::proportional((size * 0.4).min(48.0)), v.weak_text_color());
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
    ui.add_space(14.0);
    ui.label(RichText::new(text).strong().size(16.0));
    ui.add_space(4.0);
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

/// iOS-style toggle switch (seafoam when on).
pub fn switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(34.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CheckBox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let v = ui.visuals();
        let off_bg = v.widgets.inactive.bg_fill;
        let on_bg = v.selection.bg_fill;
        let bg = lerp_color(off_bg, on_bg, t);
        let r = rect.height() / 2.0;
        ui.painter().rect(rect, r, bg, egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
        let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        let knob = if *on { v.extreme_bg_color } else { v.widgets.inactive.fg_stroke.color };
        ui.painter().circle_filled(egui::pos2(cx, rect.center().y), r - 3.0, knob);
    }
    resp
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Rounded content card used by settings and dashboards.
pub fn card<R>(ui: &mut Ui, title: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = ui.visuals().faint_bg_color;
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let r = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(stroke.width.min(1.0), stroke.color))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(t) = title {
                ui.label(RichText::new(t).strong().size(15.0));
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(10.0);
    r
}

/// Section header with an optional "Show all" link. Returns true if the link was clicked.
pub fn section_header(ui: &mut Ui, title: &str, show_all: bool) -> bool {
    let mut clicked = false;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(19.0));
        if show_all {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = ui.add(egui::Label::new(RichText::new("Show all").small().strong().color(ui.visuals().weak_text_color())).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            });
        }
    });
    ui.add_space(6.0);
    clicked
}

/// Number of cards per row and card width for the available width (≈5–6 per row).
pub fn grid_metrics(avail: f32) -> (usize, f32) {
    grid_metrics_min(avail, 140.0)
}

pub fn grid_metrics_min(avail: f32, min_w: f32) -> (usize, f32) {
    let gap = 16.0;
    let avail = avail - 18.0; // leave room for the scrollbar
    let n = ((avail + gap) / (min_w + gap)).floor().clamp(2.0, 9.0) as usize;
    let w = ((avail - gap * (n as f32 - 1.0)) / n as f32).floor();
    (n, w)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CardShape {
    Square,
    Round,
    /// Landscape artwork (16:10), as in "Made For You".
    Wide,
}

pub struct CardOut {
    pub open: bool,
    pub play: bool,
}

/// Art-first card: artwork, name, subtitle; a subtle highlight and a seafoam play
/// button on hover/focus. No box around it, like the reference design.
pub fn media_card(ui: &mut Ui, tex: Option<&egui::TextureHandle>, title: &str, subtitle: &str, provider: Option<ProviderKind>, w: f32, shape: CardShape, accent: Color32) -> CardOut {
    let art_h = match shape {
        CardShape::Wide => (w * 0.62).round(),
        _ => w,
    };
    let h = art_h + 46.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let hovered = resp.hovered() || resp.has_focus();
    let v = ui.visuals().clone();
    let p = ui.painter_at(rect.expand(6.0));
    if hovered {
        p.rect_filled(rect.expand(5.0), 10, v.widgets.inactive.bg_fill.gamma_multiply(0.55));
    }
    let art_rect = egui::Rect::from_min_size(rect.min, egui::vec2(w, art_h));
    let ar = if shape == CardShape::Round { (w / 2.0) as u8 } else { 8 };
    match tex {
        Some(t) => {
            // cover-fit crop
            let ts = t.size_vec2();
            let s = (w / ts.x).max(art_h / ts.y);
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2((w / (ts.x * s)).min(1.0), (art_h / (ts.y * s)).min(1.0)));
            egui::Image::new(egui::load::SizedTexture::new(t.id(), art_rect.size())).uv(uv).corner_radius(ar).paint_at(ui, art_rect);
        }
        None => {
            p.rect_filled(art_rect, ar, v.widgets.inactive.bg_fill);
            p.text(art_rect.center(), egui::Align2::CENTER_CENTER, icon::NOW(), egui::FontId::proportional((art_h * 0.3).min(48.0)), v.weak_text_color());
        }
    }
    let ty = art_rect.bottom() + 7.0;
    let clip = egui::Rect::from_min_max(egui::pos2(rect.left(), ty), egui::pos2(rect.right(), rect.bottom()));
    let pc = ui.painter_at(clip);
    let title_g = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(14.0), v.strong_text_color());
    pc.galley(egui::pos2(rect.left(), ty), title_g, Color32::WHITE);
    let mut sx = rect.left();
    if let Some(pk) = provider {
        let c = provider_color(pk);
        let r = egui::Rect::from_min_size(egui::pos2(sx, ty + 21.0), egui::vec2(18.0, 13.0));
        pc.rect_filled(r, 3, c.gamma_multiply(0.18));
        pc.text(r.center(), egui::Align2::CENTER_CENTER, pk.short(), egui::FontId::proportional(8.5), c);
        sx += 22.0;
    }
    let sub_g = ui.painter().layout_no_wrap(subtitle.to_string(), egui::FontId::proportional(12.0), v.weak_text_color());
    pc.galley(egui::pos2(sx, ty + 20.0), sub_g, Color32::WHITE);
    let mut play = false;
    if hovered {
        let c = art_rect.right_bottom() - egui::vec2(24.0, 24.0);
        let br = egui::Rect::from_center_size(c, egui::vec2(36.0, 36.0));
        let pr = ui.interact(br, resp.id.with("play"), egui::Sense::click());
        p.circle_filled(c, 17.0, if pr.hovered() { lerp_color(accent, Color32::WHITE, 0.15) } else { accent });
        p.text(c + egui::vec2(1.0, 0.0), egui::Align2::CENTER_CENTER, icon::PLAY(), egui::FontId::proportional(14.0), v.extreme_bg_color);
        play = pr.clicked();
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    CardOut { open: resp.clicked() && !play, play: play || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) }
}

/// Fill a rounded rectangle with a horizontal gradient (`color_at(t)`, t = 0 left .. 1 right).
pub fn gradient_rounded(p: &egui::Painter, rect: egui::Rect, radius: f32, color_at: impl Fn(f32) -> Color32) {
    let mut pts = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(&mut pts, egui::epaint::RoundedRect::new(rect, radius));
    if pts.len() < 3 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let tx = |x: f32| ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    // centre-row fan: one centre vertex per horizontal band keeps the gradient linear
    let c = rect.center();
    mesh.colored_vertex(c, color_at(0.5));
    for q in &pts {
        mesh.colored_vertex(*q, color_at(tx(q.x)));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    // left/right halves: add strips through the vertical centre line for a smoother ramp
    p.add(egui::Shape::mesh(mesh));
}
