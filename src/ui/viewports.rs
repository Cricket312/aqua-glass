//! Mini-player and desktop overlay windows. Each is a separate OS window that
//! exists only while open/enabled; when closed or disabled the window and its
//! GPU surface are destroyed.

use super::widgets::{self, icon};
use super::AquaApp;
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::MiniLayout;
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText};
use std::sync::atomic::Ordering;

/// Placeholder kept for API stability (state lives on AquaApp).
#[derive(Default, Clone)]
pub struct Shared;

pub fn show(app: &mut AquaApp, ctx: &egui::Context) {
    let mini = app.cfg.active("mini_player") && app.mini_open.load(Ordering::Relaxed);
    let overlay = app.cfg.active("overlay");
    if mini || overlay {
        // keep painting the satellite windows while the main window is minimized/hidden
        // (only then — sending it while visible would force continuous repaints)
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized || app.main_hidden {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::RequestPaintWhileHidden);
        }
        if app.state.playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
    if mini {
        mini_player(app, ctx);
    }
    if overlay {
        overlay_window(app, ctx);
    }
}

fn current_line(app: &AquaApp) -> Option<String> {
    let l = app.lyrics.shared.lock().unwrap();
    let l = l.as_ref()?;
    let pos = app.engine.position_ms() as i64 + app.cfg.lyrics.offset_ms as i64;
    let i = l.current_line(pos)?;
    l.line_text(i).map(|s| s.to_string())
}

fn mini_player(app: &mut AquaApp, ctx: &egui::Context) {
    let c = app.cfg.mini.clone();
    let mut b = egui::ViewportBuilder::default()
        .with_title("Aqua Glass — Mini")
        .with_inner_size(c.size)
        .with_min_inner_size([220.0, 80.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(true)
        .with_taskbar(false);
    if app.cfg.active("mini_on_top") {
        b = b.with_always_on_top();
    }
    if let Some(p) = c.pos {
        b = b.with_position(p);
    }
    let st = app.state.clone();
    let pos = app.engine.position_ms();
    let lyric = if app.cfg.active("mini_lyrics") { current_line(app) } else { None };
    let tex = st.current.as_ref().and_then(|t| app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover));
    let accent = app.accent();
    let mut close = false;
    let mut cmds = Vec::new();
    let mut geom = None;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("aqua-mini"), b, |ui, _class| {
        let ctx = ui.ctx().clone();
        let fill = ui.visuals().panel_fill.to_opaque().gamma_multiply(c.opacity);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill).corner_radius(10).inner_margin(8)).show(ui, |ui| {
            // drag anywhere on the background to move
            let bg = ui.interact(ui.max_rect(), egui::Id::new("mini-drag"), egui::Sense::drag());
            if bg.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            let compact = c.layout == MiniLayout::Compact;
            let art = match c.layout {
                MiniLayout::Square => (ui.available_width() * 0.6).min(ui.available_height() - 60.0).max(40.0),
                MiniLayout::Compact => 0.0,
                MiniLayout::Horizontal => (ui.available_height() - 4.0).clamp(40.0, 96.0),
            };
            let body = |ui: &mut egui::Ui, cmds: &mut Vec<Cmd>, close: &mut bool| {
                ui.horizontal(|ui| {
                    if let Some(t) = &st.current {
                        ui.add(egui::Label::new(RichText::new(&t.title).strong()).truncate());
                    } else {
                        ui.label("Nothing playing");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::small_button(ui, "✖", "Close mini-player").clicked() {
                            *close = true;
                        }
                    });
                });
                if let Some(t) = &st.current {
                    if !compact {
                        ui.add(egui::Label::new(RichText::new(t.artist_line()).weak()).truncate());
                    }
                }
                if let Some(l) = &lyric {
                    ui.add(egui::Label::new(RichText::new(l).color(accent).italics()).truncate());
                }
                ui.horizontal(|ui| {
                    if widgets::small_button(ui, icon::PREV(), "Previous").clicked() {
                        cmds.push(Cmd::Prev);
                    }
                    if ui.add(egui::Button::new(RichText::new(if st.playing { icon::PAUSE() } else { icon::PLAY() }).color(accent))).clicked() {
                        cmds.push(Cmd::Toggle);
                    }
                    if widgets::small_button(ui, icon::NEXT(), "Next").clicked() {
                        cmds.push(Cmd::Next);
                    }
                    if !compact {
                        ui.label(RichText::new(format!("{} / {}", fmt_ms(pos), fmt_ms(st.duration_ms))).small().monospace());
                    }
                });
                if st.duration_ms > 0 {
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::click());
                    ui.painter().rect_filled(r, 2, ui.visuals().faint_bg_color);
                    let mut f = r;
                    f.set_width(r.width() * (pos as f32 / st.duration_ms as f32).clamp(0.0, 1.0));
                    ui.painter().rect_filled(f, 2, accent);
                    if let Some(p) = resp.interact_pointer_pos() {
                        if resp.clicked() {
                            cmds.push(Cmd::Seek(((p.x - r.left()) / r.width() * st.duration_ms as f32) as u64));
                        }
                    }
                }
            };
            match c.layout {
                MiniLayout::Square => {
                    ui.vertical_centered(|ui| {
                        widgets::artwork(ui, tex.as_ref(), art, 8);
                        body(ui, &mut cmds, &mut close);
                    });
                }
                MiniLayout::Compact => body(ui, &mut cmds, &mut close),
                MiniLayout::Horizontal => {
                    ui.horizontal(|ui| {
                        widgets::artwork(ui, tex.as_ref(), art, 8);
                        ui.vertical(|ui| body(ui, &mut cmds, &mut close));
                    });
                }
            }
        });
        if ctx.input(|i| i.viewport().close_requested()) {
            close = true;
        }
        geom = ctx.input(|i| {
            let o = i.viewport().outer_rect?;
            let inner = i.viewport().inner_rect?;
            Some(([o.min.x, o.min.y], [inner.width(), inner.height()]))
        });
    });
    for c in cmds {
        app.engine.send(c);
    }
    if let Some((p, s)) = geom {
        if app.cfg.mini.pos != Some(p) || app.cfg.mini.size != s {
            app.cfg.mini.pos = Some(p);
            app.cfg.mini.size = s;
            app.mark_dirty();
        }
    }
    if close {
        app.mini_open.store(false, Ordering::Relaxed);
    }
}

fn overlay_window(app: &mut AquaApp, ctx: &egui::Context) {
    let o = app.cfg.overlay.clone();
    let mut b = egui::ViewportBuilder::default()
        .with_title("Aqua Glass — Overlay")
        .with_inner_size(o.size)
        .with_position(o.pos)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false)
        .with_mouse_passthrough(app.cfg.active("overlay_click_through"));
    if o.always_on_top {
        b = b.with_always_on_top();
    }
    let st = app.state.clone();
    let pos = app.engine.position_ms();
    let lyric = if app.cfg.active("overlay_lyrics") && app.cfg.active("lyrics") { current_line(app) } else { None };
    let tex = if app.cfg.active("overlay_artwork") { st.current.as_ref().and_then(|t| app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover)) } else { None };
    let show_art = app.cfg.active("overlay_artwork");
    let show_prog = app.cfg.active("overlay_progress");
    let accent = app.accent();
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("aqua-overlay"), b, |ui, _| {
        let fill = Color32::from_black_alpha((o.opacity * 200.0) as u8);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill).corner_radius(12).inner_margin(10)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if show_art {
                    widgets::artwork(ui, tex.as_ref(), (o.size[1] - 24.0).clamp(32.0, 120.0), 8);
                }
                ui.vertical(|ui| {
                    let Some(t) = &st.current else {
                        ui.label(RichText::new("Aqua Glass").size(o.font_size).color(Color32::WHITE));
                        return;
                    };
                    ui.add(egui::Label::new(RichText::new(&t.title).size(o.font_size).strong().color(Color32::WHITE)).truncate());
                    ui.add(egui::Label::new(RichText::new(t.artist_line()).size(o.font_size * 0.75).color(Color32::from_gray(200))).truncate());
                    if let Some(l) = &lyric {
                        ui.add(egui::Label::new(RichText::new(l).size(o.font_size * 0.85).color(accent)).truncate());
                    }
                    if show_prog && st.duration_ms > 0 {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 1, Color32::from_white_alpha(40));
                        let mut f = r;
                        f.set_width(r.width() * (pos as f32 / st.duration_ms as f32).clamp(0.0, 1.0));
                        ui.painter().rect_filled(f, 1, accent);
                    }
                });
            });
        });
    });
}
