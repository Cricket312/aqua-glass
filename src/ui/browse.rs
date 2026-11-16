//! Home, Search, Discover, Now Playing and Queue pages.

use super::widgets::{self, icon};
use super::{AquaApp, Page, PlayFrom};
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::ProviderKind;
use crate::library::LIKED_ID;
use crate::model::*;
use crate::util::fmt_ms;
use eframe::egui::{self, RichText, Ui};

/// A card on the Home/Discover pages and what activating it does.
#[derive(Clone)]
enum CardAct {
    OpenPlaylist(ProviderKind, String),
    OpenAlbum(Album),
    PlayTracks(Vec<Track>, usize),
    /// Start Similarity Radio seeded by this track.
    Radio(Track),
}

#[derive(Clone)]
struct CardData {
    shape: widgets::CardShape,
    art: String,
    title: String,
    sub: String,
    provider: Option<ProviderKind>,
    open: CardAct,
    play: Option<CardAct>,
}

fn run(app: &mut AquaApp, a: CardAct) {
    match a {
        CardAct::OpenPlaylist(p, id) => app.navigate(Page::Playlist(p, id)),
        CardAct::OpenAlbum(al) => app.navigate(Page::Album(al)),
        CardAct::PlayTracks(t, i) => app.play_list(t, i, PlayFrom::Other),
        CardAct::Radio(t) => {
            if app.radio.is_some() {
                app.play_single(t, PlayFrom::Radio);
            } else {
                app.play_single(t, PlayFrom::Other);
                app.toast("Similarity Radio is off — playing just this song.", false);
            }
        }
    }
}

/// One horizontal row of cards (count adapts to the width; "Show all" when cut).
fn card_row(app: &mut AquaApp, ui: &mut Ui, title: &str, cards: &[CardData], show_all: Option<Page>) {
    if cards.is_empty() {
        return;
    }
    let wide = cards.first().is_some_and(|c| c.shape == widgets::CardShape::Wide);
    let (n, w) = widgets::grid_metrics_min(ui.available_width(), if wide { 170.0 } else { 140.0 });
    if widgets::section_header(ui, title, show_all.is_some() && cards.len() > n) {
        if let Some(p) = show_all {
            app.navigate(p);
        }
    }
    let accent = app.accent();
    let thumbs = app.cfg.active("artwork_in_lists");
    let mut act = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for c in cards.iter().take(n) {
            let tex = if thumbs { app.artwork.lock().unwrap().get(&c.art, Kind::Cover) } else { None };
            let o = widgets::media_card(ui, tex.as_ref(), &c.title, &c.sub, c.provider, w, c.shape, accent);
            if o.play {
                act = Some(c.play.clone().unwrap_or(c.open.clone()));
            } else if o.open {
                act = Some(c.open.clone());
            }
        }
    });
    if let Some(a) = act {
        run(app, a);
    }
}

struct Featured {
    kind: &'static str,
    title: String,
    desc: String,
    art: String,
    open: CardAct,
    play: Option<CardAct>,
    playlist: Option<(ProviderKind, String)>,
}

/// Hero banner: artwork background darkened by a left-to-right gradient, title,
/// description, Play pill and pager dots (no auto-rotation › no repaints).
fn featured_banner(app: &mut AquaApp, ui: &mut Ui, items: &[Featured]) {
    let idx = app.featured_idx.min(items.len() - 1);
    let f = &items[idx];
    let w = ui.available_width() - 18.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 176.0), egui::Sense::click());
    let v = ui.visuals().clone();
    let theme = app.current_theme().clone();
    let p = ui.painter_at(rect);
    let thumbs = app.cfg.active("artwork_in_lists");
    // blurred cover as the backdrop (computed small, cheap) + the sharp cover on the right
    let bg_tex = if thumbs { app.artwork.lock().unwrap().get(&f.art, Kind::Background { px: 256, blur: 18 }) } else { None };
    let cover = if thumbs { app.artwork.lock().unwrap().get(&f.art, Kind::Cover) } else { None };
    match bg_tex {
        Some(t) => {
            let ts = t.size_vec2();
            let s = (rect.width() / ts.x).max(rect.height() / ts.y);
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2((rect.width() / (ts.x * s)).min(1.0), (rect.height() / (ts.y * s)).min(1.0)));
            egui::Image::new(egui::load::SizedTexture::new(t.id(), rect.size())).uv(uv).corner_radius(16).paint_at(ui, rect);
        }
        None => {
            p.rect_filled(rect, 16, crate::theme::c(theme.surface));
        }
    }
    let bg = crate::theme::c(theme.panel);
    widgets::gradient_rounded(&p, rect, 16.0, |t| bg.gamma_multiply((0.95 - t * 0.6).clamp(0.3, 0.95)));
    p.rect_stroke(rect, 16, egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    if let Some(t) = &cover {
        let side = rect.height() - 36.0;
        let r = egui::Rect::from_min_size(egui::pos2(rect.right() - side - 28.0, rect.top() + 18.0), egui::vec2(side, side));
        egui::Image::new(egui::load::SizedTexture::new(t.id(), r.size())).corner_radius(10).paint_at(ui, r);
    }
    let text_rect = egui::Rect::from_min_max(rect.min + egui::vec2(28.0, 22.0), egui::pos2(rect.right() - rect.height() - 30.0, rect.bottom() - 22.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui2 = &mut child;
    ui2.label(RichText::new(f.kind).size(11.0).color(v.text_color()));
    ui2.add(egui::Label::new(RichText::new(&f.title).size(30.0).strong().color(v.strong_text_color())).truncate());
    ui2.add(egui::Label::new(RichText::new(&f.desc).size(14.0).color(v.text_color())).truncate());
    ui2.add_space(10.0);
    let mut act = None;
    ui2.horizontal(|ui| {
        let accent = app.accent();
        let b = egui::Button::new(RichText::new(format!("{}  Play", icon::PLAY())).size(15.0).strong().color(v.extreme_bg_color)).fill(accent).corner_radius(18).min_size(egui::vec2(96.0, 36.0));
        if ui.add(b).clicked() {
            act = f.play.clone().or(Some(f.open.clone()));
        }
        if let Some((pk, id)) = &f.playlist {
            let imported = app.library.lock().unwrap().lib(*pk).playlist(id).is_some_and(|e| e.imported);
            if *pk != ProviderKind::Local && !imported && ui.add(egui::Button::new(RichText::new("♡").size(17.0)).frame(false)).on_hover_text("Import (keep a synced copy)").clicked() {
                {
                    let mut s = app.library.lock().unwrap();
                    if let Some(e) = s.lib(*pk).playlist_mut(id) {
                        e.imported = true;
                        e.meta.snapshot.clear();
                    }
                    s.save_index(*pk);
                }
                app.sync_now(*pk, false);
            }
        }
        if ui.add(egui::Button::new(RichText::new("…").size(17.0)).frame(false)).on_hover_text("Open").clicked() {
            act = Some(f.open.clone());
        }
    });
    // pager dots
    if items.len() > 1 {
        let y = rect.bottom() - 14.0;
        let cx = text_rect.center().x;
        for i in 0..items.len() {
            let c = egui::pos2(cx - (items.len() as f32 - 1.0) * 13.0 + i as f32 * 26.0, y);
            let r = ui.interact(egui::Rect::from_center_size(c, egui::vec2(18.0, 18.0)), resp.id.with(("dot", i)), egui::Sense::click());
            p.circle_filled(c, 4.0, if i == idx { app.accent() } else { v.weak_text_color().gamma_multiply(0.6) });
            if r.clicked() {
                app.featured_idx = i;
            }
        }
    }
    if let Some(a) = act {
        if let (CardAct::OpenPlaylist(pk, id), true) = (&a, f.play.is_some()) {
            // play the playlist directly
            if let Some(Ok(t)) = app.playlist_tracks(*pk, id) {
                app.play_list(t, 0, PlayFrom::Other);
                return;
            }
        }
        run(app, a);
    } else if resp.clicked() {
        run(app, f.open.clone());
    }
}

fn pl_card(p: &Playlist) -> CardData {
    let sub = if p.is_liked { format!("{} songs", p.track_count) } else if p.owner.is_empty() { format!("{} songs", p.track_count) } else { format!("By {}", p.owner) };
    CardData { shape: widgets::CardShape::Square, art: p.artwork_url.clone(), title: p.name.clone(), sub, provider: Some(p.provider), open: CardAct::OpenPlaylist(p.provider, p.id.clone()), play: None }
}

pub fn home(app: &mut AquaApp, ui: &mut Ui) {
    let hour = time::OffsetDateTime::now_local().map(|t| t.hour()).unwrap_or(12);
    let greet = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    let any_account = app.spotify.is_signed_in();
    let has_local = !app.cfg.services.local_folders.is_empty();
    // gather data (only real data; empty sections are not shown)
    let recent: Vec<Track> = {
        let h = app.history.lock().unwrap();
        h.recent_tracks(12).into_iter().filter(|t| app.filter.allows(t.provider)).collect()
    };
    let top = app.history.lock().unwrap().top_artists(8);
    let lists: Vec<(Playlist, bool, i64)> = {
        let s = app.library.lock().unwrap();
        s.libs
            .iter()
            .filter(|(p, _)| app.filter.allows(**p) && app.provider_enabled(**p))
            .flat_map(|(_, l)| l.playlists.iter().map(|e| (e.meta.clone(), e.imported, e.last_synced)))
            .collect()
    };
    let albums: Vec<Album> = {
        let s = app.library.lock().unwrap();
        s.libs.values().flat_map(|l| l.albums.iter().cloned()).filter(|a| app.filter.allows(a.provider)).take(16).collect()
    };
    let sp_recent: Vec<Track> = if app.filter.allows(ProviderKind::Spotify) { app.library.lock().unwrap().lib(ProviderKind::Spotify).recent.clone() } else { vec![] };

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(greet).size(30.0).strong());
        if !any_account && !has_local {
            ui.add_space(8.0);
            widgets::card(ui, Some("Connect your music"), |ui| {
                ui.label("Sign in to see your playlists and library — nothing has to be recreated. Your Spotify playlists, liked songs, saved albums and followed artists are imported automatically.");
                ui.horizontal(|ui| {
                    if app.cfg.services.spotify_enabled && ui.button("Connect Spotify").clicked() {
                        app.spotify_login();
                    }
                    if ui.button("Local files…").clicked() {
                        app.settings.category = crate::features::Category::Services;
                        app.navigate(Page::Settings);
                    }
                });
                widgets::note(ui, "Spotify playback requires Premium. Import, sync and transfer work with any account.");
            });
        }
        let mut fav: Vec<Playlist> = lists.iter().filter(|(p, _, _)| p.is_liked).map(|x| x.0.clone()).collect();
        fav.extend(lists.iter().filter(|(p, _, _)| !p.is_liked && p.owned_by_user).map(|x| x.0.clone()));
        // featured banner: up to three real items from your library
        let mut featured: Vec<Featured> = Vec::new();
        let mut by_sync: Vec<&(Playlist, bool, i64)> = lists.iter().filter(|(p, _, _)| !p.is_liked && !p.artwork_url.is_empty()).collect();
        by_sync.sort_by_key(|x| -x.2);
        for (p, _, _) in by_sync.into_iter().take(2) {
            let desc = if p.description.trim().is_empty() { format!("{} songs · {}", p.track_count, p.provider.label()) } else { p.description.clone() };
            featured.push(Featured { kind: "FEATURED PLAYLIST", title: p.name.clone(), desc, art: p.artwork_url.clone(), open: CardAct::OpenPlaylist(p.provider, p.id.clone()), play: Some(CardAct::OpenPlaylist(p.provider, p.id.clone())), playlist: Some((p.provider, p.id.clone())) });
        }
        if app.radio.is_some() {
            if let Some((a, t, _)) = top.first() {
                featured.push(Featured { kind: "YOUR TOP MIX", title: format!("{a} Mix"), desc: "Similarity Radio from the artist you play most.".into(), art: t.artwork_url.clone(), open: CardAct::Radio(t.clone()), play: Some(CardAct::Radio(t.clone())), playlist: None });
            }
        }
        if !featured.is_empty() {
            ui.add_space(12.0);
            featured_banner(app, ui, &featured);
        }
        let rp: Vec<CardData> = recent
            .iter()
            .chain(sp_recent.iter())
            .scan(std::collections::HashSet::new(), |seen, t| Some(if seen.insert(t.key()) { Some(t.clone()) } else { None }))
            .flatten()
            .take(12)
            .map(|t| CardData { shape: widgets::CardShape::Square, art: t.artwork_url.clone(), title: t.title.clone(), sub: t.artist_line(), provider: Some(t.provider), open: CardAct::PlayTracks(vec![t.clone()], 0), play: None })
            .collect();
        card_row(app, ui, "Recently Played", &rp, None);
        if fav.len() > 6 {
            let c: Vec<CardData> = fav.iter().skip(6).map(pl_card).collect();
            card_row(app, ui, "Favorite Playlists", &c, Some(Page::Playlists));
        }
        let made: Vec<CardData> = lists.iter().filter(|(p, _, _)| p.provider == ProviderKind::Spotify && p.owner.eq_ignore_ascii_case("spotify")).map(|x| CardData { shape: widgets::CardShape::Wide, ..pl_card(&x.0) }).collect();
        card_row(app, ui, "Made For You", &made, Some(Page::Playlists));
        let added: Vec<CardData> = albums
            .iter()
            .map(|a| CardData { shape: widgets::CardShape::Square, art: a.artwork_url.clone(), title: a.name.clone(), sub: a.artists.join(", "), provider: Some(a.provider), open: CardAct::OpenAlbum(a.clone()), play: None })
            .collect();
        card_row(app, ui, "Recently Added", &added, Some(Page::Albums));
        if app.radio.is_some() {
            let mixes: Vec<CardData> = top
                .iter()
                .map(|(a, t, n)| CardData { shape: widgets::CardShape::Wide, art: t.artwork_url.clone(), title: format!("{a} Mix"), sub: format!("Similarity Radio · {n} plays"), provider: None, open: CardAct::Radio(t.clone()), play: None })
                .collect();
            card_row(app, ui, "Your Top Mixes", &mixes, Some(Page::Discover));
            let disc: Vec<CardData> = recent
                .iter()
                .filter(|t| !top.iter().any(|(_, x, _)| x.key() == t.key()))
                .take(8)
                .map(|t| CardData { shape: widgets::CardShape::Square, art: t.artwork_url.clone(), title: format!("Like “{}”", t.title), sub: format!("Discover from {}", t.artist_line()), provider: None, open: CardAct::Radio(t.clone()), play: None })
                .collect();
            card_row(app, ui, "Discover New Music", &disc, Some(Page::Discover));
        }
        let mut imported: Vec<&(Playlist, bool, i64)> = lists.iter().filter(|(p, imp, _)| *imp && p.provider == ProviderKind::Spotify && !p.is_liked).collect();
        imported.sort_by_key(|x| -x.2);
        let imp: Vec<CardData> = imported.iter().map(|x| pl_card(&x.0)).collect();
        card_row(app, ui, "Recently Imported Spotify Playlists", &imp, Some(Page::Library));
        let local: Vec<CardData> = lists.iter().filter(|(p, _, _)| p.provider == ProviderKind::Local).map(|x| pl_card(&x.0)).collect();
        card_row(app, ui, "Local Music", &local, Some(Page::Library));
        ui.add_space(20.0);
    });
}

// ---------------------------------------------------------------------------
// Search

pub fn search(app: &mut AquaApp, ui: &mut Ui) {
    if app.search.query.is_empty() && !app.top_search.is_empty() && app.search.results.is_empty() {
        // typed but not submitted yet
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if app.search.query.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("Search").size(24.0).strong());
            widgets::note(ui, "Type in the search box above and press Enter. Use the service chips to choose where to search.");
            if app.radio.is_some() && app.cfg.active("radio_from_search") {
                widgets::note(ui, "Playing a song from the results starts Similarity Radio automatically.");
            }
            let recent = app.history.lock().unwrap().recent_tracks(8);
            if !recent.is_empty() {
                widgets::section_header(ui, "Recently played", false);
                song_rows(app, ui, &recent);
            }
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Results for “{}”", app.search.query)).size(20.0).strong());
            if app.search.pending > 0 {
                ui.spinner();
            }
        });
        // songs from all providers, preferred provider first
        let mut order = vec![ProviderKind::Spotify, ProviderKind::Local];
        order.sort_by_key(|p| if *p == app.cfg.playback.preferred_provider { 0 } else { 1 });
        let mut songs: Vec<Track> = Vec::new();
        let mut albums: Vec<Album> = Vec::new();
        let mut artists: Vec<Artist> = Vec::new();
        let mut playlists: Vec<Playlist> = Vec::new();
        let mut errors = Vec::new();
        for p in &order {
            match app.search.results.get(p) {
                Some(Ok(r)) => {
                    songs.extend(r.tracks.iter().cloned());
                    albums.extend(r.albums.iter().cloned());
                    artists.extend(r.artists.iter().cloned());
                    playlists.extend(r.playlists.iter().cloned());
                }
                Some(Err(e)) => errors.push(format!("{}: {e}", p.label())),
                None => {}
            }
        }
        for e in errors {
            widgets::error(ui, &e);
        }
        if !app.search.library_hits.is_empty() {
            widgets::section_header(ui, "In your library", false);
            let hits = app.search.library_hits.iter().take(5).cloned().collect::<Vec<_>>();
            song_rows(app, ui, &hits);
        }
        if !songs.is_empty() {
            widgets::section_header(ui, "Songs", false);
            let s = songs.into_iter().take(20).collect::<Vec<_>>();
            song_rows(app, ui, &s);
        } else if app.search.pending == 0 && app.search.library_hits.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("No songs found.").weak());
        }
        let ac: Vec<CardData> = artists
            .iter()
            .map(|a| CardData { shape: widgets::CardShape::Square, art: a.artwork_url.clone(), title: a.name.clone(), sub: "Artist".into(), provider: Some(a.provider), open: CardAct::OpenAlbum(Album::default()), play: None })
            .collect();
        // artists open their page (handled separately to keep CardAct simple)
        if !ac.is_empty() {
            let (n, w) = widgets::grid_metrics(ui.available_width());
            widgets::section_header(ui, "Artists", false);
            let mut open = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (a, c) in artists.iter().zip(ac.iter()).take(n) {
                    let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(&c.art, Kind::Cover) } else { None };
                    let o = widgets::media_card(ui, tex.as_ref(), &c.title, &c.sub, c.provider, w, widgets::CardShape::Round, app.accent());
                    if o.open || o.play {
                        open = Some(a.clone());
                    }
                }
            });
            if let Some(a) = open {
                app.navigate(Page::Artist(a));
            }
        }
        let alc: Vec<CardData> = albums
            .iter()
            .map(|a| CardData { shape: widgets::CardShape::Square, art: a.artwork_url.clone(), title: a.name.clone(), sub: a.artists.join(", "), provider: Some(a.provider), open: CardAct::OpenAlbum(a.clone()), play: None })
            .collect();
        card_row(app, ui, "Albums", &alc, None);
        let plc: Vec<CardData> = playlists.iter().map(pl_card).collect();
        card_row(app, ui, "Playlists", &plc, None);
        ui.add_space(20.0);
    });
}

/// Song rows with artwork; Play starts the song immediately (and Similarity Radio when enabled).
fn song_rows(app: &mut AquaApp, ui: &mut Ui, tracks: &[Track]) {
    let mut act: Option<(usize, u8)> = None; // 0 play, 1 queue, 2 alternatives, 3 like
    let accent = app.accent();
    let cur = app.state.current.as_ref().map(|t| t.key());
    for (i, t) in tracks.iter().enumerate() {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 50.0), egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6, ui.visuals().faint_bg_color);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let ui2 = &mut child;
        let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover) } else { None };
        widgets::artwork(ui2, tex.as_ref(), 40.0, 4);
        ui2.vertical(|ui| {
            ui.set_width((w * 0.45).max(160.0));
            let mut title = RichText::new(&t.title).size(14.0);
            if cur.as_deref() == Some(&t.key()) {
                title = title.color(accent);
            }
            ui.add(egui::Label::new(title).truncate());
            ui.horizontal(|ui| {
                widgets::provider_badge(ui, t.provider);
                if t.explicit {
                    ui.label(RichText::new("E").small().weak());
                }
                ui.add(egui::Label::new(RichText::new(t.artist_line()).size(12.0).color(ui.visuals().weak_text_color())).truncate());
            });
        });
        ui2.add_sized([w * 0.25, 20.0], egui::Label::new(RichText::new(&t.album).size(12.5).color(ui2.visuals().weak_text_color())).truncate());
        ui2.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(fmt_ms(t.duration_ms)).size(12.0).color(ui.visuals().weak_text_color()));
            if resp.hovered() || resp.has_focus() {
                if t.provider == ProviderKind::Local && widgets::small_button(ui, "🔃", "Find this song on Spotify").clicked() {
                    act = Some((i, 2));
                }
                if widgets::small_button(ui, "+", "Add to queue").clicked() {
                    act = Some((i, 1));
                }
                if t.provider != ProviderKind::Local && widgets::small_button(ui, if app.is_liked(t) { "♥" } else { "♡" }, "Like").clicked() {
                    act = Some((i, 3));
                }
            }
            let pb = egui::Button::new(RichText::new(icon::PLAY()).color(ui.visuals().extreme_bg_color)).fill(accent).corner_radius(14).min_size(egui::vec2(28.0, 28.0));
            if ui.add(pb).on_hover_text(if app.radio.is_some() && app.cfg.active("radio_from_search") { "Play (starts Similarity Radio)" } else { "Play" }).clicked() {
                act = Some((i, 0));
            }
        });
        if resp.double_clicked() {
            act = Some((i, 0));
        }
    }
    if let Some((i, k)) = act {
        let t = tracks[i].clone();
        match k {
            0 => app.play_single(t, PlayFrom::Search),
            1 => {
                app.engine.send(Cmd::Enqueue(vec![t.clone()]));
                app.toast(format!("Added “{}” to the queue", t.title), false);
            }
            2 => app.find_alternatives(&t),
            _ => app.like(&t),
        }
    }
}

// ---------------------------------------------------------------------------
// Discover

pub fn discover(app: &mut AquaApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("Discover").size(24.0).strong());
        ui.add_space(6.0);
        widgets::card(ui, Some("Similarity Radio"), |ui| {
            super::shell::radio_controls(app, ui);
            if app.radio.is_some() {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Discovery strength:");
                    for s in crate::radio::Strength::ALL {
                        let sel = app.cfg.radio.strength == s;
                        if ui.add(egui::Button::new(s.label()).selected(sel).corner_radius(12)).on_hover_text(s.describe()).clicked() {
                            app.cfg.radio.strength = s;
                            app.mark_dirty();
                        }
                    }
                });
                widgets::note(ui, app.cfg.radio.strength.describe());
            }
        });
        if app.radio.is_none() {
            return;
        }
        let top = app.history.lock().unwrap().top_artists(12);
        let mixes: Vec<CardData> = top
            .iter()
            .map(|(a, t, n)| CardData { shape: widgets::CardShape::Square, art: t.artwork_url.clone(), title: format!("{a} Mix"), sub: format!("{n} plays"), provider: None, open: CardAct::Radio(t.clone()), play: None })
            .collect();
        let mixes: Vec<CardData> = mixes.into_iter().map(|c| CardData { shape: widgets::CardShape::Wide, ..c }).collect();
        card_row(app, ui, "Your Top Mixes", &mixes, None);
        let recent = app.history.lock().unwrap().recent_tracks(12);
        let rc: Vec<CardData> = recent
            .iter()
            .map(|t| CardData { shape: widgets::CardShape::Square, art: t.artwork_url.clone(), title: format!("Like “{}”", t.title), sub: t.artist_line(), provider: None, open: CardAct::Radio(t.clone()), play: None })
            .collect();
        card_row(app, ui, "Start radio from recent songs", &rc, None);
        let liked: Vec<Track> = {
            let mut s = app.library.lock().unwrap();
            let mut v = s.tracks(ProviderKind::Spotify, LIKED_ID).map(|t| t.iter().take(12).cloned().collect::<Vec<_>>()).unwrap_or_default();
            if v.is_empty() {
                v = s.tracks(ProviderKind::Local, crate::library::LOCAL_ALL_ID).map(|t| t.iter().take(12).cloned().collect()).unwrap_or_default();
            }
            v
        };
        let lc: Vec<CardData> = liked
            .iter()
            .map(|t| CardData { shape: widgets::CardShape::Square, art: t.artwork_url.clone(), title: format!("Like “{}”", t.title), sub: t.artist_line(), provider: None, open: CardAct::Radio(t.clone()), play: None })
            .collect();
        card_row(app, ui, "From your library", &lc, None);
        if top.is_empty() && recent.is_empty() && liked.is_empty() {
            widgets::note(ui, "Play some music first — mixes and suggestions appear here based on what you listen to (history stays on this PC).");
        }
    });
}

// ---------------------------------------------------------------------------
// Now Playing

pub fn now_playing(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    let Some(t) = st.current.clone() else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Nothing is playing").size(20.0).strong());
            widgets::note(ui, "Pick something from Home, Search or your Library.");
        });
        return;
    };
    let lyrics_on = app.cfg.active("lyrics");
    if app.lyrics.fullscreen && lyrics_on {
        if ui.button("✖ Exit fullscreen lyrics").clicked() {
            app.lyrics.fullscreen = false;
        }
        super::pages::lyrics_panel(app, ui, true);
        return;
    }
    let wide = ui.available_width() > 820.0;
    let accent = app.accent();
    let body = |app: &mut AquaApp, ui: &mut Ui| {
        ui.label(RichText::new(&t.title).size(28.0).strong());
        ui.label(RichText::new(t.artist_line()).size(17.0));
        if !t.album.is_empty() {
            ui.label(RichText::new(&t.album).color(ui.visuals().weak_text_color()));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::provider_badge(ui, t.provider);
            if t.provider != ProviderKind::Local {
                let liked = app.is_liked(&t);
                if ui.add(egui::Button::new(RichText::new(if liked { "♥ Liked" } else { "♡ Like" }).color(if liked { accent } else { ui.visuals().text_color() }))).clicked() && !liked {
                    app.like(&t);
                }
                if ui.button("Other version…").clicked() {
                    app.find_alternatives(&t);
                }
            }
            if ui.button(format!("{} Queue", icon::QUEUE())).clicked() {
                app.cfg.general.right_panel = crate::config::RightPanel::Queue;
                app.mark_dirty();
            }
            if lyrics_on && app.cfg.active("lyrics_fullscreen") && ui.button("⛶ Lyrics").clicked() {
                app.lyrics.fullscreen = true;
            }
        });
        // progress
        let pos = app.engine.position_ms();
        ui.add_space(8.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(520.0), 4.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2, ui.visuals().widgets.inactive.bg_fill);
        let mut f = r;
        f.set_width(r.width() * (pos as f32 / st.duration_ms.max(1) as f32).clamp(0.0, 1.0));
        ui.painter().rect_filled(f, 2, accent);
        ui.label(RichText::new(format!("{} / {}", fmt_ms(pos), fmt_ms(st.duration_ms))).size(11.5).color(ui.visuals().weak_text_color()));
        ui.add_space(10.0);
        widgets::card(ui, None, |ui| super::shell::radio_controls(app, ui));
        super::pages::visualizer(app, ui);
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(8.0);
        let tex = app.artwork.lock().unwrap().get(&t.artwork_url, Kind::Cover);
        if wide {
            ui.horizontal_top(|ui| {
                let size = (ui.available_width() * 0.42).clamp(260.0, 460.0);
                widgets::artwork(ui, tex.as_ref(), size, 10);
                ui.add_space(24.0);
                ui.vertical(|ui| body(app, ui));
            });
        } else {
            let size = (ui.available_width() * 0.6).clamp(180.0, 340.0);
            ui.vertical_centered(|ui| {
                widgets::artwork(ui, tex.as_ref(), size, 10);
            });
            ui.add_space(10.0);
            body(app, ui);
        }
        if lyrics_on && app.cfg.general.right_panel != crate::config::RightPanel::Lyrics {
            ui.add_space(14.0);
            let w = ui.available_width();
            ui.allocate_ui(egui::vec2(w, 420.0), |ui| {
                ui.set_min_height(200.0);
                super::pages::lyrics_panel(app, ui, false);
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Queue page

pub fn queue_page(app: &mut AquaApp, ui: &mut Ui) {
    let st = app.state.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Queue").size(24.0).strong());
        ui.add_space(12.0);
        if ui.button("Clear upcoming").clicked() {
            app.engine.send(Cmd::ClearQueue);
        }
        let mut sh = st.shuffle;
        if ui.checkbox(&mut sh, "Shuffle").changed() {
            app.engine.send(Cmd::SetShuffle(sh));
        }
        if !st.queue.is_empty() && ui.button("Save as playlist").clicked() {
            save_queue_as_playlist(app);
        }
    });
    widgets::card(ui, None, |ui| super::shell::radio_controls(app, ui));
    widgets::note(ui, "Double-click to jump. Songs you add yourself always play before Similarity Radio recommendations.");
    let mut cmd = None;
    let n = st.queue.len();
    let pos = st.queue_pos;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 32.0, n, |ui, range| {
        for i in range {
            let t = &st.queue[i];
            let cur = pos == Some(i);
            let radio = st.is_radio(i);
            let r = ui.horizontal(|ui| {
                ui.add_sized([34.0, 24.0], egui::Label::new(RichText::new(if cur { icon::NOW().to_string() } else { (i + 1).to_string() }).color(ui.visuals().weak_text_color())));
                widgets::provider_badge(ui, t.provider);
                let mut title = RichText::new(format!("{} — {}", t.title, t.artist_line()));
                if cur {
                    title = title.color(app.accent());
                }
                ui.add(egui::Label::new(title).truncate());
                if radio {
                    ui.label(RichText::new("radio").size(10.5).color(app.accent())).on_hover_text("Recommended by Similarity Radio");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_button(ui, "✖", "Remove").clicked() {
                        cmd = Some(Cmd::Remove(i));
                    }
                    if i + 1 < n && widgets::small_button(ui, "⏷", "Move down").clicked() {
                        cmd = Some(Cmd::Move(i, i + 1));
                    }
                    if i > 0 && widgets::small_button(ui, "⏶", "Move up").clicked() {
                        cmd = Some(Cmd::Move(i, i - 1));
                    }
                    ui.label(RichText::new(fmt_ms(t.duration_ms)).size(12.0).color(ui.visuals().weak_text_color()));
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

/// Save the current queue: as a private Spotify playlist when the queue has Spotify
/// tracks and you're signed in; local tracks are exported as an M3U file.
pub fn save_queue_as_playlist(app: &mut AquaApp) {
    let q = app.state.queue.clone();
    let name = format!("Aqua Glass Queue {}", crate::util::fmt_unix(crate::util::now_unix()));
    let sp: Vec<String> = q.iter().filter(|t| t.provider == ProviderKind::Spotify).map(|t| if t.uri.is_empty() { format!("spotify:track:{}", t.id) } else { t.uri.clone() }).collect();
    if !sp.is_empty() && app.spotify.is_signed_in() {
        let api = app.spotify.clone();
        let user = app.sp_profile.as_ref().map(|p| p.id.clone()).unwrap_or_default();
        let n = name.clone();
        app.jobs.spawn("Save queue to Spotify", move |h| {
            let r = api.create_playlist(&user, &n, "Saved from the Aqua Glass queue").and_then(|id| api.add_items(&id, &sp));
            h.send(match r {
                Ok(()) => super::jobs::Msg::Toast(format!("Saved {} Spotify songs as “{n}”", sp.len()), false),
                Err(e) => super::jobs::Msg::Toast(format!("Could not save to Spotify: {e}"), true),
            });
        });
    }
    let others: Vec<Track> = q.into_iter().filter(|t| t.provider != ProviderKind::Spotify || !app.spotify.is_signed_in()).collect();
    if !others.is_empty() {
        let path = crate::util::data_dir().join("exports").join(format!("{}.m3u", name.replace([':', '/'], "-")));
        let pl = Playlist { name: name.clone(), ..Default::default() };
        match crate::transfer::export_m3u(&pl, &others, &path) {
            Ok(()) => app.toast(format!("Saved {} songs to {}", others.len(), path.display()), false),
            Err(e) => app.toast(format!("Could not save: {e}"), true),
        }
    }
}

pub fn quick_settings(app: &mut AquaApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    ui.label(RichText::new("Quick settings").strong());
    for id in ["similarity_radio", "lyrics", "visualizer", "bg_album_art", "bg_blur", "bg_animated", "eq", "discord", "overlay"] {
        if let Some(f) = crate::features::get(id) {
            let mut v = app.cfg.on(id);
            ui.horizontal(|ui| {
                if widgets::switch(ui, &mut v).changed() {
                    app.request_toggle(id, v);
                }
                ui.label(f.name);
            });
        }
    }
    if app.cfg.services.spotify_backend == crate::config::SpotifyBackend::Soloist {
        let apo = app.cfg.audio.eq_engine == crate::config::EqEngine::EqualizerApo && app.apo_linked;
        ui.label(RichText::new(if apo { "EQ applies to Spotify too (Equalizer APO). The visualizer follows local files only." } else { "EQ and visualizer affect local files only — Spotify Soloist outputs its audio directly. Use Equalizer APO (Settings › Audio) for Spotify." }).small().weak());
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
