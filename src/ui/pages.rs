//! Application shell and content pages.

use super::widgets::{self, icon};
use super::{AquaApp, Confirm, Filter, Page};
use crate::artwork::Kind;
use crate::audio::engine::{Cmd, SpotifyStatus};
use crate::config::{PlayerBarLayout, ProviderKind, RepeatMode};
use crate::library::LIKED_ID;
use crate::model::*;
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText, Ui};

pub fn shell(app: &mut AquaApp, ui: &mut Ui) {
    let translucent = app.cfg.active("translucent_panels") || app.cfg.active("window_backdrop");
    let panel_fill = ui.visuals().panel_fill;
    let fill = if translucent { panel_fill } else { panel_fill.to_opaque() };
    let bar_h = match app.cfg.appearance.player_bar_layout {
        PlayerBarLayout::Compact => 56.0,
        _ => app.cfg.appearance.player_bar_height,
    };
    egui::Panel::bottom("player_bar")
        .exact_size(bar_h)
        .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6)))
        .show(ui, |ui| player_bar(app, ui));
    egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(app.cfg.general.sidebar_width)
        .size_range(150.0..=400.0)
        .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin::same(10)))
        .show(ui, |ui| {
            let w = ui.available_width() + 20.0;
            if (w - app.cfg.general.sidebar_width).abs() > 2.0 && ui.input(|i| !i.pointer.any_down()) {
                app.cfg.general.sidebar_width = w;
                app.mark_dirty();
            }
            sidebar(app, ui)
        });
    let central_fill = if app.bg.active { Color32::TRANSPARENT } else { ui.visuals().extreme_bg_color };
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(central_fill).inner_margin(egui::Margin::symmetric(18, 12)))
        .show(ui, |ui| {
            let page = app.page.clone();
            match page {
                Page::Home => home(app, ui),
                Page::Search => search(app, ui),
                Page::Library => library(app, ui),
                Page::Playlists => playlists_page(app, ui),
                Page::Artists => artists_page(app, ui),
                Page::Albums => albums_page(app, ui),
                Page::NowPlaying => now_playing(app, ui),
                Page::Queue => queue(app, ui),
                Page::Transfer => super::transfer_ui::page(app, ui),
                Page::Settings => super::settings::page(app, ui),
                Page::Playlist(p, id) => playlist_view(app, ui, p, &id),
                Page::Album(a) => album_view(app, ui, &a),
                Page::Artist(a) => artist_view(app, ui, &a),
            }
        });
}

fn nav(app: &mut AquaApp, ui: &mut Ui, ic: &str, label: &str, page: Page) {
    let sel = app.page == page;
    let r = ui.add_sized([ui.available_width(), 30.0], egui::Button::selectable(sel, format!("{ic}  {label}")));
    if r.clicked() {
        app.navigate(page);
    }
}

fn sidebar(app: &mut AquaApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("◆").color(app.accent()).size(22.0));
        ui.label(RichText::new("Aqua Glass").strong().size(19.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // application menu
            ui.menu_button("☰", |ui| {
                if ui.button(format!("{}  Settings   (Ctrl+,)", icon::SETTINGS)).clicked() {
                    app.navigate(Page::Settings);
                    ui.close();
                }
                if ui.button(format!("{}  Mini-player", icon::MINI)).clicked() {
                    app.do_action(crate::shortcuts::Action::ToggleMiniPlayer, &ui.ctx().clone());
                    ui.close();
                }
                if ui.button(format!("{}  Playlist Transfer", icon::TRANSFER)).clicked() {
                    app.navigate(Page::Transfer);
                    ui.close();
                }
                if ui.button(format!("{}  Sync library now", icon::SYNC)).clicked() {
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
            });
        });
    });
    ui.add_space(8.0);
    nav(app, ui, icon::HOME, "Home", Page::Home);
    nav(app, ui, icon::SEARCH, "Search", Page::Search);
    nav(app, ui, icon::LIBRARY, "Library", Page::Library);
    nav(app, ui, icon::PLAYLIST, "Playlists", Page::Playlists);
    nav(app, ui, icon::ARTIST, "Artists", Page::Artists);
    nav(app, ui, icon::ALBUM, "Albums", Page::Albums);
    nav(app, ui, icon::NOW, "Now Playing", Page::NowPlaying);
    nav(app, ui, icon::QUEUE, "Queue", Page::Queue);
    nav(app, ui, icon::TRANSFER, "Playlist Transfer", Page::Transfer);
    ui.add_space(10.0);
    ui.label(RichText::new("SERVICES").small().weak());
    ui.horizontal_wrapped(|ui| {
        let mut f = app.filter;
        ui.selectable_value(&mut f, Filter::All, "All");
        for p in [ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local] {
            if app.provider_enabled(p) {
                ui.selectable_value(&mut f, Filter::Only(p), p.short()).on_hover_text(format!("Show only {}", p.label()));
            }
        }
        app.filter = f;
    });
    let dot = |ui: &mut Ui, on: bool, label: &str, tip: &str| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(if on { Color32::from_rgb(52, 211, 153) } else { Color32::GRAY }));
            ui.label(RichText::new(label).small()).on_hover_text(tip);
        });
    };
    if app.cfg.services.spotify_enabled {
        let st = app.state.spotify.clone();
        let tip = match &st {
            SpotifyStatus::Ready => "Library + Premium playback ready".to_string(),
            SpotifyStatus::Unavailable(e) => format!("Playback unavailable: {e}"),
            _ => app.sp_status.clone(),
        };
        dot(ui, app.spotify.is_signed_in(), "Spotify", &tip);
    }
    if app.cfg.services.youtube_enabled {
        dot(ui, app.youtube.is_signed_in(), "YouTube Music", &app.yt_status.clone());
    }

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(2.0);
        // permanently visible settings button + live RAM
        ui.horizontal(|ui| {
            let r = ui.add(egui::Button::new(RichText::new(format!("{}  Settings", icon::SETTINGS)).size(15.0))).on_hover_text("Settings (Ctrl+,)");
            if r.clicked() {
                app.navigate(Page::Settings);
            }
            let mb = app.monitor.current_mb();
            let target = app.cfg.perf.ram_target_mb as f32;
            let c = if mb > target { widgets::impact_color(crate::features::Impact::VeryHigh) } else { ui.visuals().weak_text_color() };
            let r = ui.add(egui::Label::new(RichText::new(format!("{mb:.0} MB")).small().color(c)).sense(egui::Sense::click()));
            if r.on_hover_text("Private working set — click for the Performance dashboard").clicked() {
                app.settings.category = crate::features::Category::Performance;
                app.navigate(Page::Settings);
            }
        });
        let jobs: Vec<super::jobs::JobInfo> = app.jobs.active.lock().unwrap().values().cloned().collect();
        for j in jobs.iter().take(3) {
            ui.horizontal(|ui| {
                ui.spinner();
                let p = j.progress.filter(|(_, t)| *t > 0).map(|(d, t)| format!(" {d}/{t}")).unwrap_or_default();
                ui.label(RichText::new(format!("{}{p}", j.label)).small().weak());
            });
        }
    });
}

fn player_bar(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let pos = app.engine.position_ms();
    let dur = st.duration_ms;
    let layout = app.cfg.appearance.player_bar_layout;
    let art_size = if layout == PlayerBarLayout::Compact { 40.0 } else { (ui.available_height() - 6.0).clamp(40.0, 72.0) };
    ui.horizontal_centered(|ui| {
        // left: track info
        let left_w = if layout == PlayerBarLayout::Centered { ui.available_width() * 0.28 } else { ui.available_width() * 0.30 };
        ui.allocate_ui_with_layout(egui::vec2(left_w, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if let Some(t) = &st.current {
                let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
                if widgets::artwork(ui, tex.as_ref(), art_size, 6).clicked() {
                    app.navigate(Page::NowPlaying);
                }
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(&t.title).strong()).truncate());
                    ui.horizontal(|ui| {
                        widgets::provider_badge(ui, t.provider);
                        ui.add(egui::Label::new(RichText::new(t.artist_line()).weak()).truncate());
                    });
                });
            } else {
                widgets::artwork(ui, None, art_size, 6);
                ui.label(RichText::new("Nothing playing").weak());
            }
        });
        // center: controls + progress
        let center_w = ui.available_width() * if layout == PlayerBarLayout::Centered { 0.62 } else { 0.55 };
        ui.allocate_ui_with_layout(egui::vec2(center_w, ui.available_height()), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.horizontal(|ui| {
                let w = 5.0 * 34.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                let accent = app.accent();
                let sh = ui.add(egui::Button::new(RichText::new(icon::SHUFFLE).color(if st.shuffle { accent } else { ui.visuals().weak_text_color() })).frame(false)).on_hover_text("Shuffle");
                if sh.clicked() {
                    app.engine.send(Cmd::SetShuffle(!st.shuffle));
                    app.cfg.playback.shuffle = !st.shuffle;
                    app.mark_dirty();
                }
                if ui.add(egui::Button::new(RichText::new(icon::PREV).size(18.0)).frame(false)).on_hover_text("Previous").clicked() {
                    app.engine.send(Cmd::Prev);
                }
                let pi = if st.playing { icon::PAUSE } else { icon::PLAY };
                if ui.add(egui::Button::new(RichText::new(pi).size(22.0).color(accent)).corner_radius(18)).on_hover_text("Play / Pause (Space)").clicked() {
                    app.engine.send(Cmd::Toggle);
                }
                if ui.add(egui::Button::new(RichText::new(icon::NEXT).size(18.0)).frame(false)).on_hover_text("Next").clicked() {
                    app.engine.send(Cmd::Next);
                }
                let (ri, rc) = match st.repeat {
                    RepeatMode::Off => (icon::REPEAT, ui.visuals().weak_text_color()),
                    RepeatMode::All => (icon::REPEAT, accent),
                    RepeatMode::One => (icon::REPEAT_ONE, accent),
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
            if layout != PlayerBarLayout::Compact {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_ms(pos)).small().monospace());
                    let mut p = pos as f32;
                    let w = ui.available_width() - 48.0;
                    ui.spacing_mut().slider_width = w.max(60.0);
                    let r = ui.add_enabled(dur > 0 && !st.external, egui::Slider::new(&mut p, 0.0..=(dur.max(1) as f32)).show_value(false).trailing_fill(true));
                    if r.drag_stopped() || (r.clicked() && !r.dragged()) {
                        app.engine.send(Cmd::Seek(p as u64));
                    }
                    ui.label(RichText::new(fmt_ms(dur)).small().monospace());
                });
            }
            if st.external {
                widgets::note(ui, "Playing in YouTube Music in your browser.");
            } else if let Some(e) = &st.error {
                ui.label(RichText::new(e).small().color(Color32::from_rgb(248, 113, 113)));
            }
        });
        // right: extras
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.cfg.active("quick_settings") {
                ui.menu_button(icon::QUICK, |ui| quick_settings(app, ui)).response.on_hover_text("Quick settings");
            }
            if app.cfg.active("mini_player") && widgets::small_button(ui, icon::MINI, "Mini-player").clicked() {
                app.do_action(crate::shortcuts::Action::ToggleMiniPlayer, &ui.ctx().clone());
            }
            if widgets::small_button(ui, icon::QUEUE, "Queue").clicked() {
                app.navigate(Page::Queue);
            }
            if app.cfg.active("lyrics") && widgets::small_button(ui, icon::LYRICS, "Lyrics / Now Playing").clicked() {
                app.navigate(Page::NowPlaying);
            }
            let mut v = st.volume;
            ui.spacing_mut().slider_width = 90.0;
            if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false)).changed() {
                app.engine.send(Cmd::SetVolume(v));
                app.cfg.playback.volume = v;
                app.mark_dirty();
            }
            if widgets::small_button(ui, if st.muted { icon::MUTE } else { icon::VOL }, "Mute").clicked() {
                app.engine.send(Cmd::ToggleMute);
                app.cfg.playback.muted = !st.muted;
                app.mark_dirty();
            }
            if app.cfg.active("vis_in_bar") && ui.available_width() > 120.0 {
                let accent = app.accent();
                if let Some((bars, _)) = app.vis_data() {
                    let bars = bars.to_vec();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 28.0), egui::Sense::hover());
                    let n = bars.len().min(24).max(1);
                    let w = rect.width() / n as f32;
                    for i in 0..n {
                        let v = bars[i * bars.len() / n];
                        let h = (v * rect.height()).max(1.0);
                        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left() + i as f32 * w, rect.bottom() - h), egui::pos2(rect.left() + (i as f32 + 0.8) * w, rect.bottom())), 1, accent);
                    }
                }
            }
        });
    });
}

fn quick_settings(app: &mut AquaApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    ui.label(RichText::new("Quick settings").strong());
    for id in ["lyrics", "visualizer", "bg_album_art", "bg_blur", "bg_animated", "eq", "discord", "overlay"] {
        if let Some(f) = crate::features::get(id) {
            let mut v = app.cfg.on(id);
            if ui.checkbox(&mut v, f.name).changed() {
                app.request_toggle(id, v);
            }
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        for p in [crate::features::Preset::Minimal, crate::features::Preset::Balanced, crate::features::Preset::Enhanced] {
            if ui.selectable_label(app.cfg.preset == p, p.label()).clicked() {
                app.cfg.apply_preset(p);
                app.mark_dirty();
            }
        }
    });
    if ui.button("Disable all visual customizations").clicked() {
        app.cfg.disable_all_visual();
        app.mark_dirty();
    }
    if ui.button("All settings…").clicked() {
        app.navigate(Page::Settings);
        ui.close();
    }
}

fn back_button(app: &mut AquaApp, ui: &mut Ui) {
    if !app.back.is_empty() && ui.button("← Back").clicked() {
        app.go_back();
    }
}

// ----------------------------------------------------------------------------
// Track lists

pub enum RowAction {
    Play(usize),
    Next(Track),
    Queue(Track),
    Like(Track),
    Open(Track),
    Alternatives(Track),
    GoAlbum(Track),
}

/// Virtualized track list: only visible rows are laid out.
pub fn track_list(app: &mut AquaApp, ui: &mut Ui, tracks: &[Track], id_salt: &str) -> Option<RowAction> {
    let row_h = if app.cfg.active("artwork_in_lists") { 44.0 } else { 30.0 };
    let mut act = None;
    let cur_key = app.state.current.as_ref().map(|t| t.key());
    let thumbs = app.cfg.active("artwork_in_lists");
    let prefetch = app.cfg.active("artwork_prefetch");
    let accent = app.accent();
    egui::ScrollArea::vertical().id_salt(id_salt).auto_shrink([false, false]).show_rows(ui, row_h, tracks.len(), |ui, range| {
        if prefetch && thumbs {
            let mut a = app.artwork.lock().unwrap();
            for t in tracks.iter().skip(range.end).take(15) {
                a.prefetch(&t.artwork_url);
            }
        }
        for i in range {
            let t = &tracks[i];
            let playing = cur_key.as_deref() == Some(&t.key());
            let r = ui
                .horizontal(|ui| {
                    ui.set_height(row_h);
                    ui.add_sized([36.0, row_h], egui::Label::new(RichText::new(if playing { icon::NOW.to_string() } else { (i + 1).to_string() }).weak()));
                    if thumbs {
                        let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
                        widgets::artwork(ui, tex.as_ref(), row_h - 6.0, 4);
                    }
                    let w = ui.available_width();
                    ui.allocate_ui_with_layout(egui::vec2(w * 0.45, row_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                        let mut title = RichText::new(&t.title);
                        if playing {
                            title = title.color(accent).strong();
                        }
                        if t.unavailable {
                            title = title.strikethrough().weak();
                        }
                        ui.add(egui::Label::new(title).truncate());
                        ui.add(egui::Label::new(RichText::new(t.artist_line()).small().weak()).truncate());
                    });
                    ui.allocate_ui_with_layout(egui::vec2(w * 0.30, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&t.album).weak()).truncate());
                    });
                    widgets::provider_badge(ui, t.provider);
                    if t.explicit {
                        ui.label(RichText::new("E").small().weak()).on_hover_text("Explicit");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button(icon::MORE, |ui| {
                            if ui.button("Play next").clicked() {
                                act = Some(RowAction::Next(t.clone()));
                                ui.close();
                            }
                            if ui.button("Add to queue").clicked() {
                                act = Some(RowAction::Queue(t.clone()));
                                ui.close();
                            }
                            if t.provider != ProviderKind::Local && ui.button(format!("{} Like", icon::HEART)).clicked() {
                                act = Some(RowAction::Like(t.clone()));
                                ui.close();
                            }
                            if !t.album_id.is_empty() && t.provider == ProviderKind::Spotify && ui.button("Go to album").clicked() {
                                act = Some(RowAction::GoAlbum(t.clone()));
                                ui.close();
                            }
                            if t.provider != ProviderKind::Local && ui.button("Find on the other service…").clicked() {
                                act = Some(RowAction::Alternatives(t.clone()));
                                ui.close();
                            }
                            if t.provider != ProviderKind::Local && ui.button(format!("Open in browser {}", icon::EXTERNAL)).clicked() {
                                act = Some(RowAction::Open(t.clone()));
                                ui.close();
                            }
                        });
                        ui.label(RichText::new(fmt_ms(t.duration_ms)).monospace().weak());
                    });
                })
                .response;
            let r = r.interact(egui::Sense::click());
            if r.double_clicked() {
                act = Some(RowAction::Play(i));
            }
            if r.hovered() && app.cfg.active("hover_effects") {
                ui.painter().rect_filled(r.rect, 4, accent.gamma_multiply(0.06));
            }
        }
    });
    act
}

pub fn handle_row_action(app: &mut AquaApp, tracks: &[Track], a: RowAction) {
    match a {
        RowAction::Play(i) => app.play(tracks.to_vec(), i),
        RowAction::Next(t) => app.engine.send(Cmd::PlayNext(vec![t])),
        RowAction::Queue(t) => {
            app.engine.send(Cmd::Enqueue(vec![t.clone()]));
            app.toast(format!("Queued “{}”", t.title), false);
        }
        RowAction::Like(t) => app.like(&t),
        RowAction::Open(t) => {
            if let Some(u) = t.web_url(app.cfg.services.youtube_open_in) {
                let _ = open::that_detached(u);
            }
        }
        RowAction::Alternatives(t) => app.find_alternatives(&t),
        RowAction::GoAlbum(t) => app.navigate(Page::Album(Album {
            provider: t.provider,
            id: t.album_id.clone(),
            name: t.album.clone(),
            artists: t.artists.clone(),
            artwork_url: t.artwork_url.clone(),
            ..Default::default()
        })),
    }
}

// ----------------------------------------------------------------------------
// Pages

fn home(app: &mut AquaApp, ui: &mut Ui) {
    let hour = time::OffsetDateTime::now_local().map(|t| t.hour()).unwrap_or(12);
    let greet = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    widgets::heading(ui, greet);
    let any_account = app.spotify.is_signed_in() || app.youtube.is_signed_in();
    let has_local = !app.cfg.services.local_folders.is_empty();
    if !any_account && !has_local {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new("Welcome to Aqua Glass").strong().size(17.0));
            ui.label("Connect a music service to see your playlists and library. Nothing has to be recreated: your existing Spotify playlists, liked songs, saved albums and followed artists are imported automatically.");
            ui.horizontal(|ui| {
                if app.cfg.services.spotify_enabled && ui.button("Connect Spotify").clicked() {
                    app.spotify_login();
                }
                if app.cfg.services.youtube_enabled && ui.button("Set up YouTube Music…").clicked() {
                    app.settings.category = crate::features::Category::Services;
                    app.navigate(Page::Settings);
                }
                if ui.button("Add a local music folder…").clicked() {
                    app.settings.category = crate::features::Category::Services;
                    app.navigate(Page::Settings);
                }
            });
            widgets::note(ui, "Spotify playback requires Spotify Premium. Library import, sync and transfer work with any account.");
        });
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let recent = app.library.lock().unwrap().lib(ProviderKind::Spotify).recent.clone();
        if !recent.is_empty() && app.filter.allows(ProviderKind::Spotify) {
            widgets::section(ui, "Recently played");
            let mut act = None;
            ui.horizontal_wrapped(|ui| {
                for (i, t) in recent.iter().take(12).enumerate() {
                    if card(app, ui, &t.artwork_url, &t.title, &t.artist_line(), t.provider).clicked() {
                        act = Some(i);
                    }
                }
            });
            if let Some(i) = act {
                app.play(recent.clone(), i);
            }
        }
        widgets::section(ui, "Your playlists");
        let lists = all_playlists(app);
        if lists.is_empty() {
            widgets::note(ui, "No playlists yet — sign in or sync from Settings → Library & Synchronization.");
        }
        let mut open = None;
        ui.horizontal_wrapped(|ui| {
            for p in lists.iter().take(24) {
                let sub = format!("{} · {} tracks", p.provider.label(), p.track_count);
                if card(app, ui, &p.artwork_url, &p.name, &sub, p.provider).clicked() {
                    open = Some(Page::Playlist(p.provider, p.id.clone()));
                }
            }
        });
        if let Some(p) = open {
            app.navigate(p);
        }
        let albums: Vec<Album> = {
            let mut s = app.library.lock().unwrap();
            s.libs.values().flat_map(|l| l.albums.iter().cloned()).filter(|a| app.filter.allows(a.provider)).take(12).collect()
        };
        if !albums.is_empty() {
            widgets::section(ui, "Saved albums");
            let mut open = None;
            ui.horizontal_wrapped(|ui| {
                for a in &albums {
                    if card(app, ui, &a.artwork_url, &a.name, &a.artists.join(", "), a.provider).clicked() {
                        open = Some(Page::Album(a.clone()));
                    }
                }
            });
            if let Some(p) = open {
                app.navigate(p);
            }
        }
    });
}

fn card(app: &mut AquaApp, ui: &mut Ui, art: &str, title: &str, sub: &str, p: ProviderKind) -> egui::Response {
    let size = 140.0;
    let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(art, Kind::Cover) } else { None };
    ui.allocate_ui(egui::vec2(size, size + 44.0), |ui| {
        ui.vertical(|ui| {
            let r = widgets::artwork(ui, tex.as_ref(), size, 8);
            ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
            ui.horizontal(|ui| {
                widgets::provider_badge(ui, p);
                ui.add(egui::Label::new(RichText::new(sub).small().weak()).truncate());
            });
            r
        })
        .inner
    })
    .inner
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn all_playlists(app: &mut AquaApp) -> Vec<Playlist> {
    let s = app.library.lock().unwrap();
    s.libs
        .iter()
        .filter(|(p, _)| app.filter.allows(**p) && app.provider_enabled(**p))
        .flat_map(|(_, l)| l.playlists.iter().map(|e| e.meta.clone()))
        .collect()
}

fn search(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Search");
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut app.search.query).hint_text("Songs, artists, albums, playlists…").desired_width(420.0));
        if app.search.focus {
            r.request_focus();
            app.search.focus = false;
        }
        if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button("Search").clicked() {
            app.run_search();
        }
        if app.search.pending > 0 {
            ui.spinner();
        }
    });
    widgets::note(ui, "Searches the services shown in the sidebar filter. YouTube searches use 100 of your 10,000 daily Data-API quota units.");
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if !app.search.library_hits.is_empty() {
            widgets::section(ui, "In your library");
            let hits = app.search.library_hits.clone();
            simple_tracks(app, ui, &hits, 8);
        }
        let mut order = vec![ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local];
        order.sort_by_key(|p| if *p == app.cfg.playback.preferred_provider { 0 } else { 1 });
        for p in order {
            let Some(r) = app.search.results.get(&p).cloned() else { continue };
            widgets::section(ui, p.label());
            match r {
                Err(e) => widgets::error(ui, &e),
                Ok(res) => {
                    if res.tracks.is_empty() && res.albums.is_empty() && res.artists.is_empty() && res.playlists.is_empty() {
                        widgets::note(ui, "No results.");
                    }
                    simple_tracks(app, ui, &res.tracks, 15);
                    if !res.albums.is_empty() {
                        ui.label(RichText::new("Albums").strong());
                        let mut open = None;
                        ui.horizontal_wrapped(|ui| {
                            for a in &res.albums {
                                if card(app, ui, &a.artwork_url, &a.name, &a.artists.join(", "), a.provider).clicked() {
                                    open = Some(Page::Album(a.clone()));
                                }
                            }
                        });
                        if let Some(p) = open {
                            app.navigate(p);
                        }
                    }
                    if !res.artists.is_empty() {
                        ui.label(RichText::new("Artists").strong());
                        let mut open = None;
                        ui.horizontal_wrapped(|ui| {
                            for a in &res.artists {
                                if card(app, ui, &a.artwork_url, &a.name, "Artist", a.provider).clicked() {
                                    open = Some(Page::Artist(a.clone()));
                                }
                            }
                        });
                        if let Some(p) = open {
                            app.navigate(p);
                        }
                    }
                    if !res.playlists.is_empty() {
                        ui.label(RichText::new("Playlists").strong());
                        let mut open = None;
                        ui.horizontal_wrapped(|ui| {
                            for pl in &res.playlists {
                                if card(app, ui, &pl.artwork_url, &pl.name, &pl.owner, pl.provider).clicked() {
                                    open = Some(Page::Playlist(pl.provider, pl.id.clone()));
                                }
                            }
                        });
                        if let Some(p) = open {
                            app.navigate(p);
                        }
                    }
                }
            }
        }
    });
}

/// Non-virtualized short track list (search results).
fn simple_tracks(app: &mut AquaApp, ui: &mut Ui, tracks: &[Track], max: usize) {
    let mut act = None;
    for (i, t) in tracks.iter().take(max).enumerate() {
        ui.horizontal(|ui| {
            if ui.button(icon::PLAY).on_hover_text("Play").clicked() {
                act = Some(RowAction::Play(i));
            }
            if ui.button("+").on_hover_text("Add to queue").clicked() {
                act = Some(RowAction::Queue(t.clone()));
            }
            widgets::provider_badge(ui, t.provider);
            ui.add(egui::Label::new(RichText::new(&t.title).strong()).truncate());
            ui.add(egui::Label::new(RichText::new(format!("{} · {}", t.artist_line(), fmt_ms(t.duration_ms))).weak()).truncate());
            if t.provider != ProviderKind::Local && widgets::small_button(ui, "⇄", "Find this track on the other service").clicked() {
                act = Some(RowAction::Alternatives(t.clone()));
            }
        });
    }
    if let Some(a) = act {
        // play a single search result (not the whole result list)
        let a = match a {
            RowAction::Play(i) => {
                app.play(vec![tracks[i].clone()], 0);
                return;
            }
            a => a,
        };
        handle_row_action(app, tracks, a);
    }
}

fn library(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Library");
    for p in [ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local] {
        if !app.filter.allows(p) || !app.provider_enabled(p) {
            continue;
        }
        let (n_pl, n_imp, last, err, n_alb, n_art, conflicts) = {
            let mut s = app.library.lock().unwrap();
            let l = s.lib(p);
            (l.playlists.len(), l.playlists.iter().filter(|e| e.imported).count(), l.last_sync, l.last_error.clone(), l.albums.len(), l.artists.len(), l.conflicts.len())
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::provider_badge(ui, p);
                ui.label(RichText::new(p.label()).strong().size(16.0));
                let busy = app.jobs.is_running(&format!("Sync {}", p.label())) || (p == ProviderKind::Local && app.local_scanning);
                if busy {
                    ui.spinner();
                } else if ui.button(format!("{} {}", icon::SYNC, if p == ProviderKind::Local { "Rescan" } else { "Sync Now" })).clicked() {
                    app.sync_now(p, false);
                }
                if p != ProviderKind::Local && ui.button("Import all playlists").on_hover_text("Cache every playlist locally and keep it in sync").clicked() {
                    {
                        let mut s = app.library.lock().unwrap();
                        for e in s.lib(p).playlists.iter_mut() {
                            if !e.imported {
                                e.imported = true;
                                e.meta.snapshot.clear();
                            }
                        }
                    }
                    app.sync_now(p, true);
                }
            });
            ui.label(format!("{n_pl} playlists ({n_imp} imported) · {n_alb} albums · {n_art} artists · last sync {}", crate::util::fmt_unix(last)));
            if !err.is_empty() {
                widgets::error(ui, &err);
            }
            if conflicts > 0 {
                widgets::warn(ui, &format!("{conflicts} item(s) need review — Settings → Library & Synchronization"));
            }
        });
    }
    ui.add_space(6.0);
    widgets::section(ui, "Playlists");
    widgets::note(ui, "Tick “Imported” to cache a playlist locally and keep it synced. Un-ticked playlists are read live from the service when opened.");
    let lists: Vec<(ProviderKind, crate::library::PlaylistEntry)> = {
        let s = app.library.lock().unwrap();
        s.libs.iter().filter(|(p, _)| app.filter.allows(**p) && app.provider_enabled(**p)).flat_map(|(p, l)| l.playlists.iter().map(|e| (*p, e.clone()))).collect()
    };
    let mut changed: Option<(ProviderKind, String, bool)> = None;
    let mut open = None;
    let mut remove = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 28.0, lists.len(), |ui, range| {
        for (p, e) in &lists[range] {
            ui.horizontal(|ui| {
                let mut imp = e.imported;
                if *p != ProviderKind::Local && ui.checkbox(&mut imp, "Imported").changed() {
                    changed = Some((*p, e.meta.id.clone(), imp));
                }
                widgets::provider_badge(ui, *p);
                if ui.link(&e.meta.name).clicked() {
                    open = Some(Page::Playlist(*p, e.meta.id.clone()));
                }
                ui.label(RichText::new(format!("{} tracks · {}", e.meta.track_count, if e.meta.owned_by_user { "yours" } else { &e.meta.owner })).weak());
                if e.removed_upstream {
                    widgets::warn(ui, "removed on service");
                    if ui.small_button("Remove local copy").clicked() {
                        remove = Some((*p, e.meta.id.clone(), e.meta.name.clone()));
                    }
                }
                if !e.meta.items_error.is_empty() {
                    ui.label(RichText::new("⚠").color(Color32::from_rgb(251, 191, 36))).on_hover_text(&e.meta.items_error);
                }
            });
        }
    });
    if let Some((p, id, v)) = changed {
        {
            let mut s = app.library.lock().unwrap();
            if let Some(e) = s.lib(p).playlist_mut(&id) {
                e.imported = v;
                e.meta.snapshot.clear();
            }
            s.save_index(p);
        }
        if v {
            app.sync_now(p, false);
        } else {
            app.library.lock().unwrap().remove_playlist_cache_only(p, &id);
        }
    }
    if let Some(p) = open {
        app.navigate(p);
    }
    if let Some((p, id, name)) = remove {
        app.confirm = Some(Confirm::RemoveImported(p, id, name));
    }
}

fn playlists_page(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Playlists");
    ui.add(egui::TextEdit::singleline(&mut app.view.filter).hint_text("Filter…").desired_width(260.0));
    let f = app.view.filter.to_lowercase();
    let lists: Vec<Playlist> = all_playlists(app).into_iter().filter(|p| f.is_empty() || p.name.to_lowercase().contains(&f)).collect();
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 52.0, lists.len(), |ui, range| {
        for p in &lists[range] {
            let r = ui.horizontal(|ui| {
                let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(&p.artwork_url, Kind::Cover) } else { None };
                widgets::artwork(ui, tex.as_ref(), 46.0, 6);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&p.name).strong());
                    ui.horizontal(|ui| {
                        widgets::provider_badge(ui, p.provider);
                        ui.label(RichText::new(format!("{} tracks · {}", p.track_count, p.owner)).weak().small());
                    });
                });
            });
            if r.response.interact(egui::Sense::click()).clicked() {
                open = Some(Page::Playlist(p.provider, p.id.clone()));
            }
        }
    });
    if let Some(p) = open {
        app.navigate(p);
    }
}

fn artists_page(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Artists");
    let artists: Vec<Artist> = {
        let s = app.library.lock().unwrap();
        s.libs.values().flat_map(|l| l.artists.iter().cloned()).filter(|a| app.filter.allows(a.provider)).collect()
    };
    if artists.is_empty() {
        widgets::note(ui, "No followed artists imported. Enable “Import followed artists” in Settings → Library & Synchronization and sync.");
    }
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for a in &artists {
                if card(app, ui, &a.artwork_url, &a.name, &a.genres.iter().take(2).cloned().collect::<Vec<_>>().join(", "), a.provider).clicked() {
                    open = Some(Page::Artist(a.clone()));
                }
            }
        });
    });
    if let Some(p) = open {
        app.navigate(p);
    }
}

fn albums_page(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Albums");
    let albums: Vec<Album> = {
        let s = app.library.lock().unwrap();
        s.libs.values().flat_map(|l| l.albums.iter().cloned()).filter(|a| app.filter.allows(a.provider)).collect()
    };
    if albums.is_empty() {
        widgets::note(ui, "No saved albums imported yet. YouTube's API has no album objects; local albums come from file tags.");
    }
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for a in &albums {
                if card(app, ui, &a.artwork_url, &a.name, &a.artists.join(", "), a.provider).clicked() {
                    open = Some(Page::Album(a.clone()));
                }
            }
        });
    });
    if let Some(p) = open {
        app.navigate(p);
    }
}

fn playlist_header(app: &mut AquaApp, ui: &mut Ui, art: &str, title: &str, sub: &str, desc: &str, p: ProviderKind) {
    ui.horizontal(|ui| {
        let tex = app.artwork.lock().unwrap().get(art, Kind::Cover);
        widgets::artwork(ui, tex.as_ref(), 120.0, 10);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).heading().strong());
            ui.horizontal(|ui| {
                widgets::provider_badge(ui, p);
                ui.label(RichText::new(sub).weak());
            });
            if !desc.is_empty() {
                ui.add(egui::Label::new(RichText::new(desc).weak()).wrap());
            }
        });
    });
}

fn playlist_view(app: &mut AquaApp, ui: &mut Ui, p: ProviderKind, id: &str) {
    back_button(app, ui);
    let entry = app.library.lock().unwrap().lib(p).playlist(id).cloned();
    let meta = entry.as_ref().map(|e| e.meta.clone()).unwrap_or_else(|| {
        // a playlist opened from search results
        let mut m = Playlist { provider: p, id: id.to_string(), name: "Playlist".into(), ..Default::default() };
        if let Some(Ok(r)) = app.search.results.get(&p) {
            if let Some(x) = r.playlists.iter().find(|x| x.id == id) {
                m = x.clone();
            }
        }
        m
    });
    let imported = entry.as_ref().is_some_and(|e| e.imported);
    let status = if imported { format!("imported · synced {}", crate::util::fmt_unix(entry.as_ref().unwrap().last_synced)) } else { "live from service (not imported)".into() };
    playlist_header(app, ui, &meta.artwork_url, &meta.name, &format!("{} · {} tracks · {}", meta.owner, meta.track_count, status), &meta.description, p);
    let tracks = app.playlist_tracks(p, id);
    ui.horizontal(|ui| {
        let ok = matches!(tracks, Some(Ok(_)));
        if ui.add_enabled(ok, egui::Button::new(format!("{}  Play", icon::PLAY))).clicked() {
            if let Some(Ok(t)) = &tracks {
                app.engine.send(Cmd::SetShuffle(false));
                app.play(t.clone(), 0);
            }
        }
        if ui.add_enabled(ok, egui::Button::new(format!("{}  Shuffle", icon::SHUFFLE))).clicked() {
            if let Some(Ok(t)) = &tracks {
                app.engine.send(Cmd::SetShuffle(true));
                let start = rand::random_range(0..t.len().max(1));
                app.play(t.clone(), start);
            }
        }
        if p != ProviderKind::Local && entry.is_some() && !imported && ui.button("Import").on_hover_text("Cache locally and keep in sync").clicked() {
            {
                let mut s = app.library.lock().unwrap();
                if let Some(e) = s.lib(p).playlist_mut(id) {
                    e.imported = true;
                    e.meta.snapshot.clear();
                }
                s.save_index(p);
            }
            app.sync_now(p, false);
        }
        if let Some(Ok(t)) = &tracks {
            let t = t.clone();
            ui.menu_button("Export…", |ui| {
                for (label, ext) in [("JSON (full metadata)", "json"), ("CSV (spreadsheet)", "csv"), ("M3U (playlist file)", "m3u")] {
                    if ui.button(label).clicked() {
                        export_playlist(app, &meta, &t, ext);
                        ui.close();
                    }
                }
            });
            if ui.button(format!("{} Transfer…", icon::TRANSFER)).clicked() {
                app.transfer.select_source(meta.clone(), t.clone());
                app.navigate(Page::Transfer);
            }
        }
        ui.add(egui::TextEdit::singleline(&mut app.view.filter).hint_text("Filter tracks…").desired_width(180.0));
    });
    if !meta.items_error.is_empty() {
        widgets::warn(ui, &format!("Last sync could not read this playlist's tracks: {}", meta.items_error));
        if meta.items_error.contains("403") || meta.items_error.to_lowercase().contains("permitted") {
            widgets::note(ui, "Spotify restricts playlist contents for some apps (Development Mode, Feb 2026). Playlists you own or collaborate on are always readable.");
        }
    }
    ui.separator();
    match tracks {
        None => {
            ui.spinner();
        }
        Some(Err(e)) => widgets::error(ui, &e),
        Some(Ok(t)) => {
            let f = app.view.filter.to_lowercase();
            let shown: Vec<Track> = if f.is_empty() { t } else { t.into_iter().filter(|x| x.title.to_lowercase().contains(&f) || x.artist_line().to_lowercase().contains(&f) || x.album.to_lowercase().contains(&f)).collect() };
            if let Some(a) = track_list(app, ui, &shown, id) {
                handle_row_action(app, &shown, a);
            }
        }
    }
}

fn export_playlist(app: &mut AquaApp, p: &Playlist, tracks: &[Track], ext: &str) {
    let name: String = p.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let default = crate::util::data_dir().join("exports").join(format!("{}.{ext}", name.trim()));
    let path = pick_save(&default, ext).unwrap_or(default);
    let r = match ext {
        "json" => crate::transfer::export_json(p, tracks, &path),
        "csv" => crate::transfer::export_csv(tracks, &path),
        _ => crate::transfer::export_m3u(p, tracks, &path),
    };
    match r {
        Ok(()) => app.toast(format!("Exported to {}", path.display()), false),
        Err(e) => app.toast(format!("Export failed: {e}"), true),
    }
}

pub fn pick_save(default: &std::path::Path, ext: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let _ = crate::util::ensure_dir(default.parent()?);
        return rfd::FileDialog::new()
            .set_directory(default.parent()?)
            .set_file_name(default.file_name()?.to_string_lossy())
            .add_filter(ext, &[ext])
            .save_file();
    }
    #[cfg(not(windows))]
    {
        let _ = (default, ext);
        None
    }
}

pub fn pick_open(exts: &[&str]) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        return rfd::FileDialog::new().add_filter("Supported", exts).pick_file();
    }
    #[cfg(not(windows))]
    {
        let _ = exts;
        None
    }
}

pub fn pick_folder() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        return rfd::FileDialog::new().pick_folder();
    }
    #[cfg(not(windows))]
    None
}

fn album_view(app: &mut AquaApp, ui: &mut Ui, a: &Album) {
    back_button(app, ui);
    playlist_header(app, ui, &a.artwork_url, &a.name, &format!("{} · {}", a.artists.join(", "), a.release_date), "", a.provider);
    let key = format!("album:{}", a.id);
    let tracks: Option<Result<Vec<Track>, String>> = match a.provider {
        ProviderKind::Local => {
            let all = app.library.lock().unwrap().tracks(ProviderKind::Local, crate::library::LOCAL_ALL_ID).cloned().unwrap_or_default();
            let mut v: Vec<Track> = all.into_iter().filter(|t| t.album_id == a.id).collect();
            v.sort_by_key(|t| t.track_number);
            Some(Ok(v))
        }
        _ => {
            if let Some(r) = app.view.live.get(&key) {
                Some(r.clone())
            } else {
                if !app.view.loading.contains(&key) && a.provider == ProviderKind::Spotify {
                    app.view.loading.insert(key.clone());
                    let api = app.spotify.clone();
                    let al = a.clone();
                    app.jobs.spawn("Load album", move |h| {
                        let r = api.album_tracks(&al).map_err(|e| e.to_string());
                        h.send(super::jobs::Msg::AlbumTracks { album: al, res: r });
                    });
                }
                None
            }
        }
    };
    ui.separator();
    match tracks {
        None => {
            ui.spinner();
        }
        Some(Err(e)) => widgets::error(ui, &e),
        Some(Ok(t)) => {
            if ui.button(format!("{}  Play album", icon::PLAY)).clicked() {
                app.play(t.clone(), 0);
            }
            if let Some(act) = track_list(app, ui, &t, &key) {
                handle_row_action(app, &t, act);
            }
        }
    }
}

fn artist_view(app: &mut AquaApp, ui: &mut Ui, a: &Artist) {
    back_button(app, ui);
    playlist_header(app, ui, &a.artwork_url, &a.name, &a.genres.join(", "), "", a.provider);
    if a.provider != ProviderKind::Spotify {
        widgets::note(ui, "Artist pages are available for Spotify artists. YouTube's Data API exposes channels, not artist discographies.");
        return;
    }
    let albums = app.view.artist_albums.get(&a.id).cloned();
    if albums.is_none() && !app.view.loading.contains(&format!("artist:{}", a.id)) {
        app.view.loading.insert(format!("artist:{}", a.id));
        let api = app.spotify.clone();
        let ar = a.clone();
        app.jobs.spawn("Load artist", move |h| {
            let r = api.artist_albums(&ar.id).map_err(|e| e.to_string());
            h.send(super::jobs::Msg::ArtistAlbums { artist: ar, res: r });
        });
    }
    ui.separator();
    match albums {
        None => {
            ui.spinner();
        }
        Some(Err(e)) => widgets::error(ui, &e),
        Some(Ok(list)) => {
            let mut open = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for al in &list {
                        if card(app, ui, &al.artwork_url, &al.name, &al.release_date, al.provider).clicked() {
                            open = Some(Page::Album(al.clone()));
                        }
                    }
                });
            });
            if let Some(p) = open {
                app.navigate(p);
            }
        }
    }
}

fn now_playing(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let Some(t) = st.current.clone() else {
        widgets::heading(ui, "Now Playing");
        ui.label("Nothing is playing. Pick something from your library or search.");
        return;
    };
    let lyrics_on = app.cfg.active("lyrics");
    if app.lyrics.fullscreen && lyrics_on {
        if ui.button("✖ Exit fullscreen lyrics").clicked() {
            app.lyrics.fullscreen = false;
        }
        lyrics_panel(app, ui, true);
        return;
    }
    ui.columns(if lyrics_on { 2 } else { 1 }, |cols| {
        let ui = &mut cols[0];
        ui.vertical_centered(|ui| {
            let size = (ui.available_width() * 0.8).clamp(160.0, 380.0);
            let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
            widgets::artwork(ui, tex.as_ref(), size, 14);
            ui.add_space(8.0);
            ui.label(RichText::new(&t.title).heading().strong());
            ui.label(RichText::new(t.artist_line()).size(16.0));
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() / 2.0 - 80.0).max(0.0));
                widgets::provider_badge(ui, t.provider);
                ui.label(RichText::new(&t.album).weak());
            });
            if st.external {
                widgets::note(ui, "This YouTube track plays in YouTube Music in your browser — Google provides no API for third-party audio playback.");
                if ui.button(format!("Open again {}", icon::EXTERNAL)).clicked() {
                    if let Some(u) = t.web_url(app.cfg.services.youtube_open_in) {
                        let _ = open::that_detached(u);
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() / 2.0 - 120.0).max(0.0));
                if t.provider != ProviderKind::Local && ui.button(format!("{} Like", icon::HEART)).clicked() {
                    app.like(&t);
                }
                if t.provider != ProviderKind::Local && ui.button("⇄ Other version…").clicked() {
                    app.find_alternatives(&t);
                }
                if lyrics_on && app.cfg.active("lyrics_fullscreen") && ui.button("⛶ Fullscreen lyrics").clicked() {
                    app.lyrics.fullscreen = true;
                }
            });
            ui.add_space(10.0);
            visualizer(app, ui);
        });
        if lyrics_on {
            lyrics_panel(app, &mut cols[1], false);
        }
    });
}

pub fn visualizer(app: &mut AquaApp, ui: &mut Ui) {
    if !app.cfg.active("visualizer") {
        return;
    }
    if app.state.external {
        widgets::note(ui, "The visualizer reacts to audio played inside Aqua Glass (Spotify, local files).");
        return;
    }
    let v = app.cfg.visualizer.clone();
    let (ca, cb) = if app.cfg.active("vis_artwork_colors") {
        let url = app.state.current.as_ref().map(|t| t.artwork_url.clone()).unwrap_or_default();
        let d = app.artwork.lock().unwrap().dominant(&url).unwrap_or(v.color_a);
        (d, crate::theme::lerp_rgb(d, [255, 255, 255], 0.35))
    } else {
        (v.color_a, v.color_b)
    };
    let Some((bars, wave)) = app.vis_data() else { return };
    let (bars, wave) = (bars.to_vec(), wave.to_vec());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, v.height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let col = |t: f32| crate::theme::c(crate::theme::lerp_rgb(ca, cb, t));
    match v.mode {
        crate::config::VisMode::Bars => {
            let n = bars.len().max(1);
            let bw = rect.width() / n as f32;
            for (i, b) in bars.iter().enumerate() {
                let h = (b * rect.height()).max(2.0);
                let r = egui::Rect::from_min_max(egui::pos2(rect.left() + i as f32 * bw + 1.0, rect.bottom() - h), egui::pos2(rect.left() + (i as f32 + 1.0) * bw - 1.0, rect.bottom()));
                p.rect_filled(r, 2, col(i as f32 / n as f32));
            }
        }
        crate::config::VisMode::Waveform => {
            if wave.len() > 1 {
                let pts: Vec<egui::Pos2> = wave
                    .iter()
                    .enumerate()
                    .map(|(i, s)| egui::pos2(rect.left() + rect.width() * i as f32 / (wave.len() - 1) as f32, rect.center().y - s * rect.height() * 0.9 * v.sensitivity))
                    .collect();
                p.add(egui::Shape::line(pts, egui::Stroke::new(2.0, col(0.3))));
            }
        }
        crate::config::VisMode::Circular => {
            let c = rect.center();
            let r0 = rect.height() * 0.25;
            let n = bars.len().max(1);
            for (i, b) in bars.iter().enumerate() {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                let dir = egui::vec2(a.cos(), a.sin());
                p.line_segment([c + dir * r0, c + dir * (r0 + b * rect.height() * 0.25)], egui::Stroke::new(3.0, col(i as f32 / n as f32)));
            }
        }
        crate::config::VisMode::Minimal => {
            let level = bars.iter().take(bars.len() / 3 + 1).cloned().fold(0.0, f32::max);
            let r = rect.height() * (0.15 + 0.3 * level);
            p.circle_filled(rect.center(), r, col(level).gamma_multiply(0.6));
            p.circle_stroke(rect.center(), r * 1.2, egui::Stroke::new(1.5, col(1.0 - level)));
        }
    }
}

pub fn lyrics_panel(app: &mut AquaApp, ui: &mut Ui, fullscreen: bool) {
    let cfg = app.cfg.lyrics.clone();
    let pos = app.engine.position_ms() as i64 + cfg.offset_ms as i64;
    let synced = app.cfg.active("lyrics_synced");
    let words = app.cfg.active("lyrics_word");
    let accent = crate::theme::c(cfg.highlight);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Lyrics").strong());
        ui.label(RichText::new("from LRCLIB").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+250ms").on_hover_text("Lyrics later").clicked() {
                app.cfg.lyrics.offset_ms += 250;
                app.mark_dirty();
            }
            ui.label(RichText::new(format!("offset {} ms", app.cfg.lyrics.offset_ms)).small());
            if ui.small_button("−250ms").on_hover_text("Lyrics earlier").clicked() {
                app.cfg.lyrics.offset_ms -= 250;
                app.mark_dirty();
            }
        });
    });
    let frame = egui::Frame::new().fill(Color32::from_black_alpha((cfg.bg_opacity * 255.0) as u8)).corner_radius(8).inner_margin(10);
    frame.show(ui, |ui| match &app.lyrics.current {
        None => {
            ui.spinner();
        }
        Some(Err(e)) => {
            widgets::note(ui, &format!("Lyrics unavailable: {e}"));
            if ui.button("Retry").clicked() {
                app.lyrics.key.clear();
                app.lyrics.cache.clear();
            }
        }
        Some(Ok(l)) => {
            use crate::services::lyrics::Lyrics;
            let align = match cfg.align {
                crate::config::Align::Left => egui::Align::Min,
                crate::config::Align::Center => egui::Align::Center,
                crate::config::Align::Right => egui::Align::Max,
            };
            let size = if fullscreen { cfg.font_size * 1.5 } else { cfg.font_size };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(align), |ui| match l {
                    Lyrics::Instrumental => {
                        ui.label(RichText::new("♪ Instrumental ♪").size(size));
                    }
                    Lyrics::NotFound => {
                        ui.label("No lyrics found for this track.");
                    }
                    Lyrics::Plain(lines) => {
                        for line in lines {
                            ui.label(RichText::new(line).size(size).color(crate::theme::c(cfg.color)));
                        }
                    }
                    Lyrics::Synced(lines) => {
                        let cur = if synced { l.current_line(pos) } else { None };
                        for (i, line) in lines.iter().enumerate() {
                            ui.add_space(cfg.line_spacing * 0.5);
                            let is_cur = Some(i) == cur;
                            let r = if is_cur && words && !line.words.is_empty() {
                                let mut job = egui::text::LayoutJob::default();
                                for w in &line.words {
                                    let c = if w.t_ms <= pos { accent } else { crate::theme::c(cfg.color) };
                                    job.append(&w.text, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(size), color: c, ..Default::default() });
                                }
                                ui.label(job)
                            } else {
                                let mut rt = RichText::new(if line.text.is_empty() { "♪" } else { &line.text }).size(size);
                                rt = if is_cur { rt.color(accent) } else { rt.color(crate::theme::c(cfg.color)) };
                                if is_cur && cfg.bold_current {
                                    rt = rt.strong();
                                }
                                ui.label(rt)
                            };
                            if is_cur {
                                r.scroll_to_me(Some(egui::Align::Center));
                            }
                            if synced && r.interact(egui::Sense::click()).clicked() {
                                app.engine.send(Cmd::Seek((line.t_ms - cfg.offset_ms as i64).max(0) as u64));
                            }
                            ui.add_space(cfg.line_spacing * 0.5);
                        }
                    }
                });
            });
        }
    });
}

fn queue(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Queue");
    let st = app.state.clone();
    ui.horizontal(|ui| {
        ui.label(format!("{} tracks", st.queue.len()));
        if ui.button("Clear upcoming").clicked() {
            app.engine.send(Cmd::ClearQueue);
        }
        let mut sh = st.shuffle;
        if ui.checkbox(&mut sh, "Shuffle").changed() {
            app.engine.send(Cmd::SetShuffle(sh));
        }
    });
    widgets::note(ui, "Double-click a track to jump to it. Spotify, YouTube and local tracks can be mixed in one queue.");
    ui.separator();
    let mut cmd = None;
    let n = st.queue.len();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 30.0, n, |ui, range| {
        for i in range {
            let t = &st.queue[i];
            let cur = st.queue_pos == Some(i);
            let r = ui.horizontal(|ui| {
                ui.add_sized([36.0, 24.0], egui::Label::new(RichText::new(if cur { icon::NOW.to_string() } else { (i + 1).to_string() }).weak()));
                widgets::provider_badge(ui, t.provider);
                let mut title = RichText::new(format!("{} — {}", t.title, t.artist_line()));
                if cur {
                    title = title.color(app.accent()).strong();
                }
                ui.add(egui::Label::new(title).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_button(ui, "✖", "Remove").clicked() {
                        cmd = Some(Cmd::Remove(i));
                    }
                    if i + 1 < n && widgets::small_button(ui, "▼", "Move down").clicked() {
                        cmd = Some(Cmd::Move(i, i + 1));
                    }
                    if i > 0 && widgets::small_button(ui, "▲", "Move up").clicked() {
                        cmd = Some(Cmd::Move(i, i - 1));
                    }
                    ui.label(RichText::new(fmt_ms(t.duration_ms)).monospace().weak());
                });
            });
            if r.response.interact(egui::Sense::click()).double_clicked() {
                cmd = Some(Cmd::JumpTo(i));
            }
        }
    });
    if let Some(c) = cmd {
        app.engine.send(c);
    }
}

#[allow(dead_code)]
fn liked_id() -> &'static str {
    LIKED_ID
}
