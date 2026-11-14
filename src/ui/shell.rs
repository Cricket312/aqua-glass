//! Application frame (styled after the Aqua Glass concept): collapsible sidebar
//! with pill navigation and a RAM card, top bar with back/forward and a centred
//! search pill, persistent bottom player, and the optional right panel
//! (Queue / Similarity Radio / Lyrics).

use super::widgets::{self, icon};
use super::{AquaApp, Filter, Page, PlayFrom};
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::{PlayerBarLayout, ProviderKind, RepeatMode, RightPanel};
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText, Ui};

pub fn shell(app: &mut AquaApp, ui: &mut Ui) {
    let theme = app.current_theme().clone();
    let custom = app.cfg.active("custom_theme");
    // panels are slightly translucent over the (optional) static gradient
    let a = if app.cfg.active("translucent_panels") || app.cfg.active("window_backdrop") { theme.panel_alpha } else if app.cfg.active("bg_gradient") { 0.9 } else { 1.0 };
    let sidebar_fill = if custom { crate::theme::ca(theme.sidebar, a) } else { ui.visuals().panel_fill };
    let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let bar_h = match app.cfg.appearance.player_bar_layout {
        PlayerBarLayout::Compact => 64.0,
        _ => app.cfg.appearance.player_bar_height.clamp(70.0, 110.0).max(84.0),
    };
    egui::Panel::bottom("player_bar")
        .exact_size(bar_h)
        .frame(egui::Frame::new().fill(sidebar_fill).stroke(egui::Stroke::new(1.0, border)).inner_margin(egui::Margin::symmetric(18, 8)))
        .show(ui, |ui| player_bar(app, ui));
    let collapsed = app.cfg.general.sidebar_collapsed;
    egui::Panel::left("sidebar")
        .resizable(!collapsed)
        .exact_size(if collapsed { 68.0 } else { app.cfg.general.sidebar_width.max(200.0) })
        .frame(egui::Frame::new().fill(sidebar_fill).stroke(egui::Stroke::new(1.0, border.gamma_multiply(0.6))).inner_margin(egui::Margin::symmetric(if collapsed { 10 } else { 14 }, 14)))
        .show(ui, |ui| {
            if !collapsed {
                let w = ui.available_width() + 28.0;
                if (w - app.cfg.general.sidebar_width).abs() > 2.0 && ui.input(|i| !i.pointer.any_down()) {
                    app.cfg.general.sidebar_width = w.clamp(200.0, 360.0);
                    app.mark_dirty();
                }
            }
            sidebar(app, ui, collapsed)
        });
    if app.cfg.general.right_panel != RightPanel::Closed {
        egui::Panel::right("right_panel")
            .resizable(true)
            .default_size(340.0)
            .size_range(280.0..=480.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 4, right: 14, top: 14, bottom: 12 }))
            .show(ui, |ui| {
                let fill = if custom { crate::theme::ca(theme.panel, a) } else { ui.visuals().faint_bg_color };
                egui::Frame::new().fill(fill).stroke(egui::Stroke::new(1.0, border)).corner_radius(14).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                    ui.set_min_height(ui.available_height());
                    right_panel(app, ui)
                });
            });
    }
    let central_fill = if app.bg.active { Color32::TRANSPARENT } else { ui.visuals().extreme_bg_color };
    egui::CentralPanel::default().frame(egui::Frame::new().fill(central_fill)).show(ui, |ui| {
        egui::Panel::top("top_bar").exact_size(60.0).show_separator_line(false).frame(egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 12, bottom: 6 })).show(ui, |ui| top_bar(app, ui));
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 24, top: 2, bottom: 8 })).show(ui, |ui| {
            let page = app.page.clone();
            match page {
                Page::Home => super::browse::home(app, ui),
                Page::Search => super::browse::search(app, ui),
                Page::Discover => super::browse::discover(app, ui),
                Page::NowPlaying => super::browse::now_playing(app, ui),
                Page::Queue => super::browse::queue_page(app, ui),
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
    });
}

// ---------------------------------------------------------------------------
// Sidebar

fn nav_item(app: &mut AquaApp, ui: &mut Ui, ic: &str, label: &str, page: Page, collapsed: bool) {
    let active = app.page == page || matches!((&app.page, &page), (Page::Playlist(..), Page::Playlists) | (Page::Album(_), Page::Albums) | (Page::Artist(_), Page::Artists));
    let h = 42.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
    let v = ui.visuals().clone();
    let accent = app.accent();
    if active {
        // seafoam-tinted pill, as in the concept
        ui.painter().rect(rect, 10, accent.gamma_multiply(0.16), egui::Stroke::new(1.0, accent.gamma_multiply(0.38)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 10, v.widgets.inactive.bg_fill.gamma_multiply(0.7));
    }
    let col = if active || resp.hovered() { v.strong_text_color() } else { v.text_color().gamma_multiply(0.85) };
    if collapsed {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ic, egui::FontId::proportional(18.0), col);
    } else {
        ui.painter().text(rect.left_center() + egui::vec2(16.0, 0.0), egui::Align2::LEFT_CENTER, ic, egui::FontId::proportional(17.0), col);
        ui.painter().text(rect.left_center() + egui::vec2(48.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(15.0), col);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if collapsed { resp.on_hover_text(label) } else { resp };
    if resp.clicked() {
        app.navigate(page);
    }
    ui.add_space(2.0);
}

fn sidebar(app: &mut AquaApp, ui: &mut Ui, collapsed: bool) {
    // understated branding: small droplet + name, collapse control
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        if !collapsed || app.cfg.active("sidebar_branding") {
            ui.label(RichText::new("💧").size(15.0).color(app.accent()));
        }
        if !collapsed {
            ui.label(RichText::new("Aqua Glass").size(14.0).color(ui.visuals().text_color()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let t = if collapsed { "»" } else { "«" };
            if ui.add(egui::Button::new(RichText::new(t).size(14.0).color(ui.visuals().weak_text_color())).frame(false)).on_hover_text(if collapsed { "Expand sidebar" } else { "Collapse sidebar" }).clicked() {
                app.cfg.general.sidebar_collapsed = !collapsed;
                app.mark_dirty();
            }
            if !collapsed {
                app_menu(app, ui);
            }
        });
    });
    ui.add_space(16.0);
    nav_item(app, ui, icon::HOME(), "Home", Page::Home, collapsed);
    nav_item(app, ui, icon::SEARCH(), "Search", Page::Search, collapsed);
    nav_item(app, ui, icon::LIBRARY(), "Library", Page::Library, collapsed);
    nav_item(app, ui, icon::PLAYLIST(), "Playlists", Page::Playlists, collapsed);
    nav_item(app, ui, icon::ARTIST(), "Artists", Page::Artists, collapsed);
    nav_item(app, ui, icon::ALBUM(), "Albums", Page::Albums, collapsed);
    nav_item(app, ui, "📶", "Discover", Page::Discover, collapsed);
    nav_item(app, ui, icon::QUEUE(), "Queue", Page::Queue, collapsed);
    nav_item(app, ui, icon::TRANSFER(), "Playlist Transfer", Page::Transfer, collapsed);
    ui.add_space(8.0);
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
    ui.add_space(10.0);
    nav_item(app, ui, icon::SETTINGS(), "Settings", Page::Settings, collapsed);

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ram_card(app, ui, collapsed);
        ui.add_space(8.0);
        if let Some(left) = crate::services::ratelimit::status("spotify.com").cooldown_left {
            let txt = if collapsed { format!("⏸ {}s", left.as_secs().max(1)) } else { format!("⏸ Spotify paused {} s (rate limit)", left.as_secs().max(1)) };
            let r = ui.add(egui::Label::new(RichText::new(txt).size(11.5).color(widgets::impact_color(crate::features::Impact::High))).sense(egui::Sense::click()));
            if r.on_hover_text("Spotify asked Aqua Glass to slow down. All Spotify requests wait until the pause ends; playback isn't affected. Click for details.").clicked() {
                app.settings.category = crate::features::Category::Services;
                app.navigate(Page::Settings);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            ui.add_space(4.0);
        }
        let jobs: Vec<super::jobs::JobInfo> = app.jobs.active.lock().unwrap().values().cloned().collect();
        if let Some(j) = jobs.first() {
            ui.horizontal(|ui| {
                ui.spinner();
                if !collapsed {
                    let p = j.progress.filter(|(_, t)| *t > 0).map(|(d, t)| format!(" {d}/{t}")).unwrap_or_default();
                    let more = if jobs.len() > 1 { format!(" +{}", jobs.len() - 1) } else { String::new() };
                    ui.add(egui::Label::new(RichText::new(format!("{}{p}{more}", j.label)).small().weak()).truncate());
                }
            });
        }
        if !collapsed && app.cfg.active("sidebar_playlists") {
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label(RichText::new("YOUR PLAYLISTS").size(10.5).color(ui.visuals().weak_text_color()));
                let lists = super::pages::all_playlists(app);
                let mut open = None;
                egui::ScrollArea::vertical().id_salt("side_pl").auto_shrink([false, false]).show_rows(ui, 24.0, lists.len(), |ui, range| {
                    for p in &lists[range] {
                        let sel = app.page == Page::Playlist(p.provider, p.id.clone());
                        let txt = RichText::new(&p.name).size(13.0).color(if sel { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() });
                        if ui.add(egui::Label::new(txt).truncate().sense(egui::Sense::click())).on_hover_text(format!("{} · {} tracks", p.provider.label(), p.track_count)).clicked() {
                            open = Some(Page::Playlist(p.provider, p.id.clone()));
                        }
                    }
                });
                if let Some(p) = open {
                    app.navigate(p);
                }
            });
        }
    });
}

fn app_menu(app: &mut AquaApp, ui: &mut Ui) {
    ui.menu_button(RichText::new("…").size(14.0).color(ui.visuals().weak_text_color()), |ui| {
        if ui.button("Settings   (Ctrl+,)").clicked() {
            app.navigate(Page::Settings);
            ui.close();
        }
        if ui.button("Now Playing").clicked() {
            app.navigate(Page::NowPlaying);
            ui.close();
        }
        if app.cfg.active("mini_player") && ui.button("Mini-player").clicked() {
            app.do_action(crate::shortcuts::Action::ToggleMiniPlayer, &ui.ctx().clone());
            ui.close();
        }
        if ui.button("Sync library now").clicked() {
            app.sync_now(ProviderKind::Spotify, false);
            app.sync_now(ProviderKind::YouTube, false);
            ui.close();
        }
        ui.separator();
        if ui.button("Disable all visual customizations").clicked() {
            app.cfg.disable_all_visual();
            app.mark_dirty();
            ui.close();
        }
        if ui.button("Quit").clicked() {
            let _ = app.ui_tx.send(crate::winint::UiCmd::Quit);
            ui.close();
        }
    })
    .response
    .on_hover_text("Menu");
}

/// Compact, always-visible RAM card (opens the Performance dashboard).
fn ram_card(app: &mut AquaApp, ui: &mut Ui, collapsed: bool) {
    let mb = app.monitor.current_mb();
    let target = app.cfg.perf.ram_target_mb as f32;
    let (state, col) = match app.pressure {
        crate::perf::Pressure::Normal => ("", app.accent()),
        crate::perf::Pressure::Elevated => ("Saving memory", widgets::impact_color(crate::features::Impact::High)),
        crate::perf::Pressure::Over => ("Over target", widgets::impact_color(crate::features::Impact::VeryHigh)),
    };
    let v = ui.visuals().clone();
    let w = ui.available_width();
    let h = if collapsed { 44.0 } else { 66.0 };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() { v.widgets.inactive.bg_fill } else { v.faint_bg_color };
    p.rect(rect, 12, fill, egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    let inner = rect.shrink2(egui::vec2(if collapsed { 8.0 } else { 14.0 }, 10.0));
    if collapsed {
        p.text(inner.center_top(), egui::Align2::CENTER_TOP, format!("{mb:.0}"), egui::FontId::proportional(12.0), v.text_color());
    } else {
        p.text(inner.left_top(), egui::Align2::LEFT_TOP, "▣", egui::FontId::proportional(15.0), v.text_color());
        p.text(inner.left_top() + egui::vec2(24.0, 0.0), egui::Align2::LEFT_TOP, "RAM Usage", egui::FontId::proportional(13.5), v.strong_text_color());
        let sub = if state.is_empty() { format!("{mb:.0} MB / {target:.0} MB") } else { format!("{mb:.0} / {target:.0} MB · {state}") };
        p.text(inner.left_top() + egui::vec2(24.0, 19.0), egui::Align2::LEFT_TOP, sub, egui::FontId::proportional(12.0), if state.is_empty() { v.weak_text_color() } else { col });
        p.text(inner.right_top() + egui::vec2(0.0, 6.0), egui::Align2::RIGHT_TOP, "›", egui::FontId::proportional(16.0), v.weak_text_color());
    }
    let bar = egui::Rect::from_min_size(egui::pos2(inner.left(), inner.bottom() - 6.0), egui::vec2(inner.width(), 6.0));
    p.rect_filled(bar, 3, v.widgets.inactive.bg_fill);
    let mut f = bar;
    f.set_width(inner.width() * (mb / target.max(1.0)).clamp(0.02, 1.0));
    p.rect_filled(f, 3, col);
    if resp
        .on_hover_text(format!("Private memory {mb:.0} MB of the {target:.0} MB target. Click for the Performance dashboard."))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.settings.category = crate::features::Category::Performance;
        app.navigate(Page::Settings);
    }
}

// ---------------------------------------------------------------------------
// Top bar

fn circle_button(ui: &mut Ui, text: &str, enabled: bool, tip: &str) -> bool {
    let fill = ui.visuals().faint_bg_color;
    let b = egui::Button::new(RichText::new(text).size(17.0)).fill(fill).corner_radius(18).min_size(egui::vec2(36.0, 36.0));
    ui.add_enabled(enabled, b).on_hover_text(tip).clicked()
}

fn top_bar(app: &mut AquaApp, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        if circle_button(ui, "‹", !app.back.is_empty(), "Back") {
            app.go_back();
        }
        if circle_button(ui, "›", !app.forward.is_empty(), "Forward") {
            app.go_forward();
        }
        // centred search pill with a filter button at its right end
        let avail = ui.available_width();
        let w = (avail - 40.0).clamp(240.0, 600.0);
        ui.add_space(((avail - w) / 2.0 - 40.0).max(12.0));
        let v = ui.visuals().clone();
        let mut submit = false;
        egui::Frame::new().fill(v.faint_bg_color).stroke(egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color)).corner_radius(20).inner_margin(egui::Margin { left: 14, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
            ui.set_width(w - 20.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::SEARCH()).size(15.0).color(v.text_color()));
                let te = ui.add(egui::TextEdit::singleline(&mut app.top_search).frame(egui::Frame::NONE).hint_text("Search for songs, artists, albums, playlists…").desired_width(w - 100.0).vertical_align(egui::Align::Center));
                if app.search.focus {
                    te.request_focus();
                    app.search.focus = false;
                }
                submit = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                filter_menu(app, ui);
            });
        });
        if submit && !app.top_search.trim().is_empty() {
            app.search.query = app.top_search.clone();
            app.navigate(Page::Search);
            app.run_search();
        }
        if app.search.pending > 0 {
            ui.spinner();
        }
    });
}

/// Search scope / service filter + account status, behind the pill's filter button.
fn filter_menu(app: &mut AquaApp, ui: &mut Ui) {
    let active = app.filter != Filter::All;
    let c = if active { app.accent() } else { ui.visuals().text_color() };
    ui.menu_button(RichText::new("⚙").size(14.0).color(c), |ui| {
        ui.label(RichText::new("Search and browse").strong());
        let mut f = app.filter;
        ui.radio_value(&mut f, Filter::All, "All services");
        for p in [ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local] {
            if app.provider_enabled(p) {
                ui.radio_value(&mut f, Filter::Only(p), p.label());
            }
        }
        if f != app.filter {
            app.filter = f;
            if app.page == Page::Search && !app.search.query.is_empty() {
                app.run_search();
            }
        }
        ui.separator();
        ui.label(RichText::new("Accounts").strong());
        if app.cfg.services.spotify_enabled {
            let s = match &app.state.spotify {
                crate::audio::engine::SpotifyStatus::Ready => "Spotify — library + Premium playback ready".to_string(),
                crate::audio::engine::SpotifyStatus::Unavailable(e) => format!("Spotify — playback unavailable: {e}"),
                _ => format!("Spotify — {}", app.sp_status),
            };
            ui.label(RichText::new(s).size(12.0));
        }
        if app.cfg.services.youtube_enabled {
            ui.label(RichText::new(format!("YouTube Music — {}", app.yt_status)).size(12.0));
        }
        if ui.button("Music services settings…").clicked() {
            app.settings.category = crate::features::Category::Services;
            app.navigate(Page::Settings);
            ui.close();
        }
    })
    .response
    .on_hover_text("Filter by service · accounts");
}

// ---------------------------------------------------------------------------
// Bottom player

fn icon_btn(ui: &mut Ui, text: &str, size: f32, color: Color32, tip: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(text).size(size).color(color)).frame(false).min_size(egui::vec2(30.0, 30.0))).on_hover_text(tip).clicked()
}

fn player_bar(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let pos = app.engine.position_ms();
    let dur = st.duration_ms;
    let accent = app.accent();
    let full_w = ui.available_width();
    let side_w = (full_w * 0.27).clamp(220.0, 380.0);
    let center_w = (full_w - side_w * 2.0).max(260.0);
    let h = ui.available_height();
    let v = ui.visuals().clone();
    ui.horizontal(|ui| {
        // ---- left: artwork, title, artist, like
        ui.allocate_ui_with_layout(egui::vec2(side_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let art = (h - 6.0).clamp(40.0, 64.0);
            if let Some(t) = &st.current {
                let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
                if widgets::artwork(ui, tex.as_ref(), art, 6).on_hover_text("Now Playing").clicked() {
                    app.navigate(Page::NowPlaying);
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(((h - 42.0) / 2.0).max(0.0));
                    ui.set_max_width(side_w - art - 70.0);
                    if ui.add(egui::Label::new(RichText::new(&t.title).size(15.0).strong()).truncate().sense(egui::Sense::click())).clicked() {
                        app.navigate(Page::NowPlaying);
                    }
                    ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.5).color(v.weak_text_color())).truncate());
                });
                if t.provider != ProviderKind::Local {
                    let liked = app.is_liked(t);
                    if icon_btn(ui, if liked { "♥" } else { "♡" }, 17.0, if liked { accent } else { v.weak_text_color() }, if liked { "Liked" } else { "Like" }) && !liked {
                        let t = t.clone();
                        app.like(&t);
                    }
                }
            } else {
                widgets::artwork(ui, None, art, 6);
                ui.add_space(8.0);
                ui.label(RichText::new("Nothing playing").color(v.weak_text_color()));
            }
        });
        // ---- centre: controls + progress
        ui.allocate_ui_with_layout(egui::vec2(center_w, h), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(((h - 66.0) / 2.0).max(0.0));
            ui.horizontal(|ui| {
                let w = 4.0 * 40.0 + 44.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if icon_btn(ui, icon::SHUFFLE(), 15.0, if st.shuffle { accent } else { v.text_color() }, "Shuffle") {
                    app.engine.send(Cmd::SetShuffle(!st.shuffle));
                    app.cfg.playback.shuffle = !st.shuffle;
                    app.mark_dirty();
                }
                if icon_btn(ui, icon::PREV(), 17.0, v.strong_text_color(), "Previous") {
                    app.engine.send(Cmd::Prev);
                }
                // seafoam circular play/pause with a dark glyph (no glow)
                let pi = if st.playing { icon::PAUSE() } else { icon::PLAY() };
                let b = egui::Button::new(RichText::new(pi).size(16.0).color(v.extreme_bg_color)).fill(accent).corner_radius(20).min_size(egui::vec2(40.0, 40.0));
                if ui.add(b).on_hover_text("Play / Pause (Space)").clicked() {
                    app.engine.send(Cmd::Toggle);
                }
                if icon_btn(ui, icon::NEXT(), 17.0, v.strong_text_color(), "Next") {
                    app.engine.send(Cmd::Next);
                }
                let (ri, rc) = match st.repeat {
                    RepeatMode::Off => (icon::REPEAT(), v.text_color()),
                    RepeatMode::All => (icon::REPEAT(), accent),
                    RepeatMode::One => (icon::REPEAT_ONE(), accent),
                };
                if icon_btn(ui, ri, 15.0, rc, "Repeat: off / all / one") {
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
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt_ms(pos)).size(11.5).color(v.weak_text_color()));
                let mut p = pos as f32;
                ui.spacing_mut().slider_width = (ui.available_width() - 48.0).max(60.0);
                let r = ui.add_enabled(dur > 0 && !st.external, egui::Slider::new(&mut p, 0.0..=(dur.max(1) as f32)).show_value(false).trailing_fill(true));
                if r.drag_stopped() || (r.clicked() && !r.dragged()) {
                    app.engine.send(Cmd::Seek(p as u64));
                }
                ui.label(RichText::new(fmt_ms(dur)).size(11.5).color(v.weak_text_color()));
            });
            if st.external {
                ui.label(RichText::new("Playing in YouTube Music in your browser").size(10.5).color(v.weak_text_color()));
            } else if let Some(e) = &st.error {
                ui.add(egui::Label::new(RichText::new(e).size(10.5).color(Color32::from_rgb(248, 113, 113))).truncate());
            }
        });
        // ---- right: queue, lyrics, device, volume, mini-player, expand
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // frameless icon buttons (incl. dropdowns), as in the reference
            let wv = &mut ui.visuals_mut().widgets;
            wv.inactive.weak_bg_fill = Color32::TRANSPARENT;
            wv.inactive.bg_fill = Color32::TRANSPARENT;
            wv.inactive.bg_stroke = egui::Stroke::NONE;
            if icon_btn(ui, "⛶", 15.0, v.text_color(), "Now Playing") {
                if app.page == Page::NowPlaying {
                    app.go_back();
                } else {
                    app.navigate(Page::NowPlaying);
                }
            }
            if app.cfg.active("quick_settings") {
                ui.menu_button(RichText::new(icon::QUICK()).size(13.0), |ui| super::browse::quick_settings(app, ui)).response.on_hover_text("Quick settings");
            }
            if app.cfg.active("mini_player") && icon_btn(ui, icon::MINI(), 14.0, v.text_color(), "Mini-player") {
                app.do_action(crate::shortcuts::Action::ToggleMiniPlayer, &ui.ctx().clone());
            }
            let mut vol = st.volume;
            ui.spacing_mut().slider_width = 100.0;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false).trailing_fill(true)).changed() {
                app.engine.send(Cmd::SetVolume(vol));
                app.cfg.playback.volume = vol;
                app.mark_dirty();
            }
            if icon_btn(ui, if st.muted { icon::MUTE() } else { icon::VOL() }, 15.0, v.text_color(), "Mute") {
                app.engine.send(Cmd::ToggleMute);
                app.cfg.playback.muted = !st.muted;
                app.mark_dirty();
            }
            device_menu(app, ui);
            let rp = app.cfg.general.right_panel;
            let tog = |app: &mut AquaApp, which: RightPanel| {
                app.cfg.general.right_panel = if app.cfg.general.right_panel == which { RightPanel::Closed } else { which };
                app.mark_dirty();
            };
            if app.cfg.active("lyrics") && icon_btn(ui, icon::LYRICS(), 14.0, if rp == RightPanel::Lyrics { accent } else { v.text_color() }, "Lyrics") {
                tog(app, RightPanel::Lyrics);
            }
            let qc = if matches!(rp, RightPanel::Queue | RightPanel::Radio) { accent } else { v.text_color() };
            if icon_btn(ui, "☰", 15.0, qc, "Queue & Similarity Radio") {
                tog(app, RightPanel::Queue);
            }
            if app.cfg.active("vis_in_bar") && ui.available_width() > 110.0 {
                if let Some((bars, _)) = app.vis_data().filter(|(b, _)| !b.is_empty()) {
                    let bars = bars.to_vec();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(70.0, 22.0), egui::Sense::hover());
                    let n = bars.len().min(18).max(1);
                    let w = rect.width() / n as f32;
                    for i in 0..n {
                        let hh = (bars[i * bars.len() / n] * rect.height()).max(1.0);
                        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left() + i as f32 * w, rect.bottom() - hh), egui::pos2(rect.left() + (i as f32 + 0.75) * w, rect.bottom())), 1, accent);
                    }
                }
            }
        });
    });
}

fn device_menu(app: &mut AquaApp, ui: &mut Ui) {
    ui.menu_button(RichText::new("🖵").size(14.0), |ui| {
        ui.label(RichText::new("Output device").strong());
        let devs = app.settings.devices.get_or_insert_with(crate::audio::output::list_devices).clone();
        if ui.selectable_label(app.cfg.audio.device.is_empty(), "System default").clicked() {
            app.cfg.audio.device.clear();
            app.mark_dirty();
            ui.close();
        }
        for d in devs {
            if ui.selectable_label(app.cfg.audio.device == d, &d).clicked() {
                app.cfg.audio.device = d;
                app.mark_dirty();
                ui.close();
            }
        }
        if ui.small_button("Refresh list").clicked() {
            app.settings.devices = None;
        }
    })
    .response
    .on_hover_text("Output device");
}

// ---------------------------------------------------------------------------
// Right panel

/// Pill-style segmented control.
fn segmented(ui: &mut Ui, cur: &mut RightPanel, items: &[(RightPanel, &str)], accent: Color32) {
    let v = ui.visuals().clone();
    let w = ui.available_width() - 30.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 38.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 19, v.extreme_bg_color.gamma_multiply(0.6));
    let seg = rect.width() / items.len() as f32;
    for (i, (val, label)) in items.iter().enumerate() {
        let r = egui::Rect::from_min_size(rect.min + egui::vec2(seg * i as f32, 0.0), egui::vec2(seg, 38.0)).shrink(3.0);
        let resp = ui.interact(r, ui.id().with(("seg", i)), egui::Sense::click());
        let sel = *cur == *val || (*val == RightPanel::Queue && *cur == RightPanel::Info);
        if sel {
            ui.painter().rect(r, 16, accent.gamma_multiply(0.22), egui::Stroke::new(1.0, accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 16, v.widgets.inactive.bg_fill.gamma_multiply(0.6));
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(14.0), if sel { v.strong_text_color() } else { v.text_color() });
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *cur = *val;
        }
    }
}

fn right_panel(app: &mut AquaApp, ui: &mut Ui) {
    let accent = app.accent();
    ui.horizontal(|ui| {
        let mut rp = app.cfg.general.right_panel;
        let mut items = vec![(RightPanel::Queue, "Queue"), (RightPanel::Radio, "Similarity Radio")];
        if app.cfg.active("lyrics") {
            items.push((RightPanel::Lyrics, "Lyrics"));
        }
        segmented(ui, &mut rp, &items, accent);
        if ui.add(egui::Button::new(RichText::new("✖").size(12.0).color(ui.visuals().weak_text_color())).frame(false)).on_hover_text("Close panel").clicked() {
            rp = RightPanel::Closed;
        }
        if rp != app.cfg.general.right_panel {
            app.cfg.general.right_panel = rp;
            app.mark_dirty();
        }
    });
    ui.add_space(10.0);
    match app.cfg.general.right_panel {
        RightPanel::Queue | RightPanel::Info => queue_tab(app, ui),
        RightPanel::Radio => radio_tab(app, ui),
        RightPanel::Lyrics => super::pages::lyrics_panel(app, ui, false),
        RightPanel::Closed => {}
    }
}

/// Similarity Radio status + controls (shared by the right panel, Now Playing and Discover).
pub fn radio_controls(app: &mut AquaApp, ui: &mut Ui) {
    if app.radio.is_none() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Similarity Radio is off").color(ui.visuals().weak_text_color()));
            if ui.small_button("Turn on").clicked() {
                app.request_toggle("similarity_radio", true);
            }
        });
        return;
    }
    let st = app.radio_status().unwrap_or_default();
    let accent = app.accent();
    ui.horizontal(|ui| {
        let mut on = st.active;
        if widgets::switch(ui, &mut on).on_hover_text("Radio on/off for this session").changed() {
            if on {
                if let Some(t) = app.state.current.clone() {
                    app.radio_send(crate::radio::Msg::Start(t));
                }
            } else {
                app.radio_send(crate::radio::Msg::Stop);
            }
        }
        if st.active {
            ui.label(RichText::new("⏺ Similarity Radio Active").size(12.5).color(accent));
            if st.fetching {
                ui.spinner();
            }
        } else {
            ui.label(RichText::new("Similarity Radio").size(12.5).color(ui.visuals().weak_text_color()));
        }
    });
    if st.active {
        if let Some(s) = &st.seed {
            ui.label(RichText::new(format!("Based on {} — {}", s.artist_line(), s.title)).size(11.5).color(ui.visuals().weak_text_color()));
        }
        if !st.source.is_empty() {
            ui.label(RichText::new(&st.source).size(11.0).color(ui.visuals().weak_text_color()));
        }
        if let Some(e) = &st.error {
            ui.add(egui::Label::new(RichText::new(e).size(11.5).color(widgets::impact_color(crate::features::Impact::High))).wrap());
        } else if let Some(m) = &st.message {
            ui.add(egui::Label::new(RichText::new(m).size(11.5).color(ui.visuals().weak_text_color())).wrap());
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(st.active, egui::Button::new("⟳ Refresh")).on_hover_text("Replace upcoming recommendations").clicked() {
            app.radio_send(crate::radio::Msg::Refresh);
        }
        if let Some(t) = app.state.current.clone() {
            if ui.button("More Like This").on_hover_text("Re-seed the radio from the current song").clicked() {
                app.radio_send(crate::radio::Msg::MoreLike(t));
            }
        }
    });
    ui.horizontal(|ui| {
        let mut close = app.cfg.on("radio_stay_close");
        if widgets::switch(ui, &mut close).changed() {
            app.set_feature("radio_stay_close", close);
        }
        ui.label(RichText::new("Stay close to original").size(12.5));
    });
    if app.lastfm_key.is_empty() {
        ui.label(RichText::new("Add a free Last.fm API key (Settings › Playback) for real similarity recommendations; without it the radio uses the same artist/album.").size(11.0).color(ui.visuals().weak_text_color()));
    }
}

fn section_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(4.0);
}

fn queue_tab(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let accent = app.accent();
    let mut cmd = None;
    if let Some(t) = &st.current {
        section_label(ui, "Now Playing");
        ui.horizontal(|ui| {
            let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
            widgets::artwork(ui, tex.as_ref(), 60.0, 6);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 34.0);
                ui.add(egui::Label::new(RichText::new(&t.title).size(14.5).strong()).truncate());
                ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.5).color(ui.visuals().weak_text_color())).truncate());
            });
            if t.provider != ProviderKind::Local {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let liked = app.is_liked(t);
                    if icon_btn(ui, if liked { "♥" } else { "♡" }, 17.0, if liked { accent } else { ui.visuals().text_color() }, "Like") && !liked {
                        let t = t.clone();
                        app.like(&t);
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.separator();
    }
    let pos = st.queue_pos.map(|p| p + 1).unwrap_or(0);
    let upcoming: Vec<(usize, &crate::model::Track, bool)> = st.queue.iter().enumerate().skip(pos).map(|(i, t)| (i, t, st.queue_radio.get(i).copied().unwrap_or(false))).collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Up Next ({})", upcoming.len())).size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let rc = if st.repeat != RepeatMode::Off { accent } else { ui.visuals().text_color() };
            if icon_btn(ui, icon::REPEAT(), 14.0, rc, "Repeat") {
                let r = match st.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                app.engine.send(Cmd::SetRepeat(r));
            }
            if icon_btn(ui, icon::SHUFFLE(), 14.0, if st.shuffle { accent } else { ui.visuals().text_color() }, "Shuffle") {
                app.engine.send(Cmd::SetShuffle(!st.shuffle));
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("rp_queue").auto_shrink([false, false]).show(ui, |ui| {
        let mut shown_radio_header = false;
        for (i, t, radio) in upcoming.iter().take(200) {
            if *radio && !shown_radio_header {
                shown_radio_header = true;
                ui.add_space(6.0);
                ui.label(RichText::new("Automatically Recommended").size(12.5).color(accent));
            }
            queue_row(app, ui, t, *i, *radio, &mut cmd);
        }
        if upcoming.is_empty() {
            ui.label(RichText::new("Nothing queued. Play a song from search to start Similarity Radio.").size(12.5).color(ui.visuals().weak_text_color()));
        }
    });
    if let Some(c) = cmd {
        app.engine.send(c);
    }
}

fn radio_tab(app: &mut AquaApp, ui: &mut Ui) {
    radio_controls(app, ui);
    ui.add_space(8.0);
    ui.separator();
    let st = app.state.clone();
    let mut cmd = None;
    let mut save = false;
    let pos = st.queue_pos.map(|p| p + 1).unwrap_or(0);
    let recs: Vec<(usize, &crate::model::Track)> = st.queue.iter().enumerate().skip(pos).filter(|(i, _)| st.queue_radio.get(*i).copied().unwrap_or(false)).collect();
    section_label(ui, &format!("Automatically Recommended ({})", recs.len()));
    egui::ScrollArea::vertical().id_salt("rp_radio").auto_shrink([false, false]).max_height(ui.available_height() - 40.0).show(ui, |ui| {
        for (i, t) in &recs {
            queue_row(app, ui, t, *i, true, &mut cmd);
        }
        if recs.is_empty() {
            ui.label(RichText::new("No recommendations queued.").size(12.5).color(ui.visuals().weak_text_color()));
        }
    });
    if !st.queue.is_empty() && ui.button("Save queue as playlist").clicked() {
        save = true;
    }
    if let Some(c) = cmd {
        app.engine.send(c);
    }
    if save {
        super::browse::save_queue_as_playlist(app);
    }
}

fn queue_row(app: &mut AquaApp, ui: &mut Ui, t: &crate::model::Track, i: usize, radio: bool, cmd: &mut Option<Cmd>) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 56.0), egui::Sense::click());
    let v = ui.visuals().clone();
    if resp.hovered() {
        ui.painter().rect_filled(rect, 8, v.widgets.inactive.bg_fill.gamma_multiply(0.6));
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(4.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover) } else { None };
    widgets::artwork(&mut child, tex.as_ref(), 46.0, 6);
    child.add_space(4.0);
    child.vertical(|ui| {
        ui.set_width((w - 120.0).max(80.0));
        ui.add(egui::Label::new(RichText::new(&t.title).size(14.0).strong()).truncate());
        ui.horizontal(|ui| {
            if t.provider != ProviderKind::Local {
                widgets::provider_badge(ui, t.provider);
            }
            ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.0).color(v.weak_text_color())).truncate());
        });
    });
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if resp.hovered() {
            if widgets::small_button(ui, "✖", if radio { "Remove recommendation" } else { "Remove" }).clicked() {
                *cmd = Some(Cmd::Remove(i));
            }
        } else {
            ui.label(RichText::new(fmt_ms(t.duration_ms)).size(12.0).color(v.weak_text_color()));
        }
    });
    if resp.double_clicked() {
        *cmd = Some(Cmd::JumpTo(i));
    }
    let _ = PlayFrom::Other;
}
