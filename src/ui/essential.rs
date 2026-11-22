//! Essential Mode: an ultra-light window that shows only the current song title,
//! the transport (shuffle, previous, play/pause, next, repeat) and the volume.
//!
//! Entering it does more than hide the interface:
//! * cover art / background textures and decoded images are released, live page
//!   caches (playlists, artist pages, search results, lyrics) dropped, the loaded
//!   playlist track lists evicted from memory (they stay on disk);
//! * the visualizer tap, lyrics fetching, library auto-sync, the animated
//!   background, dynamic artwork colours, the mini-player / overlay windows and UI
//!   animations are suspended;
//! * the window is redrawn only when something changes (engine state, input),
//!   plus a 5 s heartbeat for housekeeping — no periodic progress repaints.
//!
//! Playback, the queue, Similarity Radio (its own thread, fed by engine events),
//! global hotkeys, media keys, listening history and the audio processing keep
//! running unchanged: they never depended on the interface.
//!
//! Nothing is saved: settings stay exactly as they were. Leaving restores the
//! window size/position, the page you were on and the theme; textures and page
//! data reload lazily when they're shown again.

use super::widgets::{self, Transport};
use super::{AquaApp, Page};
use crate::audio::engine::Cmd;
use crate::config::RepeatMode;
use eframe::egui::{self, RichText, Ui};
use std::time::Duration;

/// What Essential Mode restores on exit.
#[derive(Clone, Debug)]
pub struct Saved {
    pub page: Page,
    pub pos: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The size can't be applied while minimized (auto-enter on minimize).
    pub resize_pending: bool,
    /// When to hand freed memory back to the OS: textures are only destroyed by
    /// the renderer after the frame that dropped them, so trimming right away
    /// would find nothing to return.
    pub trim_at: Option<std::time::Instant>,
}

/// Normal-mode minimum window size (see `lib.rs`).
const NORMAL_MIN: [f32; 2] = [360.0, 480.0];
const ESSENTIAL_MIN: [f32; 2] = [280.0, 96.0];

impl AquaApp {
    pub fn essential_active(&self) -> bool {
        self.essential.is_some()
    }

    pub fn toggle_essential(&mut self, ctx: &egui::Context) {
        if self.essential_active() {
            self.exit_essential(ctx);
        } else {
            self.enter_essential(ctx, true);
        }
    }

    /// `resize = false` when entering from a minimized window (applied on restore).
    pub fn enter_essential(&mut self, ctx: &egui::Context, resize: bool) {
        if self.essential_active() {
            return;
        }
        let (pos, size, maximized, fullscreen) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect.map(|r| [r.min.x, r.min.y]), v.inner_rect.map(|r| [r.width(), r.height()]), v.maximized.unwrap_or(false), v.fullscreen.unwrap_or(false))
        });
        self.essential = Some(Saved { page: self.page.clone(), pos, size, maximized, fullscreen, resize_pending: !resize, trim_at: Some(std::time::Instant::now() + Duration::from_millis(1500)) });
        if resize {
            self.apply_essential_window(ctx);
        }
        if self.cfg.essential.always_on_top {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Aqua Glass — Essential".into()));
        // no UI transitions while in Essential Mode (runtime only; theme re-applied on exit)
        ctx.all_styles_mut(|s| s.animation_time = 0.0);
        self.release_for_essential(ctx);
        log::info!("essential mode: on");
        ctx.request_repaint();
    }

    fn apply_essential_window(&mut self, ctx: &egui::Context) {
        if let Some(s) = &self.essential {
            if s.fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            if s.maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(ESSENTIAL_MIN[0], ESSENTIAL_MIN[1])));
        let sz = if self.cfg.essential.remember_size { self.cfg.essential.size } else { crate::config::EssentialCfg::default().size };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(sz[0], sz[1])));
    }

    /// Release everything the essential view doesn't need. All of it is either
    /// on disk or re-fetched lazily when a page needs it again.
    fn release_for_essential(&mut self, ctx: &egui::Context) {
        // GPU textures + decoded images of cover art (disk cache stays)
        self.artwork.lock().unwrap().clear();
        // background image textures
        self.bg.release();
        // visualizer: stop the audio tap and drop the analyzer
        self.out.set_vis(false);
        self.analyzer = None;
        // page data of inactive pages (metadata; reloaded on demand)
        self.view.live.clear();
        self.view.artist_pages.clear();
        self.view.artist_extra.clear();
        self.view.local_artists.clear();
        self.view.loading.clear();
        self.search.results.clear();
        // lyrics: keep only the current song's (tiny), drop the rest
        let cur = self.lyrics.key.clone();
        self.lyrics.cache.retain(|k, _| *k == cur);
        // playlist track lists held in memory (they're on disk)
        self.library.lock().unwrap().evict_except(None);
        self.accent_override = None;
        ctx.forget_all_images();
    }

    /// Deferred part of entering: return freed memory once the renderer has
    /// actually destroyed the released textures (a few frames later).
    pub fn essential_housekeeping(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.essential else { return };
        if let Some(t) = s.trim_at {
            let now = std::time::Instant::now();
            if now >= t {
                s.trim_at = None;
                crate::util::release_free_memory();
            } else {
                ctx.request_repaint_after(t - now);
            }
        }
    }

    pub fn exit_essential(&mut self, ctx: &egui::Context) {
        let Some(s) = self.essential.take() else { return };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(NORMAL_MIN[0], NORMAL_MIN[1])));
        if self.cfg.essential.restore_layout {
            if let Some([w, h]) = s.size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w.max(NORMAL_MIN[0]), h.max(NORMAL_MIN[1]))));
            }
            if let Some([x, y]) = s.pos {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
            }
            if s.maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            if s.fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
        } else {
            let [w, h] = ctx.input(|i| i.viewport().inner_rect.map(|r| [r.width(), r.height()])).unwrap_or(NORMAL_MIN);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w.max(NORMAL_MIN[0]), h.max(NORMAL_MIN[1]))));
        }
        if self.cfg.essential.always_on_top {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Aqua Glass".into()));
        self.page = s.page;
        // theme (incl. animation timing) and dynamic colours are re-applied next frame
        self.bg.theme_dirty = true;
        if self.page == Page::Search && !self.search.query.trim().is_empty() {
            self.run_search();
        }
        self.last_input = std::time::Instant::now();
        log::info!("essential mode: off");
        ctx.request_repaint();
    }

    /// Per-frame checks for the automatic entry options (both off by default).
    pub fn essential_auto(&mut self, ctx: &egui::Context, minimized: bool) {
        let e = &self.cfg.essential;
        if let Some(s) = &mut self.essential {
            if s.resize_pending && !minimized {
                s.resize_pending = false;
                self.apply_essential_window(ctx);
            }
            return;
        }
        if minimized && e.auto_minimized {
            self.enter_essential(ctx, false);
            return;
        }
        if e.auto_idle && !minimized && !self.main_hidden {
            let limit = Duration::from_secs(e.idle_minutes.max(1) as u64 * 60);
            let idle = self.last_input.elapsed();
            if idle >= limit {
                self.enter_essential(ctx, true);
            } else {
                // wake up when the timer runs out even if nothing else repaints
                ctx.request_repaint_after(limit - idle);
            }
        }
    }
}

/// The Essential Mode window.
pub fn show(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let v = ui.visuals().clone();
    let accent = app.accent();
    // solid background from the theme: no image, blur, animation or gradient
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0, v.panel_fill.to_opaque());
    let wide = rect.width() >= 600.0;
    // centre the content vertically in whatever size the window has
    let content_h = if wide { 44.0 } else { 86.0 };
    let top = ((rect.height() - content_h) / 2.0 - 4.0).max(6.0);
    let frame = egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 12, top: 0, bottom: 6 });
    frame.show(ui, |ui| {
        ui.add_space(top);
        let title = st.current.as_ref().map(|t| t.title.clone()).unwrap_or_else(|| "Nothing playing".into());
        let mut exit = false;
        let controls = |app: &mut AquaApp, ui: &mut Ui| {
            if widgets::transport_button(ui, Transport::Shuffle, 15.0, if st.shuffle { accent } else { v.text_color() }, "Shuffle") {
                app.engine.send(Cmd::SetShuffle(!st.shuffle));
                app.cfg.playback.shuffle = !st.shuffle;
                app.mark_dirty();
            }
            if widgets::transport_button(ui, Transport::Prev, 14.0, v.strong_text_color(), "Previous") {
                app.engine.send(Cmd::Prev);
            }
            if widgets::play_button(ui, st.playing, accent, v.extreme_bg_color, "Play / Pause") {
                app.engine.send(Cmd::Toggle);
            }
            if widgets::transport_button(ui, Transport::Next, 14.0, v.strong_text_color(), "Next") {
                app.engine.send(Cmd::Next);
            }
            let (ri, rc) = match st.repeat {
                RepeatMode::Off => (Transport::Repeat, v.text_color()),
                RepeatMode::All => (Transport::Repeat, accent),
                RepeatMode::One => (Transport::RepeatOne, accent),
            };
            if widgets::transport_button(ui, ri, 15.0, rc, "Repeat: off / all / one") {
                let r = match st.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                app.engine.send(Cmd::SetRepeat(r));
                app.cfg.playback.repeat = r;
                app.mark_dirty();
            }
        };
        let tip = match app.cfg.shortcuts.get("toggle_essential").filter(|s| !s.is_empty()) {
            Some(sc) => format!("Leave Essential Mode ({sc})"),
            None => "Leave Essential Mode".to_string(),
        };
        let exit_btn = |ui: &mut Ui| -> bool { widgets::expand_button(ui, v.weak_text_color(), &tip) };
        if wide {
            // one row: title | controls | volume | exit
            ui.horizontal_centered(|ui| {
                let title_w = (ui.available_width() - 5.0 * 36.0 - 40.0 - 200.0 - 40.0).max(80.0);
                ui.allocate_ui_with_layout(egui::vec2(title_w, 40.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&title).size(15.0).strong()).truncate());
                });
                controls(app, ui);
                ui.add_space(8.0);
                if let Some(ch) = widgets::volume_control(ui, st.volume, st.muted, 130.0) {
                    app.apply_volume(ch);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| exit = exit_btn(ui));
            });
        } else {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&title).size(15.0).strong()).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| exit = exit_btn(ui));
            });
            ui.horizontal(|ui| {
                let row = 4.0 * (32.0 + ui.spacing().item_spacing.x) + 40.0;
                let vol = (ui.available_width() - row - 28.0 - 24.0).clamp(60.0, 220.0);
                controls(app, ui);
                ui.add_space(6.0);
                if let Some(ch) = widgets::volume_control(ui, st.volume, st.muted, vol) {
                    app.apply_volume(ch);
                }
            });
        }
        if exit {
            let ctx = ui.ctx().clone();
            app.exit_essential(&ctx);
        }
    });
}
