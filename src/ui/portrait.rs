//! Adaptive Portrait Mode: a single-column layout for narrow, tall windows.
//!
//! It is only a different *layout* over the same `AquaApp`: the playback engine,
//! music providers, library, radio and artwork cache are shared with landscape;
//! nothing is duplicated when switching.
//!
//! Memory rules:
//! * only what is on screen is built: sidebar, right panel, visualizer and the
//!   landscape player bar don't exist in portrait;
//! * only the active Queue / Lyrics / Discover tab is rendered (its state — scroll
//!   position, lyrics, radio settings — lives in shared app state, not in widgets);
//! * the queue is virtualised (`show_rows`): rows and their artwork are only
//!   requested while visible, and artwork loads lazily through the shared cache;
//! * on every layout switch, cover textures not shown in the last second are
//!   released (`Artwork::evict_to(1)`), so landscape-only art doesn't linger;
//! * no visualizer in portrait, so the audio tap is off (`vis_wanted` stays false).

use super::widgets::{self, Transport};
use super::{AquaApp, Page};
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::{LayoutMode, PortraitTab, ProviderKind, RepeatMode};
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText, Ui};

/// Window widths (logical px) where Auto switches. Landscape needs ~720 px (its old
/// minimum window width); the gap avoids flicker while resizing.
pub const ENTER_PORTRAIT_W: f32 = 720.0;
pub const LEAVE_PORTRAIT_W: f32 = 800.0;
/// Tall windows (e.g. maximized on a rotated 1080×1920 monitor) also get portrait:
/// height ≥ 1.4 × width switches, and it stays until height < 1.25 × width.
pub const ENTER_PORTRAIT_RATIO: f32 = 1.4;
pub const LEAVE_PORTRAIT_RATIO: f32 = 1.25;
/// Size used when the user forces portrait from a landscape window.
pub const PORTRAIT_WINDOW: [f32; 2] = [400.0, 860.0];

/// Decide the layout for this frame. Auto: portrait when the window is too narrow
/// for the sidebar layout, or clearly taller than wide (vertical monitor); both
/// with hysteresis so resizing near a threshold doesn't flicker.
pub fn wants_portrait(mode: LayoutMode, width: f32, height: f32, was_portrait: bool) -> bool {
    match mode {
        LayoutMode::Landscape => false,
        LayoutMode::Portrait => true,
        LayoutMode::Auto => {
            let (w_limit, ratio) = if was_portrait { (LEAVE_PORTRAIT_W, LEAVE_PORTRAIT_RATIO) } else { (ENTER_PORTRAIT_W, ENTER_PORTRAIT_RATIO) };
            width < w_limit || height >= width.max(1.0) * ratio
        }
    }
}

pub fn shell(app: &mut AquaApp, ui: &mut Ui) {
    let v = ui.visuals().clone();
    let border = v.widgets.noninteractive.bg_stroke.color;
    let fill = v.panel_fill;
    // bottom navigation
    egui::Panel::bottom("portrait_nav")
        .exact_size(58.0)
        .frame(egui::Frame::new().fill(fill).stroke(egui::Stroke::new(1.0, border)).inner_margin(egui::Margin::symmetric(8, 6)))
        .show(ui, |ui| bottom_nav(app, ui));
    // mini player above the navigation everywhere except the Playing screen
    if app.page != Page::NowPlaying && app.state.current.is_some() {
        egui::Panel::bottom("portrait_mini")
            .exact_size(62.0)
            .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ui, |ui| mini_player(app, ui));
    }
    let central_fill = if app.bg.active { Color32::TRANSPARENT } else { v.extreme_bg_color };
    egui::CentralPanel::default().frame(egui::Frame::new().fill(central_fill).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 6 })).show(ui, |ui| {
        header(app, ui);
        ui.add_space(6.0);
        let page = app.page.clone();
        match page {
            Page::NowPlaying => playing(app, ui),
            Page::Home => super::browse::home(app, ui),
            Page::Search => {
                // portrait has no top bar: the search field sits at the top of the page
                let w = ui.available_width();
                ui.horizontal(|ui| {
                    super::shell::search_pill(app, ui, w, "Songs, artists, albums…");
                    if app.search.pending > 0 {
                        ui.spinner();
                    }
                });
                ui.add_space(8.0);
                super::browse::search(app, ui)
            }
            Page::Discover => super::browse::discover(app, ui),
            Page::Queue => {
                // the queue lives in the Playing screen's Queue tab in portrait
                app.cfg.general.portrait_tab = PortraitTab::Queue;
                app.page = Page::NowPlaying;
                playing(app, ui);
            }
            Page::Library => super::pages::library(app, ui),
            Page::Playlists => super::pages::playlists_page(app, ui),
            Page::Artists => super::pages::artists_page(app, ui),
            Page::Albums => super::pages::albums_page(app, ui),
            Page::Transfer => super::transfer_ui::page(app, ui),
            Page::Settings => super::settings::page(app, ui),
            Page::Playlist(p, id) => super::pages::playlist_view(app, ui, p, &id),
            Page::Album(a) => super::pages::album_view(app, ui, &a),
            Page::Artist(a) => super::pages::artist_view(app, ui, &a),
        }
    });
}

fn header(app: &mut AquaApp, ui: &mut Ui) {
    let accent = app.accent();
    ui.horizontal(|ui| {
        if app.page != Page::NowPlaying && !app.back.is_empty() && ui.add(egui::Button::new("‹").frame(false)).on_hover_text("Back").clicked() {
            app.go_back();
        }
        ui.label(RichText::new("💧").color(accent).size(15.0));
        ui.label(RichText::new("Aqua Glass").size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new(super::widgets::icon::SETTINGS()).size(15.0)).frame(false)).on_hover_text("Settings").clicked() {
                app.navigate(Page::Settings);
            }
            if ui.add(egui::Button::new(RichText::new(super::widgets::icon::SEARCH()).size(14.0)).frame(false)).on_hover_text("Search").clicked() {
                app.navigate(Page::Search);
                app.search.focus = true;
            }
            // compact RAM indicator (opens the Performance page)
            let mb = app.monitor.current_mb();
            let target = app.cfg.perf.ram_target_mb as f32;
            let r = ui.add(egui::Label::new(RichText::new(format!("RAM {mb:.0}/{target:.0} MB")).size(11.5).color(ui.visuals().weak_text_color())).sense(egui::Sense::click()));
            if r.on_hover_text("Private memory of Aqua Glass. Click for the Performance dashboard.").clicked() {
                app.settings.category = crate::features::Category::Performance;
                app.navigate(Page::Settings);
            }
        });
    });
}

/// The Playing screen: large artwork, track, progress, controls, one tab.
fn playing(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let accent = app.accent();
    let v = ui.visuals().clone();
    let w = ui.available_width();
    // artwork: as wide as the column, but leave room for the tab content
    let art = w.min((ui.available_height() * 0.42).max(150.0)).floor();
    ui.vertical_centered(|ui| {
        let tex = st.current.as_ref().and_then(|t| app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover));
        widgets::artwork(ui, tex.as_ref(), art, 14);
    });
    ui.add_space(10.0);
    match &st.current {
        Some(t) => {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(w - 40.0);
                    let r = ui.add(egui::Label::new(RichText::new(&t.title).size(20.0).strong()).truncate().sense(egui::Sense::click()));
                    if r.on_hover_text(format!("Go to album: {}", t.album)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let t = t.clone();
                        app.open_album_of(&t);
                    }
                    let mut go = None;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (i, a) in t.artists.iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new(", ").size(14.0).color(v.weak_text_color()));
                            }
                            let r = ui.add(egui::Label::new(RichText::new(a).size(14.0).color(v.weak_text_color())).truncate().sense(egui::Sense::click()));
                            if r.on_hover_text(format!("Go to artist: {a}")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                go = Some(i);
                            }
                        }
                    });
                    if let Some(i) = go {
                        let t = t.clone();
                        app.open_artist_of(&t, i);
                    }
                });
                if t.provider != ProviderKind::Local {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let liked = app.is_liked(t);
                        if ui.add(egui::Button::new(RichText::new(if liked { "♥" } else { "♡" }).size(20.0).color(if liked { accent } else { v.text_color() })).frame(false)).on_hover_text("Like").clicked() && !liked {
                            let t = t.clone();
                            app.like(&t);
                        }
                    });
                }
            });
        }
        None => {
            ui.label(RichText::new("Nothing playing").size(20.0).strong());
            ui.label(RichText::new("Pick something from Home, Search or Library.").color(v.weak_text_color()));
        }
    }
    ui.add_space(6.0);
    // progress
    let pos = app.engine.position_ms();
    let dur = st.duration_ms;
    let mut p = pos as f32;
    ui.spacing_mut().slider_width = w;
    let r = ui.add_enabled(dur > 0, egui::Slider::new(&mut p, 0.0..=(dur.max(1) as f32)).show_value(false).trailing_fill(true));
    if r.drag_stopped() || (r.clicked() && !r.dragged()) {
        app.engine.send(Cmd::Seek(p as u64));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(fmt_ms(pos)).size(11.5).color(v.weak_text_color()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(fmt_ms(dur)).size(11.5).color(v.weak_text_color()));
        });
    });
    if let Some(e) = &st.error {
        ui.add(egui::Label::new(RichText::new(e).size(11.5).color(Color32::from_rgb(248, 113, 113))).wrap());
    }
    // transport (same vector controls as landscape)
    ui.horizontal(|ui| {
        let row = 4.0 * (32.0 + ui.spacing().item_spacing.x) + 40.0 + 2.0 * 18.0;
        ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
        if widgets::transport_button(ui, Transport::Shuffle, 16.0, if st.shuffle { accent } else { v.text_color() }, "Shuffle") {
            app.engine.send(Cmd::SetShuffle(!st.shuffle));
            app.cfg.playback.shuffle = !st.shuffle;
            app.mark_dirty();
        }
        ui.add_space(18.0);
        if widgets::transport_button(ui, Transport::Prev, 15.0, v.strong_text_color(), "Previous") {
            app.engine.send(Cmd::Prev);
        }
        if widgets::play_button(ui, st.playing, accent, v.extreme_bg_color, "Play / Pause") {
            app.engine.send(Cmd::Toggle);
        }
        if widgets::transport_button(ui, Transport::Next, 15.0, v.strong_text_color(), "Next") {
            app.engine.send(Cmd::Next);
        }
        ui.add_space(18.0);
        let (ri, rc) = match st.repeat {
            RepeatMode::Off => (Transport::Repeat, v.text_color()),
            RepeatMode::All => (Transport::Repeat, accent),
            RepeatMode::One => (Transport::RepeatOne, accent),
        };
        if widgets::transport_button(ui, ri, 16.0, rc, "Repeat: off / all / one") {
            let r = match st.repeat {
                RepeatMode::Off => RepeatMode::All,
                RepeatMode::All => RepeatMode::One,
                RepeatMode::One => RepeatMode::Off,
            };
            app.engine.send(Cmd::SetRepeat(r));
            app.cfg.playback.repeat = r;
            app.mark_dirty();
        }
    });
    // volume: always visible under the transport, centred, scales with the column
    ui.horizontal(|ui| {
        let sw = (w * 0.55).clamp(120.0, 360.0);
        let row = 28.0 + 6.0 + sw;
        ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
        if let Some(ch) = widgets::volume_control(ui, st.volume, st.muted, sw) {
            app.apply_volume(ch);
        }
    });
    ui.add_space(6.0);
    // tabs: only the active one is built
    let mut tab = app.cfg.general.portrait_tab;
    tabs(ui, &mut tab, accent);
    if tab != app.cfg.general.portrait_tab {
        app.cfg.general.portrait_tab = tab;
        app.mark_dirty();
    }
    ui.add_space(4.0);
    match tab {
        PortraitTab::Queue => queue_tab(app, ui),
        PortraitTab::Lyrics => {
            if app.cfg.active("lyrics") {
                super::pages::lyrics_panel(app, ui, false);
            } else {
                ui.label(RichText::new("Lyrics are off — turn them on in Settings › Lyrics.").color(v.weak_text_color()));
            }
        }
        PortraitTab::Discover => discover_tab(app, ui),
    }
}

fn tabs(ui: &mut Ui, cur: &mut PortraitTab, accent: Color32) {
    let items = [(PortraitTab::Queue, "Queue"), (PortraitTab::Lyrics, "Lyrics"), (PortraitTab::Discover, "Discover")];
    let w = ui.available_width() / items.len() as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (t, label) in items {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 32.0), egui::Sense::click());
            let sel = *cur == t;
            let col = if sel { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(14.0), col);
            if sel {
                let y = rect.bottom() - 2.0;
                ui.painter().hline(egui::Rangef::new(rect.center().x - 18.0, rect.center().x + 18.0), y, egui::Stroke::new(2.0, accent));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *cur = t;
            }
        }
    });
}

/// Up next, virtualised: only visible rows are laid out and request artwork.
fn queue_tab(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let accent = app.accent();
    let start = st.queue_pos.map(|p| p + 1).unwrap_or(0).min(st.queue.len());
    let upcoming: Vec<usize> = (start..st.queue.len()).collect();
    let radio = st.queue_origin.iter().skip(start).filter(|o| **o == aqua_core::queue::EntryOrigin::Radio).count();
    ui.horizontal(|ui| {
        if app.radio.is_some() && app.radio_status().is_some_and(|s| s.active) {
            ui.label(RichText::new("📡 Similarity Radio").strong().color(accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Active").size(11.5).color(accent));
            });
        } else {
            ui.label(RichText::new(format!("Up next ({})", upcoming.len())).strong());
        }
    });
    if upcoming.is_empty() {
        ui.label(RichText::new("Nothing queued. Play a song from search to start Similarity Radio.").size(12.5).color(ui.visuals().weak_text_color()));
        return;
    }
    let mut cmd = None;
    egui::ScrollArea::vertical().id_salt("portrait_queue").auto_shrink([false, false]).max_height(ui.available_height() - 22.0).show_rows(ui, 56.0, upcoming.len(), |ui, range| {
        for k in range {
            let i = upcoming[k];
            let t = &st.queue[i];
            super::shell::queue_row(app, ui, t, i, st.is_radio(i), &mut cmd);
        }
    });
    if radio > 0 {
        ui.label(RichText::new(format!("{radio} recommendations queued · refills automatically")).size(11.5).color(ui.visuals().weak_text_color()));
    }
    if let Some(c) = cmd {
        app.engine.send(c);
    }
}

/// Compact Discover: Similarity Radio controls and discovery strength.
fn discover_tab(app: &mut AquaApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("portrait_discover").auto_shrink([false, false]).show(ui, |ui| {
        super::shell::radio_controls(app, ui);
        if app.radio.is_some() {
            ui.add_space(6.0);
            ui.label(RichText::new("Discovery strength").strong());
            ui.horizontal_wrapped(|ui| {
                for s in crate::radio::Strength::ALL {
                    let sel = app.cfg.radio.strength == s;
                    if ui.add(egui::Button::new(s.label()).selected(sel).corner_radius(12)).on_hover_text(s.describe()).clicked() {
                        app.cfg.radio.strength = s;
                        app.mark_dirty();
                    }
                }
            });
            ui.label(RichText::new(app.cfg.radio.strength.describe()).size(12.0).color(ui.visuals().weak_text_color()));
        }
        ui.add_space(8.0);
        if ui.button("Open the full Discover page").clicked() {
            app.navigate(Page::Discover);
        }
    });
}

fn mini_player(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let accent = app.accent();
    let v = ui.visuals().clone();
    let Some(t) = st.current.clone() else { return };
    ui.horizontal_centered(|ui| {
        let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Thumb);
        if widgets::artwork(ui, tex.as_ref(), 46.0, 6).on_hover_text("Now Playing").clicked() {
            app.navigate(Page::NowPlaying);
        }
        let r = ui
            .vertical(|ui| {
                ui.set_max_width((ui.available_width() - 90.0).max(60.0));
                ui.add(egui::Label::new(RichText::new(&t.title).size(14.0).strong()).truncate());
                ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.0).color(v.weak_text_color())).truncate());
            })
            .response
            .interact(egui::Sense::click());
        if r.clicked() {
            app.navigate(Page::NowPlaying);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::transport_button(ui, Transport::Next, 14.0, v.strong_text_color(), "Next") {
                app.engine.send(Cmd::Next);
            }
            if widgets::play_button(ui, st.playing, accent, v.extreme_bg_color, "Play / Pause") {
                app.engine.send(Cmd::Toggle);
            }
        });
    });
}

fn bottom_nav(app: &mut AquaApp, ui: &mut Ui) {
    use super::widgets::icon;
    let items: [(&str, &str, Page); 4] = [(icon::NOW(), "Playing", Page::NowPlaying), (icon::HOME(), "Home", Page::Home), (icon::SEARCH(), "Search", Page::Search), (icon::LIBRARY(), "Library", Page::Library)];
    let accent = app.accent();
    let w = ui.available_width() / items.len() as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (ic, label, page) in items {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, ui.available_height()), egui::Sense::click());
            let active = app.page == page
                || matches!((&app.page, &page), (Page::Playlist(..) | Page::Album(_) | Page::Artist(_) | Page::Playlists | Page::Albums | Page::Artists, Page::Library));
            let col = if active { accent } else { ui.visuals().weak_text_color() };
            let p = ui.painter();
            p.text(rect.center() - egui::vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, ic, egui::FontId::proportional(17.0), col);
            p.text(rect.center() + egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.5), col);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if page == Page::Search {
                    app.search.focus = true; // ready to type
                }
                app.navigate(page);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_layout_has_hysteresis() {
        assert!(wants_portrait(LayoutMode::Auto, 400.0, 860.0, false));
        assert!(!wants_portrait(LayoutMode::Auto, 1240.0, 780.0, false));
        // too narrow for the sidebar layout (its old minimum was 720 px)
        assert!(wants_portrait(LayoutMode::Auto, 600.0, 860.0, false));
        assert!(!wants_portrait(LayoutMode::Auto, 760.0, 860.0, false));
        // once portrait, widening a little doesn't flip back (no flicker while resizing)
        assert!(wants_portrait(LayoutMode::Auto, 760.0, 860.0, true));
        assert!(!wants_portrait(LayoutMode::Auto, 820.0, 860.0, true));
        // tall windows: maximized on a rotated 1080×1920 monitor (taskbar + title bar)
        assert!(wants_portrait(LayoutMode::Auto, 1080.0, 1840.0, false));
        assert!(wants_portrait(LayoutMode::Auto, 1080.0, 1920.0, false));
        // half of that monitor, or a tall-ish window on a normal monitor: landscape
        assert!(!wants_portrait(LayoutMode::Auto, 1080.0, 920.0, false));
        assert!(!wants_portrait(LayoutMode::Auto, 1000.0, 1200.0, false));
        // ratio hysteresis: 1.3 stays portrait once in it, doesn't enter it
        assert!(!wants_portrait(LayoutMode::Auto, 1000.0, 1300.0, false));
        assert!(wants_portrait(LayoutMode::Auto, 1000.0, 1300.0, true));
        assert!(!wants_portrait(LayoutMode::Auto, 1000.0, 1200.0, true));
        assert!(wants_portrait(LayoutMode::Portrait, 1600.0, 900.0, false));
        assert!(!wants_portrait(LayoutMode::Landscape, 380.0, 900.0, true));
    }
}
