//! Application frame: collapsible sidebar, top search bar, persistent bottom
//! player and the optional right panel (Queue & Radio / Lyrics / Now Playing).

use super::widgets::{self, icon};
use super::{AquaApp, Filter, Page, PlayFrom};
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::{PlayerBarLayout, ProviderKind, RepeatMode, RightPanel};
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText, Ui};

pub fn shell(app: &mut AquaApp, ui: &mut Ui) {
    let theme = app.current_theme().clone();
    let translucent = app.cfg.active("translucent_panels") || app.cfg.active("window_backdrop");
    let a = if translucent { theme.panel_alpha } else { 1.0 };
    let custom = app.cfg.active("custom_theme");
    let sidebar_fill = if custom { crate::theme::ca(theme.sidebar, a) } else { ui.visuals().panel_fill };
    let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let bar_h = match app.cfg.appearance.player_bar_layout {
        PlayerBarLayout::Compact => 60.0,
        _ => app.cfg.appearance.player_bar_height.clamp(64.0, 110.0),
    };
    egui::Panel::bottom("player_bar")
        .exact_size(bar_h)
        .frame(egui::Frame::new().fill(sidebar_fill).stroke(egui::Stroke::new(1.0, border)).inner_margin(egui::Margin::symmetric(14, 6)))
        .show(ui, |ui| player_bar(app, ui));
    let collapsed = app.cfg.general.sidebar_collapsed;
    egui::Panel::left("sidebar")
        .resizable(!collapsed)
        .exact_size(if collapsed { 64.0 } else { app.cfg.general.sidebar_width })
        .frame(egui::Frame::new().fill(sidebar_fill).inner_margin(egui::Margin::symmetric(if collapsed { 8 } else { 12 }, 10)))
        .show(ui, |ui| {
            if !collapsed {
                let w = ui.available_width() + 24.0;
                if (w - app.cfg.general.sidebar_width).abs() > 2.0 && ui.input(|i| !i.pointer.any_down()) {
                    app.cfg.general.sidebar_width = w.clamp(180.0, 360.0);
                    app.mark_dirty();
                }
            }
            sidebar(app, ui, collapsed)
        });
    if app.cfg.general.right_panel != RightPanel::Closed {
        egui::Panel::right("right_panel")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=480.0)
            .frame(egui::Frame::new().fill(sidebar_fill).inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ui, |ui| right_panel(app, ui));
    }
    let central_fill = if app.bg.active { Color32::TRANSPARENT } else { ui.visuals().extreme_bg_color };
    egui::CentralPanel::default().frame(egui::Frame::new().fill(central_fill)).show(ui, |ui| {
        egui::Panel::top("top_bar").exact_size(52.0).frame(egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 10, bottom: 6 })).show(ui, |ui| top_bar(app, ui));
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 22, top: 4, bottom: 8 })).show(ui, |ui| {
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

fn nav_item(app: &mut AquaApp, ui: &mut Ui, ic: &str, label: &str, page: Page, collapsed: bool) {
    let active = app.page == page || matches!((&app.page, &page), (Page::Playlist(..), Page::Playlists) | (Page::Album(_), Page::Albums) | (Page::Artist(_), Page::Artists));
    let h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
    let v = ui.visuals().clone();
    if active {
        ui.painter().rect_filled(rect, 6, v.widgets.inactive.bg_fill);
        let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 8.0), egui::vec2(3.0, h - 16.0));
        ui.painter().rect_filled(bar, 2, app.accent());
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6, v.widgets.inactive.bg_fill.gamma_multiply(0.6));
    }
    let col = if active { v.strong_text_color() } else { v.weak_text_color() };
    let col = if resp.hovered() && !active { v.text_color() } else { col };
    if collapsed {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ic, egui::FontId::proportional(17.0), col);
    } else {
        ui.painter().text(rect.left_center() + egui::vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, ic, egui::FontId::proportional(16.0), col);
        let f = if active { egui::FontId::new(14.5, egui::FontFamily::Proportional) } else { egui::FontId::proportional(14.0) };
        ui.painter().text(rect.left_center() + egui::vec2(42.0, 0.0), egui::Align2::LEFT_CENTER, label, f, col);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if collapsed { resp.on_hover_text(label) } else { resp };
    if resp.clicked() {
        app.navigate(page);
    }
}

fn sidebar(app: &mut AquaApp, ui: &mut Ui, collapsed: bool) {
    // understated branding: small droplet + small name
    ui.horizontal(|ui| {
        if !collapsed || app.cfg.active("sidebar_branding") {
            ui.label(RichText::new("💧").size(13.0).color(app.accent()));
        }
        if !collapsed {
            ui.label(RichText::new("Aqua Glass").size(12.5).color(ui.visuals().weak_text_color()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let t = if collapsed { "»" } else { "«" };
            if ui.add(egui::Button::new(RichText::new(t).size(13.0)).frame(false)).on_hover_text(if collapsed { "Expand sidebar" } else { "Collapse sidebar" }).clicked() {
                app.cfg.general.sidebar_collapsed = !collapsed;
                app.mark_dirty();
            }
            if !collapsed {
                app_menu(app, ui);
            }
        });
    });
    ui.add_space(10.0);
    nav_item(app, ui, icon::HOME(), "Home", Page::Home, collapsed);
    nav_item(app, ui, icon::SEARCH(), "Search", Page::Search, collapsed);
    nav_item(app, ui, icon::LIBRARY(), "Library", Page::Library, collapsed);
    ui.add_space(6.0);
    nav_item(app, ui, icon::PLAYLIST(), "Playlists", Page::Playlists, collapsed);
    nav_item(app, ui, icon::ARTIST(), "Artists", Page::Artists, collapsed);
    nav_item(app, ui, icon::ALBUM(), "Albums", Page::Albums, collapsed);
    ui.add_space(6.0);
    nav_item(app, ui, "📶", "Discover", Page::Discover, collapsed);
    nav_item(app, ui, icon::QUEUE(), "Queue", Page::Queue, collapsed);
    nav_item(app, ui, icon::TRANSFER(), "Playlist Transfer", Page::Transfer, collapsed);

    // bottom: settings + compact RAM indicator
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ram_indicator(app, ui, collapsed);
        ui.add_space(6.0);
        nav_item(app, ui, icon::SETTINGS(), "Settings", Page::Settings, collapsed);
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
        if !collapsed {
            // the user's playlists, compact (fills the remaining height)
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.separator();
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
    ui.menu_button(RichText::new("☰").size(12.0), |ui| {
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

fn ram_indicator(app: &mut AquaApp, ui: &mut Ui, collapsed: bool) {
    let mb = app.monitor.current_mb();
    let target = app.cfg.perf.ram_target_mb as f32;
    let state = match app.pressure {
        crate::perf::Pressure::Normal => "Normal",
        crate::perf::Pressure::Elevated => "Saving memory",
        crate::perf::Pressure::Over => "Over target",
    };
    let col = match app.pressure {
        crate::perf::Pressure::Normal => app.accent(),
        crate::perf::Pressure::Elevated => widgets::impact_color(crate::features::Impact::High),
        crate::perf::Pressure::Over => widgets::impact_color(crate::features::Impact::VeryHigh),
    };
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, if collapsed { 26.0 } else { 30.0 }), egui::Sense::click());
    let p = ui.painter();
    let weak = ui.visuals().weak_text_color();
    if collapsed {
        p.text(rect.center_top(), egui::Align2::CENTER_TOP, format!("{mb:.0}"), egui::FontId::proportional(11.0), weak);
    } else {
        p.text(rect.left_top(), egui::Align2::LEFT_TOP, format!("RAM {mb:.0} / {target:.0} MB"), egui::FontId::proportional(11.0), weak);
        p.text(rect.right_top(), egui::Align2::RIGHT_TOP, state, egui::FontId::proportional(10.5), if app.pressure == crate::perf::Pressure::Normal { weak } else { col });
    }
    let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.bottom() - 4.0), egui::vec2(w, 4.0));
    p.rect_filled(bar, 2, ui.visuals().widgets.inactive.bg_fill);
    let mut f = bar;
    f.set_width(w * (mb / target.max(1.0)).clamp(0.0, 1.0));
    p.rect_filled(f, 2, col);
    if resp
        .on_hover_text(format!("Private memory {mb:.0} MB of the {target:.0} MB target — {state}. Click for the Performance dashboard."))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.settings.category = crate::features::Category::Performance;
        app.navigate(Page::Settings);
    }
}

fn top_bar(app: &mut AquaApp, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        let can_back = !app.back.is_empty();
        if ui.add_enabled(can_back, egui::Button::new("‹").min_size(egui::vec2(28.0, 28.0)).corner_radius(14)).on_hover_text("Back").clicked() {
            app.go_back();
        }
        ui.add_space(6.0);
        // prominent search field
        let fill = ui.visuals().faint_bg_color;
        let r = egui::Frame::new().fill(fill).corner_radius(18).inner_margin(egui::Margin::symmetric(12, 5)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::SEARCH()).color(ui.visuals().weak_text_color()));
                ui.add(egui::TextEdit::singleline(&mut app.top_search).frame(egui::Frame::NONE).hint_text("What do you want to play?").desired_width(320.0))
            })
            .inner
        });
        let te = r.inner;
        if app.search.focus {
            te.request_focus();
            app.search.focus = false;
        }
        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !app.top_search.trim().is_empty() {
            app.search.query = app.top_search.clone();
            app.navigate(Page::Search);
            app.run_search();
        }
        if app.search.pending > 0 {
            ui.spinner();
        }
        // provider filter chips
        ui.add_space(10.0);
        let mut f = app.filter;
        chip(ui, &mut f, Filter::All, "All");
        for p in [ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local] {
            if app.provider_enabled(p) {
                let label = match p {
                    ProviderKind::Spotify => "Spotify",
                    ProviderKind::YouTube => "YouTube",
                    ProviderKind::Local => "Local",
                };
                chip(ui, &mut f, Filter::Only(p), label);
            }
        }
        if f != app.filter {
            app.filter = f;
            if app.page == Page::Search && !app.search.query.is_empty() {
                app.run_search();
            }
        }
        // account status, right-aligned and subtle (hidden when space is tight)
        if ui.available_width() < 170.0 {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let dot = |ui: &mut Ui, on: bool, name: &str, tip: String| {
                let c = if on { Color32::from_rgb(98, 214, 178) } else { ui.visuals().weak_text_color().gamma_multiply(0.6) };
                ui.label(RichText::new(format!("⏺ {name}")).size(11.0).color(c)).on_hover_text(tip);
            };
            if app.cfg.services.youtube_enabled {
                dot(ui, app.youtube.is_signed_in(), "YouTube", app.yt_status.clone());
            }
            if app.cfg.services.spotify_enabled {
                let tip = match &app.state.spotify {
                    crate::audio::engine::SpotifyStatus::Ready => "Library + Premium playback ready".to_string(),
                    crate::audio::engine::SpotifyStatus::Unavailable(e) => format!("Playback unavailable: {e}"),
                    _ => app.sp_status.clone(),
                };
                dot(ui, app.spotify.is_signed_in(), "Spotify", tip);
            }
        });
    });
}

fn chip(ui: &mut Ui, f: &mut Filter, v: Filter, label: &str) {
    let sel = *f == v;
    let b = egui::Button::new(RichText::new(label).size(12.5)).selected(sel).corner_radius(14).min_size(egui::vec2(0.0, 26.0));
    if ui.add(b).clicked() {
        *f = v;
    }
}

// ---------------------------------------------------------------------------
// Bottom player

fn player_bar(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let pos = app.engine.position_ms();
    let dur = st.duration_ms;
    let accent = app.accent();
    let compact = app.cfg.appearance.player_bar_layout == PlayerBarLayout::Compact;
    let full_w = ui.available_width();
    let side_w = (full_w * 0.28).clamp(200.0, 360.0);
    let center_w = (full_w - side_w * 2.0).max(240.0);
    let h = ui.available_height();
    ui.horizontal(|ui| {
        // ---- left: artwork, title, artist, like
        ui.allocate_ui_with_layout(egui::vec2(side_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let art = (h - 10.0).clamp(36.0, 56.0);
            if let Some(t) = &st.current {
                let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
                if widgets::artwork(ui, tex.as_ref(), art, 4).on_hover_text("Now Playing").clicked() {
                    app.navigate(Page::NowPlaying);
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.add_space(((h - 40.0) / 2.0).max(0.0));
                    ui.set_max_width(side_w - art - 60.0);
                    if ui.add(egui::Label::new(RichText::new(&t.title).size(13.5).strong()).truncate().sense(egui::Sense::click())).clicked() {
                        app.navigate(Page::NowPlaying);
                    }
                    ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.0).color(ui.visuals().weak_text_color())).truncate());
                });
                if t.provider != ProviderKind::Local {
                    let liked = app.is_liked(t);
                    let txt = RichText::new(if liked { "♥" } else { "♡" }).size(15.0).color(if liked { accent } else { ui.visuals().weak_text_color() });
                    if ui.add(egui::Button::new(txt).frame(false)).on_hover_text(if liked { "Liked" } else { "Like" }).clicked() && !liked {
                        let t = t.clone();
                        app.like(&t);
                    }
                }
            } else {
                widgets::artwork(ui, None, art, 4);
                ui.label(RichText::new("Nothing playing").color(ui.visuals().weak_text_color()));
            }
        });
        // ---- center: controls + progress
        ui.allocate_ui_with_layout(egui::vec2(center_w, h), egui::Layout::top_down(egui::Align::Center), |ui| {
            if !compact {
                ui.add_space(((h - 52.0) / 2.0).max(0.0));
            }
            ui.horizontal(|ui| {
                let w = 5.0 * 34.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                let weak = ui.visuals().weak_text_color();
                if ui.add(egui::Button::new(RichText::new(icon::SHUFFLE()).color(if st.shuffle { accent } else { weak })).frame(false)).on_hover_text("Shuffle").clicked() {
                    app.engine.send(Cmd::SetShuffle(!st.shuffle));
                    app.cfg.playback.shuffle = !st.shuffle;
                    app.mark_dirty();
                }
                if ui.add(egui::Button::new(RichText::new(icon::PREV()).size(16.0)).frame(false)).on_hover_text("Previous").clicked() {
                    app.engine.send(Cmd::Prev);
                }
                // compact, non-glowing play button
                let pi = if st.playing { icon::PAUSE() } else { icon::PLAY() };
                let fg = ui.visuals().extreme_bg_color;
                let b = egui::Button::new(RichText::new(pi).size(15.0).color(fg)).fill(ui.visuals().strong_text_color()).corner_radius(16).min_size(egui::vec2(32.0, 32.0));
                if ui.add(b).on_hover_text("Play / Pause (Space)").clicked() {
                    app.engine.send(Cmd::Toggle);
                }
                if ui.add(egui::Button::new(RichText::new(icon::NEXT()).size(16.0)).frame(false)).on_hover_text("Next").clicked() {
                    app.engine.send(Cmd::Next);
                }
                let (ri, rc) = match st.repeat {
                    RepeatMode::Off => (icon::REPEAT(), weak),
                    RepeatMode::All => (icon::REPEAT(), accent),
                    RepeatMode::One => (icon::REPEAT_ONE(), accent),
                };
                if ui.add(egui::Button::new(RichText::new(ri).color(rc)).frame(false)).on_hover_text("Repeat: off / all / one").clicked() {
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
                ui.label(RichText::new(fmt_ms(pos)).size(11.0).color(ui.visuals().weak_text_color()));
                let mut p = pos as f32;
                ui.spacing_mut().slider_width = (ui.available_width() - 44.0).max(60.0);
                let r = ui.add_enabled(dur > 0 && !st.external, egui::Slider::new(&mut p, 0.0..=(dur.max(1) as f32)).show_value(false).trailing_fill(true));
                if r.drag_stopped() || (r.clicked() && !r.dragged()) {
                    app.engine.send(Cmd::Seek(p as u64));
                }
                ui.label(RichText::new(fmt_ms(dur)).size(11.0).color(ui.visuals().weak_text_color()));
            });
            if st.external {
                ui.label(RichText::new("Playing in YouTube Music in your browser").size(10.5).color(ui.visuals().weak_text_color()));
            } else if let Some(e) = &st.error {
                ui.add(egui::Label::new(RichText::new(e).size(10.5).color(Color32::from_rgb(248, 113, 113))).truncate());
            }
        });
        // ---- right: queue, lyrics, device, volume, mini-player
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.cfg.active("quick_settings") {
                ui.menu_button(RichText::new(icon::QUICK()).size(13.0), |ui| super::browse::quick_settings(app, ui)).response.on_hover_text("Quick settings");
            }
            if app.cfg.active("mini_player") && widgets::small_button(ui, icon::MINI(), "Mini-player").clicked() {
                app.do_action(crate::shortcuts::Action::ToggleMiniPlayer, &ui.ctx().clone());
            }
            let mut v = st.volume;
            ui.spacing_mut().slider_width = 90.0;
            if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false).trailing_fill(true)).changed() {
                app.engine.send(Cmd::SetVolume(v));
                app.cfg.playback.volume = v;
                app.mark_dirty();
            }
            if widgets::small_button(ui, if st.muted { icon::MUTE() } else { icon::VOL() }, "Mute").clicked() {
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
            if app.cfg.active("lyrics") {
                let c = if rp == RightPanel::Lyrics { accent } else { ui.visuals().text_color() };
                if ui.add(egui::Button::new(RichText::new(icon::LYRICS()).color(c)).frame(false)).on_hover_text("Lyrics").clicked() {
                    tog(app, RightPanel::Lyrics);
                }
            }
            let c = if rp == RightPanel::Queue { accent } else { ui.visuals().text_color() };
            if ui.add(egui::Button::new(RichText::new(icon::QUEUE()).color(c)).frame(false)).on_hover_text("Queue & Similarity Radio").clicked() {
                tog(app, RightPanel::Queue);
            }
            if app.cfg.active("vis_in_bar") && ui.available_width() > 110.0 {
                if let Some((bars, _)) = app.vis_data().filter(|(b, _)| !b.is_empty()) {
                    let bars = bars.to_vec();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(70.0, 22.0), egui::Sense::hover());
                    let n = bars.len().min(18).max(1);
                    let w = rect.width() / n as f32;
                    for i in 0..n {
                        let h = (bars[i * bars.len() / n] * rect.height()).max(1.0);
                        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left() + i as f32 * w, rect.bottom() - h), egui::pos2(rect.left() + (i as f32 + 0.75) * w, rect.bottom())), 1, accent);
                    }
                }
            }
        });
    });
}

fn device_menu(app: &mut AquaApp, ui: &mut Ui) {
    ui.menu_button(RichText::new("🎧").size(13.0), |ui| {
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

fn right_panel(app: &mut AquaApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let mut rp = app.cfg.general.right_panel;
        ui.selectable_value(&mut rp, RightPanel::Queue, "Queue");
        if app.cfg.active("lyrics") {
            ui.selectable_value(&mut rp, RightPanel::Lyrics, "Lyrics");
        }
        ui.selectable_value(&mut rp, RightPanel::Info, "Now Playing");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("✖").frame(false)).on_hover_text("Close panel").clicked() {
                rp = RightPanel::Closed;
            }
        });
        if rp != app.cfg.general.right_panel {
            app.cfg.general.right_panel = rp;
            app.mark_dirty();
        }
    });
    ui.separator();
    match app.cfg.general.right_panel {
        RightPanel::Queue => queue_panel(app, ui),
        RightPanel::Lyrics => super::pages::lyrics_panel(app, ui, false),
        RightPanel::Info => info_panel(app, ui),
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
            ui.label(RichText::new("⏺ Similarity Radio Active").size(12.0).color(accent));
            if st.fetching {
                ui.spinner();
            }
        } else {
            ui.label(RichText::new("Similarity Radio").size(12.0).color(ui.visuals().weak_text_color()));
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
        ui.horizontal(|ui| {
            let mut close = app.cfg.on("radio_stay_close");
            if widgets::switch(ui, &mut close).changed() {
                app.set_feature("radio_stay_close", close);
            }
            ui.label(RichText::new("Stay close to original").size(12.5));
        });
    });
    if app.lastfm_key.is_empty() {
        ui.label(RichText::new("Add a free Last.fm API key (Settings › Playback) for real similarity recommendations; without it the radio uses the same artist/album.").size(11.0).color(ui.visuals().weak_text_color()));
    }
}

fn queue_panel(app: &mut AquaApp, ui: &mut Ui) {
    widgets::card(ui, None, |ui| radio_controls(app, ui));
    let st = app.state.clone();
    let mut cmd = None;
    let mut save = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let pos = st.queue_pos.unwrap_or(0);
        if let Some(t) = &st.current {
            ui.label(RichText::new("Now Playing").strong());
            queue_row(app, ui, t, pos, true, false, &mut cmd);
        }
        let upcoming: Vec<(usize, &crate::model::Track, bool)> = st.queue.iter().enumerate().skip(pos + 1).map(|(i, t)| (i, t, st.queue_radio.get(i).copied().unwrap_or(false))).collect();
        let manual: Vec<_> = upcoming.iter().filter(|x| !x.2).collect();
        let radio: Vec<_> = upcoming.iter().filter(|x| x.2).collect();
        if !manual.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Up Next").strong());
            for (i, t, _) in manual.iter().take(50) {
                queue_row(app, ui, t, *i, false, false, &mut cmd);
            }
            if manual.len() > 50 {
                ui.label(RichText::new(format!("+{} more — open the Queue page", manual.len() - 50)).small().weak());
            }
        }
        if !radio.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Automatically Recommended").strong());
            for (i, t, _) in radio.iter() {
                queue_row(app, ui, t, *i, false, true, &mut cmd);
            }
        }
        if st.queue.is_empty() {
            ui.label(RichText::new("The queue is empty.").weak());
        }
        ui.add_space(8.0);
        if !st.queue.is_empty() && ui.button("Save queue as playlist").clicked() {
            save = true;
        }
    });
    if let Some(c) = cmd {
        app.engine.send(c);
    }
    if save {
        super::browse::save_queue_as_playlist(app);
    }
}

fn queue_row(app: &mut AquaApp, ui: &mut Ui, t: &crate::model::Track, i: usize, current: bool, radio: bool, cmd: &mut Option<Cmd>) {
    let r = ui.horizontal(|ui| {
        let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover) } else { None };
        widgets::artwork(ui, tex.as_ref(), 36.0, 4);
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 30.0);
            let c = if current { app.accent() } else { ui.visuals().strong_text_color() };
            ui.add(egui::Label::new(RichText::new(&t.title).size(13.0).color(c)).truncate());
            ui.horizontal(|ui| {
                widgets::provider_badge(ui, t.provider);
                ui.add(egui::Label::new(RichText::new(t.artist_line()).size(11.5).color(ui.visuals().weak_text_color())).truncate());
            });
        });
        if !current {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::small_button(ui, "✖", if radio { "Remove recommendation" } else { "Remove" }).clicked() {
                    *cmd = Some(Cmd::Remove(i));
                }
            });
        }
    });
    if !current && r.response.interact(egui::Sense::click()).double_clicked() {
        *cmd = Some(Cmd::JumpTo(i));
    }
}

fn info_panel(app: &mut AquaApp, ui: &mut Ui) {
    let Some(t) = app.state.current.clone() else {
        ui.label(RichText::new("Nothing playing").weak());
        return;
    };
    let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
    let w = ui.available_width();
    widgets::artwork(ui, tex.as_ref(), w, 8);
    ui.add_space(6.0);
    ui.label(RichText::new(&t.title).size(17.0).strong());
    ui.label(RichText::new(t.artist_line()).color(ui.visuals().weak_text_color()));
    if !t.album.is_empty() {
        ui.label(RichText::new(&t.album).small().weak());
    }
    ui.horizontal(|ui| {
        widgets::provider_badge(ui, t.provider);
        if t.provider != ProviderKind::Local && ui.small_button("Other version…").clicked() {
            app.find_alternatives(&t);
        }
        if ui.small_button("Open Now Playing").clicked() {
            app.navigate(Page::NowPlaying);
        }
    });
    ui.add_space(8.0);
    widgets::card(ui, None, |ui| radio_controls(app, ui));
    let _ = PlayFrom::Other;
}
