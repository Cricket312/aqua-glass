//! Application shell and content pages.

use super::widgets::{self, icon};
use super::{AquaApp, Confirm, Page};
use crate::artwork::Kind;
use crate::audio::engine::Cmd;
use crate::config::ProviderKind;
use crate::model::*;
use crate::util::fmt_ms;
use eframe::egui::{self, Color32, RichText, Ui};

fn back_button(app: &mut AquaApp, ui: &mut Ui) {
    if !app.back.is_empty() && ui.button("‹ Back").clicked() {
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
            let w_all = ui.available_width();
            let r = ui
                .allocate_ui_with_layout(egui::vec2(w_all, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_height(row_h);
                    ui.add_sized([36.0, row_h], egui::Label::new(RichText::new(if playing { icon::NOW().to_string() } else { (i + 1).to_string() }).weak()));
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
                        ui.menu_button(icon::MORE(), |ui| {
                            if ui.button("Play next").clicked() {
                                act = Some(RowAction::Next(t.clone()));
                                ui.close();
                            }
                            if ui.button("Add to queue").clicked() {
                                act = Some(RowAction::Queue(t.clone()));
                                ui.close();
                            }
                            if t.provider != ProviderKind::Local && ui.button(format!("{} Like", icon::HEART())).clicked() {
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
                            if t.provider != ProviderKind::Local && ui.button(format!("Open in browser {}", icon::EXTERNAL())).clicked() {
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
        RowAction::Play(i) => { let f = page_from(app); app.play_list(tracks.to_vec(), i, f) },
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

fn card(app: &mut AquaApp, ui: &mut Ui, art: &str, title: &str, sub: &str, p: ProviderKind) -> egui::Response {
    let (_, w) = widgets::grid_metrics(ui.available_width().max(400.0));
    let tex = if app.cfg.active("artwork_in_lists") { app.artwork.lock().unwrap().get(art, Kind::Cover) } else { None };
    let o = widgets::media_card(ui, tex.as_ref(), title, sub, Some(p), w.min(200.0), widgets::CardShape::Square, app.accent());
    let (_, resp) = ui.allocate_exact_size(egui::vec2(0.0, 0.0), egui::Sense::hover());
    if o.open || o.play {
        // synthesize a click response for callers
        let r = resp.clone();
        return clicked(r);
    }
    resp
}

fn clicked(mut r: egui::Response) -> egui::Response {
    r.flags.insert(egui::response::Flags::CLICKED);
    r
}

pub fn all_playlists(app: &mut AquaApp) -> Vec<Playlist> {
    let s = app.library.lock().unwrap();
    s.libs
        .iter()
        .filter(|(p, _)| app.filter.allows(**p) && app.provider_enabled(**p))
        .flat_map(|(_, l)| l.playlists.iter().map(|e| e.meta.clone()))
        .collect()
}

pub fn library(app: &mut AquaApp, ui: &mut Ui) {
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
                } else if ui.button(format!("{} {}", icon::SYNC(), if p == ProviderKind::Local { "Rescan" } else { "Sync Now" })).clicked() {
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
                widgets::warn(ui, &format!("{conflicts} item(s) need review — Settings › Library & Synchronization"));
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

pub fn playlists_page(app: &mut AquaApp, ui: &mut Ui) {
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

pub fn artists_page(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "Artists");
    let artists: Vec<Artist> = {
        let s = app.library.lock().unwrap();
        s.libs.values().flat_map(|l| l.artists.iter().cloned()).filter(|a| app.filter.allows(a.provider)).collect()
    };
    if artists.is_empty() {
        widgets::note(ui, "No followed artists imported. Enable “Import followed artists” in Settings › Library & Synchronization and sync.");
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

pub fn albums_page(app: &mut AquaApp, ui: &mut Ui) {
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

pub fn playlist_view(app: &mut AquaApp, ui: &mut Ui, p: ProviderKind, id: &str) {
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
        if ui.add_enabled(ok, egui::Button::new(format!("{}  Play", icon::PLAY()))).clicked() {
            if let Some(Ok(t)) = &tracks {
                app.engine.send(Cmd::SetShuffle(false));
                app.play_list(t.clone(), 0, super::PlayFrom::Other);
            }
        }
        if ui.add_enabled(ok, egui::Button::new(format!("{}  Shuffle", icon::SHUFFLE()))).clicked() {
            if let Some(Ok(t)) = &tracks {
                app.engine.send(Cmd::SetShuffle(true));
                let start = rand::random_range(0..t.len().max(1));
                app.play_list(t.clone(), start, super::PlayFrom::Other);
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
            if ui.button(format!("{} Transfer…", icon::TRANSFER())).clicked() {
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

pub fn album_view(app: &mut AquaApp, ui: &mut Ui, a: &Album) {
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
            if ui.button(format!("{}  Play album", icon::PLAY())).clicked() {
                app.play_list(t.clone(), 0, super::PlayFrom::Album);
            }
            if let Some(act) = track_list(app, ui, &t, &key) {
                handle_row_action(app, &t, act);
            }
        }
    }
}

pub fn artist_view(app: &mut AquaApp, ui: &mut Ui, a: &Artist) {
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
            ui.add(egui::Label::new(RichText::new(format!("Lyrics unavailable: {e}")).weak()).wrap());
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

fn page_from(app: &AquaApp) -> super::PlayFrom {
    match app.page {
        Page::Album(_) => super::PlayFrom::Album,
        Page::Artist(_) => super::PlayFrom::Artist,
        _ => super::PlayFrom::Other,
    }
}
