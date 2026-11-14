//! Playlist Transfer page: choose a source, preview matches, review, execute.

use super::jobs::Msg;
use super::widgets;
use super::AquaApp;
use crate::config::ProviderKind;
use crate::matching::MatchStatus;
use crate::model::{Playlist, Track};
use crate::services::{spotify::SpotifyApi, youtube::YouTubeApi};
use crate::transfer::{self, Plan, TransferLog};
use eframe::egui::{self, RichText, Ui};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Default)]
pub struct TransferUi {
    pub source: Option<(Playlist, Vec<Track>)>,
    pub target: Option<ProviderKind>,
    pub plan: Option<Plan>,
    pub building: bool,
    pub executing: bool,
    pub last: Option<Result<TransferLog, String>>,
    pub logs: Option<Vec<TransferLog>>,
    pub show: ShowFilter,
    pub source_pick: Option<(ProviderKind, String)>,
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum ShowFilter {
    #[default]
    All,
    Review,
    Unmatched,
}

impl TransferUi {
    pub fn select_source(&mut self, p: Playlist, t: Vec<Track>) {
        self.source = Some((p, t));
        self.plan = None;
        self.last = None;
    }
    pub fn on_plan(&mut self, plan: Plan, src: Vec<Track>) {
        self.building = false;
        if self.source.as_ref().is_some_and(|(p, _)| p.id == plan.source.id) {
            self.source = Some((plan.source.clone(), src));
        }
        self.plan = Some(plan);
    }
    pub fn on_done(&mut self, r: Result<TransferLog, String>) {
        self.executing = false;
        self.last = Some(r);
        self.logs = None;
    }
    pub fn remove_created(&mut self, sp: Arc<SpotifyApi>, yt: Arc<YouTubeApi>, jobs: &super::jobs::Jobs, log: TransferLog) {
        jobs.spawn("Delete transferred playlist", move |h| {
            let r = match log.target {
                ProviderKind::Spotify => {
                    let tok = sp.access_token(false);
                    tok.and_then(|t| {
                        let u = format!("https://api.spotify.com/v1/playlists/{}/followers", log.created_playlist_id);
                        let resp = crate::services::http().delete(u).bearer_auth(t).send().map_err(|e| crate::services::ApiError::Network(e.to_string()))?;
                        if resp.status().is_success() { Ok(()) } else { Err(crate::services::ApiError::Other(resp.status().as_u16(), "could not remove playlist".into())) }
                    })
                    .map_err(|e| e.to_string())
                }
                ProviderKind::YouTube => yt.delete_playlist(&log.created_playlist_id).map_err(|e| e.to_string()),
                ProviderKind::Local => Ok(()),
            };
            h.send(match r {
                Ok(()) => Msg::Toast(format!("Removed the transferred copy of “{}”", log.source_name), false),
                Err(e) => Msg::Toast(format!("Could not remove: {e}"), true),
            });
        });
    }
}

pub fn page(app: &mut AquaApp, ui: &mut Ui) {
    widgets::heading(ui, "🔃 Playlist Transfer");
    widgets::note(ui, "Copy playlists between Spotify and YouTube Music, or back them up to files. The source playlist is never modified, and a backup is saved before every transfer.");
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        widgets::section(ui, "1. Source");
        source_picker(app, ui);
        let Some((src, tracks)) = app.transfer.source.clone() else { return };
        ui.horizontal(|ui| {
            widgets::provider_badge(ui, src.provider);
            ui.label(RichText::new(&src.name).strong());
            ui.label(format!("{} tracks", tracks.len()));
        });
        ui.horizontal(|ui| {
            ui.label("Export / back up:");
            for (l, e) in [("JSON", "json"), ("CSV", "csv"), ("M3U", "m3u")] {
                if ui.button(l).clicked() {
                    let name: String = src.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' }).collect();
                    let def = crate::util::data_dir().join("exports").join(format!("{}.{e}", name.trim()));
                    let p = super::pages::pick_save(&def, e).unwrap_or(def);
                    let r = match e {
                        "json" => transfer::export_json(&src, &tracks, &p),
                        "csv" => transfer::export_csv(&tracks, &p),
                        _ => transfer::export_m3u(&src, &tracks, &p),
                    };
                    match r {
                        Ok(()) => app.toast(format!("Saved {}", p.display()), false),
                        Err(err) => app.toast(format!("Export failed: {err}"), true),
                    }
                }
            }
        });

        widgets::section(ui, "2. Destination");
        ui.horizontal(|ui| {
            for p in [ProviderKind::Spotify, ProviderKind::YouTube] {
                let signed = match p {
                    ProviderKind::Spotify => app.spotify.is_signed_in(),
                    _ => app.youtube.is_signed_in(),
                };
                let label = if signed { p.label().to_string() } else { format!("{} (sign in first)", p.label()) };
                if ui.add_enabled(signed, egui::Button::selectable(app.transfer.target == Some(p), label)).clicked() {
                    app.transfer.target = Some(p);
                    app.transfer.plan = None;
                }
            }
        });
        let Some(target) = app.transfer.target else { return };
        if target == ProviderKind::YouTube {
            let n = tracks.len();
            widgets::warn(ui, &format!("YouTube quota: matching uses ~{} units (100 per search) and adding uses ~{} units (50 per track) of the default 10,000/day. Large playlists may need several days.", n * 100 + n, n * 50 + 50));
        }

        widgets::section(ui, "3. Match tracks");
        if app.transfer.building {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching for matches…");
                if ui.button("Cancel").clicked() {
                    app.jobs.cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if ui.button("Find matches (preview)").clicked() {
            build(app, src.clone(), tracks.clone(), target);
        }
        if app.transfer.plan.as_ref().is_some_and(|p| p.target == target) {
            preview(app, ui);
        }
        if let Some(r) = &app.transfer.last {
            match r {
                Ok(l) => {
                    widgets::ok(ui, &format!("Transferred “{}” to {}: {} added, {} skipped, {} failed.", l.source_name, l.target.label(), l.added, l.skipped.len(), l.failed.len()));
                    for f in l.failed.iter().take(10) {
                        widgets::error(ui, f);
                    }
                }
                Err(e) => widgets::error(ui, e),
            }
        }
    });
    history(app, ui);
}

fn source_picker(app: &mut AquaApp, ui: &mut Ui) {
    let lists = super::pages::all_playlists(app);
    ui.horizontal(|ui| {
        let sel = app.transfer.source.as_ref().map(|(p, _)| p.name.clone()).unwrap_or_else(|| "Choose a playlist…".into());
        let mut pick = None;
        egui::ComboBox::from_id_salt("src").selected_text(sel).width(320.0).show_ui(ui, |ui| {
            for p in &lists {
                if p.provider == ProviderKind::Local && p.id == crate::library::LOCAL_ALL_ID && p.track_count > 2000 {
                    continue;
                }
                if ui.selectable_label(false, format!("[{}] {} ({})", p.provider.short(), p.name, p.track_count)).clicked() {
                    pick = Some((p.provider, p.id.clone()));
                }
            }
        });
        if let Some(p) = pick {
            app.transfer.source_pick = Some(p);
            app.transfer.source = None;
            app.transfer.plan = None;
        }
        if ui.button("Restore from file (JSON/CSV/M3U)…").clicked() {
            if let Some(path) = super::pages::pick_open(&["json", "csv", "m3u", "m3u8"]) {
                match transfer::import_file(&path) {
                    Ok((p, t)) => app.transfer.select_source(p, t),
                    Err(e) => app.toast(format!("Could not read file: {e}"), true),
                }
            }
        }
    });
    if let Some((p, id)) = app.transfer.source_pick.clone() {
        match app.playlist_tracks(p, &id) {
            None => {
                ui.spinner();
            }
            Some(Err(e)) => {
                widgets::error(ui, &e);
                app.transfer.source_pick = None;
            }
            Some(Ok(t)) => {
                let meta = app.library.lock().unwrap().lib(p).playlist(&id).map(|e| e.meta.clone()).unwrap_or_default();
                app.transfer.select_source(meta, t);
                app.transfer.source_pick = None;
            }
        }
    }
}

fn build(app: &mut AquaApp, src: Playlist, tracks: Vec<Track>, target: ProviderKind) {
    app.transfer.building = true;
    app.transfer.plan = None;
    app.jobs.cancel.store(false, Ordering::Relaxed);
    let cancel = app.jobs.cancel.clone();
    let sp = app.spotify.clone();
    let yt = app.youtube.clone();
    app.jobs.spawn(format!("Match {} tracks", tracks.len()), move |h| {
        let search = |q: &str| -> Result<Vec<Track>, String> {
            match target {
                ProviderKind::Spotify => sp.search_tracks(q, 8).map_err(|e| e.to_string()),
                _ => yt.search_tracks(q, 6).map_err(|e| e.to_string()),
            }
        };
        let plan = transfer::build_plan(src, tracks.clone(), target, &search, &|d, n| h.progress(d, n), &|| cancel.load(Ordering::Relaxed));
        h.send(Msg::PlanReady(plan, tracks));
    });
}

fn preview(app: &mut AquaApp, ui: &mut Ui) {
    let Some(plan) = app.transfer.plan.as_mut() else { return };
    let (m, a, u, sel) = plan.counts();
    let total = plan.items.len();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("Total: {total}")).strong());
            ui.label(RichText::new(format!("✔ Matched: {m}")).color(widgets::impact_color(crate::features::Impact::Low)));
            ui.label(RichText::new(format!("? Ambiguous / needs review: {a}")).color(widgets::impact_color(crate::features::Impact::Moderate)));
            ui.label(RichText::new(format!("✖ Unmatched: {u}")).color(widgets::impact_color(crate::features::Impact::VeryHigh)));
            ui.label(format!("Selected to transfer: {sel}"));
        });
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.transfer.show, ShowFilter::All, "All");
        ui.selectable_value(&mut app.transfer.show, ShowFilter::Review, "Needs review");
        ui.selectable_value(&mut app.transfer.show, ShowFilter::Unmatched, "Unmatched");
        if ui.button("Accept best for all ambiguous").on_hover_text("Selects the top candidate for each ambiguous track — review them first").clicked() {
            for it in plan.items.iter_mut().filter(|i| i.status == MatchStatus::Ambiguous && !i.candidates.is_empty()) {
                it.choice = Some(0);
            }
        }
    });
    let show = app.transfer.show;
    let idx: Vec<usize> = plan
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| match show {
            ShowFilter::All => true,
            ShowFilter::Review => i.status == MatchStatus::Ambiguous,
            ShowFilter::Unmatched => i.status == MatchStatus::Unmatched,
        })
        .map(|(k, _)| k)
        .collect();
    egui::ScrollArea::vertical().id_salt("plan").max_height(380.0).auto_shrink([false, true]).show_rows(ui, 30.0, idx.len(), |ui, range| {
        for k in idx[range].iter().copied() {
            let it = &mut plan.items[k];
            ui.horizontal(|ui| {
                let (sym, c) = match it.status {
                    MatchStatus::Matched => ("✔", crate::features::Impact::Low),
                    MatchStatus::Ambiguous => ("?", crate::features::Impact::Moderate),
                    MatchStatus::Unmatched => ("✖", crate::features::Impact::VeryHigh),
                };
                ui.label(RichText::new(sym).color(widgets::impact_color(c)));
                ui.add_sized([280.0, 20.0], egui::Label::new(format!("{} — {}", it.src.artist_line(), it.src.title)).truncate());
                ui.label("›");
                let sel = match it.choice.and_then(|c| it.candidates.get(c)) {
                    Some(c) => format!("{} — {} ({:.0}%)", c.track.artist_line(), c.track.title, c.score * 100.0),
                    None => "skip".into(),
                };
                egui::ComboBox::from_id_salt(("cand", k)).selected_text(sel).width(360.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut it.choice, None, "Skip this track");
                    for (ci, c) in it.candidates.iter().enumerate() {
                        let note = if c.notes.is_empty() { String::new() } else { format!("  [{}]", c.notes.join(", ")) };
                        ui.selectable_value(&mut it.choice, Some(ci), format!("{:.0}%  {} — {} ({}){note}", c.score * 100.0, c.track.artist_line(), c.track.title, crate::util::fmt_ms(c.track.duration_ms)));
                    }
                });
                if let Some(e) = &it.error {
                    ui.label(RichText::new("⚠").color(widgets::impact_color(crate::features::Impact::High))).on_hover_text(e);
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.label("New playlist name");
        ui.add(egui::TextEdit::singleline(&mut plan.new_name).desired_width(260.0));
    });
    let can = sel > 0 && !app.transfer.executing;
    if app.transfer.executing {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Transferring…");
        });
    } else if ui.add_enabled(can, egui::Button::new(format!("Create “{}” with {sel} tracks on {}", plan.new_name, plan.target.label()))).clicked() {
        execute(app);
    }
}

fn execute(app: &mut AquaApp) {
    let Some(plan) = app.transfer.plan.clone() else { return };
    let src_tracks = app.transfer.source.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
    app.transfer.executing = true;
    let sp = app.spotify.clone();
    let yt = app.youtube.clone();
    let user = app.sp_profile.as_ref().map(|p| p.id.clone()).unwrap_or_default();
    app.jobs.spawn("Transfer playlist", move |h| {
        let target = plan.target;
        let create = |name: &str, desc: &str| -> Result<String, String> {
            match target {
                ProviderKind::Spotify => sp.create_playlist(&user, name, desc).map_err(|e| e.to_string()),
                _ => yt.create_playlist(name, desc).map_err(|e| e.to_string()),
            }
        };
        let add = |id: &str, tracks: &[Track]| -> Result<(), String> {
            match target {
                ProviderKind::Spotify => sp.add_items(id, &tracks.iter().map(|t| if t.uri.is_empty() { format!("spotify:track:{}", t.id) } else { t.uri.clone() }).collect::<Vec<_>>()).map_err(|e| e.to_string()),
                _ => {
                    for t in tracks {
                        yt.add_item(id, &t.id).map_err(|e| e.to_string())?;
                    }
                    Ok(())
                }
            }
        };
        let r = transfer::execute(&plan, &src_tracks, &create, &add, &|d, n| h.progress(d, n));
        h.send(Msg::TransferDone(r));
    });
}

fn history(app: &mut AquaApp, ui: &mut Ui) {
    widgets::section(ui, "Transfer history");
    let logs = app.transfer.logs.get_or_insert_with(transfer::load_logs).clone();
    if logs.is_empty() {
        widgets::note(ui, "No transfers yet.");
    }
    for l in logs.iter().take(20) {
        ui.horizontal(|ui| {
            ui.label(crate::util::fmt_unix(l.at));
            widgets::provider_badge(ui, l.source_provider);
            ui.label("›");
            widgets::provider_badge(ui, l.target);
            ui.label(format!("“{}”: {} added, {} skipped, {} failed", l.source_name, l.added, l.skipped.len(), l.failed.len()));
            if ui.small_button("Backup").on_hover_text(&l.backup_file).clicked() {
                let _ = open::that_detached(std::path::Path::new(&l.backup_file).parent().unwrap_or(std::path::Path::new(".")));
            }
            if ui.small_button("Remove created playlist…").clicked() {
                app.confirm = Some(super::Confirm::RemoveTransferred(l.clone()));
            }
        });
    }
}
