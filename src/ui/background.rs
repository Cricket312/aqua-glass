//! Window background: solid color, custom image, album artwork, blur, dimming,
//! animated gradient, parallax and cross-fade transitions — each independently
//! toggleable. Nothing here runs (and no texture is held) when disabled.

use super::AquaApp;
use crate::artwork::Kind;
use crate::config::BgScale;
use eframe::egui::{self, Color32, Pos2, Rect, Ui};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct BgState {
    /// A background is visible (central panel becomes transparent).
    pub active: bool,
    pub theme_dirty: bool,
    pub prev: Option<(egui::TextureHandle, Instant)>,
    cur_src: String,
    cur: Option<egui::TextureHandle>,
}

pub fn paint(app: &mut AquaApp, ui: &mut Ui) {
    let rect = ui.max_rect();
    let painter = ui.ctx().layer_painter(egui::LayerId::background());
    let theme = app.current_theme().clone();
    let base = if app.cfg.active("window_backdrop") { Color32::TRANSPARENT } else { crate::theme::c(theme.background) };
    painter.rect_filled(rect, 0, base);
    let c = app.cfg.clone();
    let mut active = false;
    if c.active("bg_gradient") && c.active("custom_theme") {
        // faint static aqua glows (top-left and lower-right); no texture, no repaint
        let a2 = theme.accent2;
        let big = rect.width().max(rect.height());
        radial(&painter, rect.left_top() + egui::vec2(rect.width() * 0.18, rect.height() * 0.05), big * 0.65, Color32::from_rgba_unmultiplied(a2[0], a2[1], a2[2], 22));
        radial(&painter, rect.right_bottom() - egui::vec2(rect.width() * 0.2, rect.height() * 0.15), big * 0.55, Color32::from_rgba_unmultiplied(a2[0], a2[1], a2[2], 14));
        active = true;
    }

    // image source: album art takes precedence when enabled and available
    let art = if c.active("bg_album_art") { app.state.current.as_ref().map(|t| t.artwork_url.clone()).filter(|u| !u.is_empty()) } else { None };
    let img = if c.active("bg_image") && !c.background.image_path.trim().is_empty() { Some(c.background.image_path.clone()) } else { None };
    let src = art.or(img);
    if let Some(src) = src {
        let blur = if c.active("bg_blur") { (c.background.blur_radius * theme.blur_intensity) as u32 } else { 0 };
        // blurred images are rendered small (they're blurry anyway); sharp ones at window size
        let px = if blur > 0 { 384 } else { (rect.width().max(rect.height()) * ui.ctx().pixels_per_point()).min(1920.0) as u32 / 256 * 256 + 256 };
        let tex = app.artwork.lock().unwrap().get(&src, Kind::Background { px, blur });
        if src != app.bg.cur_src {
            if c.active("bg_transitions") {
                if let Some(old) = app.bg.cur.take() {
                    app.bg.prev = Some((old, Instant::now()));
                }
            }
            app.bg.cur_src = src.clone();
            app.bg.cur = None;
        }
        if let Some(t) = tex {
            app.bg.cur = Some(t);
        }
        let parallax = if c.active("bg_parallax") {
            let m = ui.ctx().pointer_hover_pos().unwrap_or(rect.center());
            let d = (m - rect.center()) / rect.size().max(egui::vec2(1.0, 1.0));
            -d * c.background.parallax_px
        } else {
            egui::Vec2::ZERO
        };
        let tdur = Duration::from_millis(c.background.transition_ms as u64);
        let mut fade_in = 1.0;
        if let Some((old, at)) = &app.bg.prev {
            let t = at.elapsed().as_secs_f32() / tdur.as_secs_f32().max(0.001);
            if t >= 1.0 || !c.active("bg_transitions") {
                app.bg.prev = None;
            } else {
                draw_image(&painter, rect, old, c.background.scale, c.background.opacity * (1.0 - t), parallax);
                fade_in = t;
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        }
        if let Some(t) = &app.bg.cur {
            draw_image(&painter, rect, t, c.background.scale, c.background.opacity * fade_in, parallax);
            active = true;
        }
        if active && c.active("bg_dim") {
            painter.rect_filled(rect, 0, Color32::from_black_alpha((c.background.dim * 255.0) as u8));
        }
    } else if app.bg.cur.is_some() || app.bg.prev.is_some() {
        // feature turned off: release textures
        app.bg.cur = None;
        app.bg.prev = None;
        app.bg.cur_src.clear();
    }

    if c.active("bg_animated") {
        let fps = if app.pressure >= crate::perf::Pressure::Elevated { c.background.anim_fps.min(15) } else { c.background.anim_fps };
        let t = ui.input(|i| i.time) as f32 * 0.15 * c.background.anim_speed;
        let accent = app.accent_override.unwrap_or(theme.accent);
        let cols = [accent, theme.accent2, crate::theme::lerp_rgb(accent, theme.accent2, 0.5)];
        for (i, col) in cols.iter().enumerate() {
            let fi = i as f32 * 2.1;
            let center = Pos2::new(rect.center().x + (t + fi).sin() * rect.width() * 0.35, rect.center().y + (t * 0.8 + fi * 1.3).cos() * rect.height() * 0.3);
            radial(&painter, center, rect.width().max(rect.height()) * 0.45, Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 46));
        }
        active = true;
        ui.ctx().request_repaint_after(Duration::from_millis(1000 / fps.max(1) as u64));
    }
    app.bg.active = active || c.active("window_backdrop");
}

fn draw_image(p: &egui::Painter, rect: Rect, t: &egui::TextureHandle, scale: BgScale, opacity: f32, offset: egui::Vec2) {
    let ts = t.size_vec2();
    let size = match scale {
        BgScale::Stretch => rect.size(),
        BgScale::Center => ts,
        BgScale::Fill => {
            let s = (rect.width() / ts.x).max(rect.height() / ts.y);
            ts * s
        }
        BgScale::Fit => {
            let s = (rect.width() / ts.x).min(rect.height() / ts.y);
            ts * s
        }
    };
    // a little oversize so parallax never shows edges
    let size = if offset != egui::Vec2::ZERO { size * 1.06 } else { size };
    let r = Rect::from_center_size(rect.center() + offset, size);
    p.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8));
}

/// Soft radial gradient blob as a triangle fan.
fn radial(p: &egui::Painter, c: Pos2, r: f32, col: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, col);
    let n = 48;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(c + egui::vec2(a.cos(), a.sin()) * r, Color32::TRANSPARENT);
        if i > 0 {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
