//! Settings: searchable, 17 categories, a toggle + RAM rating for every
//! optional feature, live performance dashboard, presets and theme editor.

use super::widgets::{self, icon};
use super::{AquaApp, Confirm, Page};
use crate::config::*;
use crate::features::{self, Category, Preset};
use crate::shortcuts::{self, Action, Binding};
use crate::theme::{self, ButtonStyle, IconStyle};
use eframe::egui::{self, RichText, Ui};

#[derive(Default)]
pub struct SettingsUi {
    pub category: Category,
    pub search: String,
    pub discord_app_id: String,
    pub discord_opts_last: Option<crate::discord::DiscordOpts>,
    pub new_preset_name: String,
    pub rename_to: String,
    pub devices: Option<Vec<String>>,
    pub status: String,
    pub selftest_artist: String,
    pub selftest_title: String,
    pub selftest_result: std::sync::Arc<std::sync::Mutex<Option<(String, std::path::PathBuf)>>>,
    pub soloist_key_input: String,
    /// `soloist --version` result for a path (run once per path, not every frame).
    pub soloist_version: Option<(String, Result<(String, i64), String>)>,
}

impl Default for Category {
    fn default() -> Self {
        Category::General
    }
}

pub fn page(app: &mut AquaApp, ui: &mut Ui) {
    if app.settings.discord_app_id.is_empty() && !app.cfg.services.discord_app_id.is_empty() {
        app.settings.discord_app_id = app.cfg.services.discord_app_id.clone();
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Settings").size(24.0).strong());
        ui.add_space(16.0);
        let fill = ui.visuals().faint_bg_color;
        egui::Frame::new().fill(fill).corner_radius(16).inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.label(RichText::new(icon::SEARCH()).color(ui.visuals().weak_text_color()));
            ui.add(egui::TextEdit::singleline(&mut app.settings.search).frame(egui::Frame::NONE).hint_text("Search settings — e.g. RAM, lyrics, Discord, background, radio").desired_width(340.0));
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.settings.search.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Disable all visual customizations").on_hover_text("Turns off every optional visual feature. Playback, accounts, libraries, playlists and playback preferences are kept.").clicked() {
                app.cfg.disable_all_visual();
                app.mark_dirty();
                app.toast("All visual customizations disabled. Use “Restore previous profile” to undo.", false);
            }
            if app.cfg.saved_visual_profile.is_some() && ui.button("Restore previous profile").clicked() {
                app.cfg.restore_visual();
                app.mark_dirty();
            }
            ui.label(RichText::new(format!("Preset: {}", app.cfg.preset.label())).size(12.0).color(ui.visuals().weak_text_color()));
        });
    });
    ui.add_space(8.0);
    if !app.settings.search.trim().is_empty() {
        search_results(app, ui);
        return;
    }
    egui::Panel::left("settings_cats").resizable(false).exact_size(190.0).frame(egui::Frame::new()).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for c in Category::ALL {
                let active = app.settings.category == c;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(180.0, 28.0), egui::Sense::click());
                let v = ui.visuals().clone();
                if active {
                    ui.painter().rect_filled(rect, 6, v.widgets.inactive.bg_fill);
                    ui.painter().rect_filled(egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 6.0), egui::vec2(3.0, 16.0)), 2, app.accent());
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 6, v.widgets.inactive.bg_fill.gamma_multiply(0.6));
                }
                let col = if active { v.strong_text_color() } else { v.weak_text_color() };
                ui.painter().text(rect.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, c.label(), egui::FontId::proportional(13.5), col);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.settings.category = c;
                    app.cfg.general.last_settings_category = c.label().to_string();
                    app.mark_dirty();
                }
            }
        });
    });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 16, ..Default::default() })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let c = app.settings.category;
            ui.horizontal(|ui| {
                ui.label(RichText::new(c.label()).size(19.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset category").clicked() {
                        app.cfg.reset_category(c);
                        app.mark_dirty();
                    }
                });
            });
            ui.add_space(6.0);
            match c {
                Category::General => general(app, ui),
                Category::Appearance => appearance(app, ui),
                Category::Themes => themes(app, ui),
                Category::Backgrounds => backgrounds(app, ui),
                Category::Playback => playback(app, ui),
                Category::Audio => audio(app, ui),
                Category::Lyrics => lyrics(app, ui),
                Category::Visualizer => visualizer(app, ui),
                Category::Discord => discord(app, ui),
                Category::Shortcuts => shortcuts_cat(app, ui),
                Category::MiniPlayer => mini(app, ui),
                Category::Services => services(app, ui),
                Category::Library => library(app, ui),
                Category::Transfer => transfer(app, ui),
                Category::Performance => performance(app, ui),
                Category::Privacy => privacy(app, ui),
                Category::Advanced => advanced(app, ui),
            }
            ui.add_space(30.0);
        });
    });
}

/// Every feature toggle in a category.
fn toggles(app: &mut AquaApp, ui: &mut Ui, cat: Category) {
    let ids: Vec<&'static str> = features::FEATURES.iter().filter(|f| f.category == cat && !f.id.starts_with("radio_") && f.id != "similarity_radio").map(|f| f.id).collect();
    toggle_card(app, ui, "Optional features", &ids);
}

/// A card of feature rows separated by thin dividers.
pub fn toggle_card(app: &mut AquaApp, ui: &mut Ui, title: &str, ids: &[&str]) {
    if ids.is_empty() {
        return;
    }
    widgets::card(ui, Some(title), |ui| {
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                ui.add_space(2.0);
                ui.separator();
            }
            feature_row(app, ui, id);
        }
    });
}

pub fn feature_row(app: &mut AquaApp, ui: &mut Ui, id: &str) {
    let Some(f) = features::get(id) else { return };
    let mut v = app.cfg.on(id);
    let deps_ok = f.deps.iter().all(|d| app.cfg.active(d));
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 70.0).max(200.0));
            ui.horizontal(|ui| {
                let mut name = RichText::new(f.name).size(14.0).strong();
                if v && !deps_ok {
                    name = name.color(ui.visuals().weak_text_color());
                }
                ui.label(name);
                if v && !deps_ok {
                    ui.label(RichText::new("inactive — needs a disabled feature").size(11.0).color(ui.visuals().weak_text_color()));
                }
            });
            ui.add(egui::Label::new(RichText::new(f.desc).size(12.5).color(ui.visuals().weak_text_color())).wrap());
            ui.horizontal_wrapped(|ui| {
                widgets::ram_badge(ui, f, app.measurements.by_feature.get(id));
                if !f.deps.is_empty() {
                    let names: Vec<String> = f
                        .deps
                        .iter()
                        .map(|d| {
                            let n = features::get(d).map(|x| x.name).unwrap_or(d);
                            if app.cfg.active(d) { format!("{n} ✔") } else { format!("{n} ✖") }
                        })
                        .collect();
                    ui.label(RichText::new(format!("Requires: {}", names.join(", "))).size(11.0).color(ui.visuals().weak_text_color()));
                }
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("↺").size(12.0)).frame(false)).on_hover_text("Reset to default").clicked() {
                app.cfg.reset_feature(id);
                app.mark_dirty();
            }
            if widgets::switch(ui, &mut v).on_hover_text(if v { "On" } else { "Off" }).changed() {
                app.request_toggle(id, v);
            }
        });
    });
}

fn search_results(app: &mut AquaApp, ui: &mut Ui) {
    let q = app.settings.search.trim().to_string();
    let cats: Vec<Category> = Category::ALL.into_iter().filter(|c| crate::util::icontains(c.label(), &q) || category_keywords(*c).iter().any(|k| crate::util::icontains(k, &q))).collect();
    let feats: Vec<&'static str> = features::FEATURES.iter().filter(|f| f.matches(&q)).map(|f| f.id).collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if !cats.is_empty() {
            ui.label(RichText::new("Categories").strong());
            ui.horizontal_wrapped(|ui| {
                for c in cats {
                    if ui.button(c.label()).clicked() {
                        app.settings.category = c;
                        app.settings.search.clear();
                    }
                }
            });
            ui.separator();
        }
        if feats.is_empty() {
            ui.label("No matching toggles.");
        }
        for id in feats {
            let cat = features::get(id).unwrap().category;
            ui.label(RichText::new(cat.label()).small().weak());
            feature_row(app, ui, id);
            ui.separator();
        }
    });
}

fn category_keywords(c: Category) -> &'static [&'static str] {
    match c {
        Category::Performance => &["ram", "memory", "cpu", "fps", "cache", "preset", "target", "budget"],
        Category::Backgrounds => &["background", "wallpaper", "blur", "image", "dim"],
        Category::Audio => &["eq", "equalizer", "device", "output", "bass", "balance", "buffer", "volume"],
        Category::Services => &["spotify", "account", "login", "sign in", "local", "folder", "premium"],
        Category::Library => &["sync", "import", "playlist", "liked", "conflict"],
        Category::Themes => &["theme", "color", "colour", "font", "editor"],
        Category::Appearance => &["font", "scale", "sidebar", "layout"],
        Category::Privacy => &["privacy", "telemetry", "cache", "log", "network", "disconnect"],
        Category::Shortcuts => &["keyboard", "hotkey", "macro", "shortcut", "media keys"],
        Category::Discord => &["discord", "rich presence"],
        Category::Lyrics => &["lyrics", "karaoke", "offset"],
        Category::MiniPlayer => &["mini", "overlay", "always on top"],
        Category::Transfer => &["transfer", "export", "migration", "backup"],
        _ => &[],
    }
}

fn slider_f(ui: &mut Ui, label: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>, dirty: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.add(egui::Slider::new(v, r)).changed() {
            *dirty = true;
        }
    });
}
fn slider_u(ui: &mut Ui, label: &str, v: &mut u32, r: std::ops::RangeInclusive<u32>, dirty: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.add(egui::Slider::new(v, r)).changed() {
            *dirty = true;
        }
    });
}

// ---------------------------------------------------------------------------

fn general(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::General);
    #[cfg(not(windows))]
    widgets::note(ui, "Tray, autostart, notifications and media controls are Windows features; they are inactive in this development build.");
    let mut d = false;
    widgets::section(ui, "Interface");
    slider_f(ui, "Interface scale (High-DPI is automatic; this is an extra zoom)", &mut app.cfg.general.ui_scale, 0.6..=2.5, &mut d);
    slider_f(ui, "Sidebar width", &mut app.cfg.general.sidebar_width, 150.0..=400.0, &mut d);
    widgets::section(ui, "Settings file");
    ui.horizontal(|ui| {
        if ui.button("Export settings…").clicked() {
            let def = crate::util::data_dir().join("exports").join("aqua-glass-settings.toml");
            let p = super::pages::pick_save(&def, "toml").unwrap_or(def);
            match app.cfg.export_to(&p) {
                Ok(()) => app.toast(format!("Settings exported to {} (no credentials included)", p.display()), false),
                Err(e) => app.toast(format!("Export failed: {e}"), true),
            }
        }
        if ui.button("Import settings…").clicked() {
            if let Some(p) = super::pages::pick_open(&["toml"]) {
                match Config::import_from(&p) {
                    Ok(mut c) => {
                        c.services = app.cfg.services.clone(); // keep account configuration
                        app.cfg = c;
                        d = true;
                        app.toast("Settings imported", false);
                    }
                    Err(e) => app.toast(format!("Invalid settings file: {e}"), true),
                }
            }
        }
        if ui.button("Restore defaults…").clicked() {
            app.confirm = Some(Confirm::ResetAll);
        }
    });
    widgets::note(ui, &format!("Settings are saved automatically to {}", Config::path().display()));
    if d {
        app.mark_dirty();
    }
}

fn appearance(app: &mut AquaApp, ui: &mut Ui) {
    let mut d = false;
    ui.horizontal(|ui| {
        ui.label("Theme");
        let names: Vec<String> = app.themes.iter().map(|t| t.name.clone()).collect();
        egui::ComboBox::from_id_salt("theme_pick").selected_text(app.cfg.appearance.theme.clone()).show_ui(ui, |ui| {
            for n in names {
                if ui.selectable_label(app.cfg.appearance.theme == n, &n).clicked() {
                    app.cfg.appearance.theme = n;
                    d = true;
                }
            }
        });
        if ui.button("Edit theme…").clicked() {
            app.settings.category = Category::Themes;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Playback bar layout");
        for (l, v) in [("Classic", PlayerBarLayout::Classic), ("Centered", PlayerBarLayout::Centered), ("Compact", PlayerBarLayout::Compact)] {
            if ui.selectable_label(app.cfg.appearance.player_bar_layout == v, l).clicked() {
                app.cfg.appearance.player_bar_layout = v;
                d = true;
            }
        }
    });
    slider_f(ui, "Playback bar height", &mut app.cfg.appearance.player_bar_height, 60.0..=130.0, &mut d);
    ui.horizontal(|ui| {
        ui.label("Font file (for “Custom Font”)");
        if ui.add(egui::TextEdit::singleline(&mut app.cfg.appearance.font_path).desired_width(300.0)).lost_focus() {
            d = true;
        }
        if ui.button("Browse…").clicked() {
            if let Some(p) = super::pages::pick_open(&["ttf", "otf", "ttc"]) {
                app.cfg.appearance.font_path = p.to_string_lossy().to_string();
                d = true;
            }
        }
    });
    widgets::note(ui, "Font weight: choose a weighted font file (e.g. segoeuisb.ttf / segoeuib.ttf). Font size is set per theme.");
    if d {
        app.mark_dirty();
    }
    widgets::section(ui, "Optional appearance features");
    toggles(app, ui, Category::Appearance);
}

fn themes(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Themes);
    let names: Vec<String> = app.themes.iter().map(|t| t.name.clone()).collect();
    ui.horizontal(|ui| {
        ui.label("Edit:");
        egui::ComboBox::from_id_salt("theme_edit").selected_text(app.edit_theme.name.clone()).show_ui(ui, |ui| {
            for n in &names {
                if ui.selectable_label(app.edit_theme.name == *n, n).clicked() {
                    if let Some(t) = app.themes.iter().find(|t| t.name == *n) {
                        app.edit_theme = t.clone();
                    }
                }
            }
        });
        if !app.cfg.on("custom_theme") {
            widgets::warn(ui, "“Custom Theme” is off — enable it in Appearance to see themes.");
        }
    });
    widgets::note(ui, "Changes preview immediately on the selected theme. Themes are plain TOML files (no scripts) in your data folder.");
    let before = app.edit_theme.clone();
    let t = &mut app.edit_theme;
    egui::Grid::new("theme_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        let color = |ui: &mut Ui, label: &str, c: &mut [u8; 3]| {
            ui.label(label);
            ui.color_edit_button_srgb(c);
            ui.end_row();
        };
        color(ui, "Accent", &mut t.accent);
        color(ui, "Secondary accent", &mut t.accent2);
        color(ui, "Background", &mut t.background);
        color(ui, "Panels", &mut t.panel);
        color(ui, "Surfaces / buttons", &mut t.surface);
        color(ui, "Text", &mut t.text);
        color(ui, "Dim text", &mut t.text_dim);
        color(ui, "Borders", &mut t.border);
        ui.label("Dark theme");
        ui.checkbox(&mut t.dark, "");
        ui.end_row();
        let s = |ui: &mut Ui, label: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>| {
            ui.label(label);
            ui.add(egui::Slider::new(v, r));
            ui.end_row();
        };
        s(ui, "Panel transparency (opacity)", &mut t.panel_alpha, 0.0..=1.0);
        s(ui, "Corner rounding", &mut t.rounding, 0.0..=30.0);
        s(ui, "Spacing", &mut t.spacing, 2.0..=24.0);
        s(ui, "Font size", &mut t.font_size, 9.0..=28.0);
        s(ui, "Heading size", &mut t.heading_size, 12.0..=48.0);
        s(ui, "Border width", &mut t.border_width, 0.0..=4.0);
        s(ui, "Shadow intensity", &mut t.shadow_intensity, 0.0..=1.0);
        s(ui, "Hover effect strength", &mut t.hover_strength, 0.0..=1.0);
        s(ui, "Animation speed", &mut t.animation_speed, 0.2..=5.0);
        s(ui, "Blur intensity (backgrounds)", &mut t.blur_intensity, 0.0..=2.0);
        ui.label("Buttons");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut t.button_style, ButtonStyle::Filled, "Filled");
            ui.selectable_value(&mut t.button_style, ButtonStyle::Outline, "Outline");
            ui.selectable_value(&mut t.button_style, ButtonStyle::Flat, "Flat");
        });
        ui.end_row();
        ui.label("Icons");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut t.icon_style, IconStyle::Solid, "Solid");
            ui.selectable_value(&mut t.icon_style, IconStyle::Outline, "Outline");
            ui.selectable_value(&mut t.icon_style, IconStyle::Text, "Text");
        });
        ui.end_row();
    });
    if *t != before {
        t.validate();
        // live preview
        let edited = t.clone();
        if let Some(slot) = app.themes.iter_mut().find(|x| x.name == edited.name) {
            *slot = edited.clone();
        }
        if app.cfg.appearance.theme != edited.name {
            app.cfg.appearance.theme = edited.name.clone();
            app.mark_dirty();
        }
        app.bg.theme_dirty = true;
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("💾 Save").clicked() {
            match theme::save_user_theme(&app.edit_theme) {
                Ok(p) => app.toast(format!("Theme saved: {}", p.display()), false),
                Err(e) => app.toast(format!("Save failed: {e}"), true),
            }
            app.reload_themes();
        }
        if ui.button("Duplicate").clicked() {
            let mut t = app.edit_theme.clone();
            t.name = format!("{} copy", t.name);
            let _ = theme::save_user_theme(&t);
            app.reload_themes();
            app.cfg.appearance.theme = t.name.clone();
            app.edit_theme = t;
            app.mark_dirty();
        }
        ui.add(egui::TextEdit::singleline(&mut app.settings.rename_to).hint_text("new name").desired_width(120.0));
        if ui.button("Rename").clicked() && !app.settings.rename_to.trim().is_empty() {
            let old = app.edit_theme.name.clone();
            let _ = theme::delete_user_theme(&old);
            app.edit_theme.name = app.settings.rename_to.trim().to_string();
            let _ = theme::save_user_theme(&app.edit_theme);
            app.cfg.appearance.theme = app.edit_theme.name.clone();
            app.settings.rename_to.clear();
            app.reload_themes();
            app.mark_dirty();
        }
        if ui.button("Export…").clicked() {
            let def = crate::util::data_dir().join("exports").join(format!("{}.toml", app.edit_theme.name));
            let p = super::pages::pick_save(&def, "toml").unwrap_or(def);
            match crate::util::atomic_write(&p, app.edit_theme.to_toml().as_bytes()) {
                Ok(()) => app.toast(format!("Theme exported to {}", p.display()), false),
                Err(e) => app.toast(format!("Export failed: {e}"), true),
            }
        }
        if ui.button("Import…").clicked() {
            if let Some(p) = super::pages::pick_open(&["toml"]) {
                match theme::import_theme(&p) {
                    Ok(t) => {
                        app.toast(format!("Imported theme “{}”", t.name), false);
                        app.cfg.appearance.theme = t.name.clone();
                        app.edit_theme = t;
                        app.reload_themes();
                        app.mark_dirty();
                    }
                    Err(e) => app.toast(format!("Invalid theme: {e}"), true),
                }
            }
        }
        if ui.button("Reset").on_hover_text("Discard your edits (built-in themes return to their original values)").clicked() {
            let _ = theme::delete_user_theme(&app.edit_theme.name);
            app.reload_themes();
            if let Some(t) = app.themes.iter().find(|t| t.name == app.edit_theme.name).cloned().or_else(|| app.themes.first().cloned()) {
                app.cfg.appearance.theme = t.name.clone();
                app.edit_theme = t;
                app.mark_dirty();
            }
        }
        if ui.button("Open themes folder").clicked() {
            let _ = crate::util::ensure_dir(&theme::themes_dir());
            let _ = open::that_detached(theme::themes_dir());
        }
    });
    widgets::note(ui, "Share a theme by sending its .toml file; others import it with “Import…”.");
}

fn backgrounds(app: &mut AquaApp, ui: &mut Ui) {
    let mut d = false;
    let b = &mut app.cfg.background;
    ui.horizontal(|ui| {
        ui.label("Background image");
        if ui.add(egui::TextEdit::singleline(&mut b.image_path).desired_width(300.0)).lost_focus() {
            d = true;
        }
        if ui.button("Browse…").clicked() {
            if let Some(p) = super::pages::pick_open(&["png", "jpg", "jpeg", "webp"]) {
                b.image_path = p.to_string_lossy().to_string();
                d = true;
            }
        }
    });
    slider_f(ui, "Blur radius", &mut b.blur_radius, 0.0..=80.0, &mut d);
    slider_f(ui, "Dimming", &mut b.dim, 0.0..=1.0, &mut d);
    slider_f(ui, "Image opacity", &mut b.opacity, 0.0..=1.0, &mut d);
    slider_f(ui, "Animation speed", &mut b.anim_speed, 0.1..=4.0, &mut d);
    slider_u(ui, "Animation FPS", &mut b.anim_fps, 5..=60, &mut d);
    slider_u(ui, "Transition duration (ms)", &mut b.transition_ms, 0..=3000, &mut d);
    slider_f(ui, "Parallax strength (px)", &mut b.parallax_px, 0.0..=60.0, &mut d);
    ui.horizontal(|ui| {
        ui.label("Scaling");
        for (l, v) in [("Fill", BgScale::Fill), ("Fit", BgScale::Fit), ("Stretch", BgScale::Stretch), ("Center", BgScale::Center)] {
            if ui.selectable_label(b.scale == v, l).clicked() {
                b.scale = v;
                d = true;
            }
        }
    });
    if d {
        app.artwork.lock().unwrap().clear_kind_backgrounds();
        app.mark_dirty();
    }
    widgets::section(ui, "Background effects");
    toggles(app, ui, Category::Backgrounds);
}

fn playback(app: &mut AquaApp, ui: &mut Ui) {
    let mut d = false;
    let p = &mut app.cfg.playback;
    ui.horizontal(|ui| {
        ui.label("Preferred provider");
        for v in [ProviderKind::Spotify, ProviderKind::Local] {
            if ui.selectable_label(p.preferred_provider == v, v.label()).clicked() {
                p.preferred_provider = v;
                d = true;
            }
        }
    });
    widgets::note(ui, "Results from the preferred source are listed first. For a local file, “Find on Spotify…” looks up the same recording — Aqua Glass never substitutes recordings silently.");
    ui.horizontal(|ui| {
        ui.label("Spotify streaming quality");
        for (l, v) in [("96 kbps", Quality::Low96), ("160 kbps", Quality::Normal160), ("320 kbps", Quality::High320)] {
            if ui.selectable_label(p.quality == v, l).clicked() {
                p.quality = v;
                d = true;
            }
        }
    });
    slider_u(ui, "Fade duration (ms) for “Fade In / Fade Out”", &mut p.fade_ms, 0..=2000, &mut d);
    slider_u(ui, "Seek step (s)", &mut p.seek_step_s, 1..=60, &mut d);
    slider_f(ui, "Volume step", &mut p.volume_step, 0.01..=0.25, &mut d);
    if d {
        app.mark_dirty();
    }
    toggles(app, ui, Category::Playback);
    radio_settings(app, ui);
}

fn radio_settings(app: &mut AquaApp, ui: &mut Ui) {
    ui.add_space(4.0);
    ui.label(RichText::new("Similarity Radio").size(17.0).strong());
    widgets::note(ui, "Play a single song from search and Aqua Glass keeps going with similar songs. Only songs that can actually be played (local library, or Spotify with Premium) are queued; songs you add yourself always play first.");
    ui.add_space(6.0);
    toggle_card(app, ui, "Radio", &["similarity_radio", "radio_from_search", "radio_from_artist", "radio_from_album", "radio_endless"]);
    let mut d = false;
    widgets::card(ui, Some("Discovery"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Discovery strength");
            for st in crate::radio::Strength::ALL {
                if ui.add(egui::Button::new(st.label()).selected(app.cfg.radio.strength == st).corner_radius(12)).on_hover_text(st.describe()).clicked() {
                    app.cfg.radio.strength = st;
                    d = true;
                }
            }
        });
        widgets::note(ui, app.cfg.radio.strength.describe());
        ui.add_space(6.0);
        let mut target = app.cfg.radio.target as u32;
        let mut thr = app.cfg.radio.threshold as u32;
        slider_u(ui, "Target queue size", &mut target, 5..=50, &mut d);
        slider_u(ui, "Refill when fewer than", &mut thr, 1..=(target.saturating_sub(1)).max(1), &mut d);
        app.cfg.radio.target = target as usize;
        app.cfg.radio.threshold = thr as usize;
        ui.horizontal(|ui| {
            ui.label("Preferred provider");
            egui::ComboBox::from_id_salt("radio_provider").selected_text(app.cfg.radio.provider.label()).show_ui(ui, |ui| {
                for pv in [crate::radio::RadioProvider::Automatic, crate::radio::RadioProvider::Spotify, crate::radio::RadioProvider::Local] {
                    if ui.selectable_label(app.cfg.radio.provider == pv, pv.label()).clicked() {
                        app.cfg.radio.provider = pv;
                        d = true;
                    }
                }
            });
        });
        widgets::note(ui, "Automatic: your local library first, then Spotify when Premium playback is ready.");
    });
    toggle_card(app, ui, "Rules", &["radio_prefer_unheard", "radio_avoid_recent", "radio_avoid_duplicates", "radio_repeat_artists", "radio_learn", "radio_stay_close"]);
    widgets::card(ui, Some("Recommendation source"), |ui| {
        if app.lastfm_key.is_empty() {
            widgets::warn(ui, "No Last.fm API key: the radio falls back to songs by the same artist / from the same album. Add a key for real similarity recommendations.");
        } else {
            widgets::ok(ui, if crate::security::persistent() { "Last.fm API key stored (Windows Credential Manager)." } else { "Last.fm API key set for this session (development build: not saved to disk)." });
        }
        ui.horizontal(|ui| {
            ui.label("Last.fm API key");
            ui.add(egui::TextEdit::singleline(&mut app.lastfm_input).password(true).desired_width(260.0).hint_text(if app.lastfm_key.is_empty() { "32-character key" } else { "replace stored key" }));
            if ui.button("Save").clicked() && !app.lastfm_input.trim().is_empty() {
                let k = app.lastfm_input.trim().to_string();
                match crate::security::set(crate::security::keys::LASTFM_API_KEY, &k) {
                    Ok(()) => {
                        app.lastfm_key = k;
                        app.lastfm_input.clear();
                        app.toast("Last.fm key saved.", false);
                    }
                    Err(e) => app.toast(format!("Could not store key: {e}"), true),
                }
            }
            if !app.lastfm_key.is_empty() && ui.button("Remove").clicked() {
                crate::security::delete(crate::security::keys::LASTFM_API_KEY);
                app.lastfm_key.clear();
            }
        });
        widgets::note(ui, "Free key: last.fm/api/account/create. Aqua Glass sends only the seed song's artist and title (and related artist names) to Last.fm; no account or scrobbling is involved.");
        ui.add_space(6.0);
        ui.label(RichText::new("Live self-test").strong());
        widgets::note(ui, "Queries the real Last.fm API with your key, runs all four discovery modes and checks the results. The report is also saved to the data folder.");
        ui.horizontal(|ui| {
            if app.settings.selftest_artist.is_empty() {
                let (a, t) = app.state.current.as_ref().map(|t| (t.artists.first().cloned().unwrap_or_default(), t.title.clone())).filter(|(a, _)| !a.is_empty()).unwrap_or(("Radiohead".into(), "Karma Police".into()));
                app.settings.selftest_artist = a;
                app.settings.selftest_title = t;
            }
            ui.add(egui::TextEdit::singleline(&mut app.settings.selftest_artist).hint_text("Artist").desired_width(150.0));
            ui.add(egui::TextEdit::singleline(&mut app.settings.selftest_title).hint_text("Title").desired_width(170.0));
            let running = app.jobs.is_running("Last.fm self-test");
            if running {
                ui.spinner();
            } else if ui.add_enabled(!app.lastfm_key.is_empty(), egui::Button::new("Run self-test")).clicked() {
                let (key, a, t) = (app.lastfm_key.clone(), app.settings.selftest_artist.clone(), app.settings.selftest_title.clone());
                let lib = app.library.clone();
                let out = app.settings.selftest_result.clone();
                app.jobs.spawn("Last.fm self-test", move |h| {
                    let local = lib.lock().unwrap().tracks(ProviderKind::Local, crate::library::LOCAL_ALL_ID).cloned().unwrap_or_default();
                    let r = crate::radio::self_test(&key, &a, &t, &local);
                    let path = crate::util::data_dir().join(format!("radio-selftest-{}.txt", crate::util::now_unix()));
                    let _ = crate::util::atomic_write(&path, r.report.as_bytes());
                    let summary = format!("Last.fm self-test: {} passed, {} failed — report saved to {}", r.passed, r.failed, path.display());
                    let failed = !r.ok_or_empty();
                    *out.lock().unwrap() = Some((r.report, path.clone()));
                    h.send(super::jobs::Msg::Toast(summary, failed));
                });
            }
        });
        let res = app.settings.selftest_result.lock().unwrap().clone();
        if let Some((rep, path)) = res {
            egui::CollapsingHeader::new("Last self-test report").default_open(true).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("selftest_report").min_scrolled_height(220.0).max_height(320.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&rep).monospace().size(11.5)).wrap());
                });
                let mut copied = false;
                ui.horizontal(|ui| {
                    if ui.small_button("Copy report").clicked() {
                        ui.ctx().copy_text(rep.clone());
                        copied = true;
                    }
                    if ui.small_button("Open report file").on_hover_text(path.display().to_string()).clicked() {
                        let _ = open::that_detached(&path);
                    }
                });
                if copied {
                    app.toast("Self-test report copied to the clipboard.", false);
                }
            });
        }
        if ui.button("Clear listening history").on_hover_text("Used for Recently Played, Prefer Unheard and Avoid Recently Played; stored only on this PC").clicked() {
            app.history.lock().unwrap().clear();
            app.toast("Listening history cleared.", false);
        }
    });
    if d {
        app.mark_dirty();
    }
}

const EQ_PRESETS: &[(&str, [f32; 10])] = &[
    ("Flat", [0.0; 10]),
    ("Bass", [5.0, 4.0, 3.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.5, 4.5, 5.0]),
    ("Vocal", [-2.0, -1.0, 0.0, 1.5, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0]),
    ("Rock", [4.0, 3.0, 1.0, -1.0, -1.5, 0.0, 1.5, 3.0, 3.5, 4.0]),
    ("Electronic", [4.5, 3.5, 1.0, 0.0, -1.5, 1.0, 0.5, 1.5, 3.5, 4.5]),
    ("Acoustic", [3.0, 2.5, 1.5, 0.5, 1.0, 1.0, 2.0, 2.5, 2.0, 1.0]),
    ("Loudness", [5.0, 3.0, 0.0, 0.0, -1.0, 0.0, -0.5, -2.0, 3.0, 2.0]),
];

fn audio(app: &mut AquaApp, ui: &mut Ui) {
    let mut d = false;
    ui.horizontal(|ui| {
        ui.label("Output device");
        let cur = if app.cfg.audio.device.is_empty() { "System default".to_string() } else { app.cfg.audio.device.clone() };
        egui::ComboBox::from_id_salt("dev").selected_text(cur).show_ui(ui, |ui| {
            if ui.selectable_label(app.cfg.audio.device.is_empty(), "System default").clicked() {
                app.cfg.audio.device.clear();
                d = true;
            }
            let devs = app.settings.devices.get_or_insert_with(crate::audio::output::list_devices).clone();
            for dv in devs {
                if ui.selectable_label(app.cfg.audio.device == dv, &dv).clicked() {
                    app.cfg.audio.device = dv;
                    d = true;
                }
            }
        });
        if ui.small_button("↻").on_hover_text("Refresh device list").clicked() {
            app.settings.devices = None;
        }
    });
    let dn = app.out.s.device_name.lock().unwrap().clone();
    widgets::note(ui, &format!("Active: {dn} @ {} Hz (shared mode). Device loss is detected and playback moves to the default device.", app.out.rate()));
    if let Some(e) = app.out.s.device_error.lock().unwrap().clone() {
        widgets::error(ui, &format!("Audio device problem: {e}"));
    }
    slider_u(ui, "Audio buffer (ms)", &mut app.cfg.audio.buffer_ms, 100..=3000, &mut d);
    slider_f(ui, "Stereo balance (L ↔ R)", &mut app.cfg.audio.balance, -1.0..=1.0, &mut d);
    slider_f(ui, "Pre-amp (dB)", &mut app.cfg.audio.preamp_db, -12.0..=6.0, &mut d);
    slider_f(ui, "Bass boost amount (dB)", &mut app.cfg.audio.bass_db, 0.0..=15.0, &mut d);
    widgets::section(ui, "Equalizer");
    ui.horizontal_wrapped(|ui| {
        for (name, g) in EQ_PRESETS {
            if ui.selectable_label(app.cfg.audio.eq_preset == *name, *name).clicked() {
                app.cfg.audio.eq_preset = name.to_string();
                app.cfg.audio.eq_gains_db = *g;
                d = true;
            }
        }
    });
    ui.horizontal(|ui| {
        for (i, f) in EQ_FREQS.iter().enumerate() {
            ui.vertical(|ui| {
                if ui.add(egui::Slider::new(&mut app.cfg.audio.eq_gains_db[i], -12.0..=12.0).vertical().show_value(false)).changed() {
                    app.cfg.audio.eq_preset = "Custom".into();
                    d = true;
                }
                ui.label(RichText::new(if *f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") }).small());
                ui.label(RichText::new(format!("{:+.0}", app.cfg.audio.eq_gains_db[i])).small().weak());
            });
        }
    });
    if d {
        app.mark_dirty();
    }
    widgets::section(ui, "Optional audio processing");
    toggles(app, ui, Category::Audio);
    widgets::section(ui, "Provider support");
    egui::Grid::new("audio_support").striped(true).show(ui, |ui| {
        ui.label(RichText::new("Feature").strong());
        ui.label(RichText::new("Spotify").strong());
        ui.label(RichText::new("Local files").strong());
        ui.end_row();
        for (f, s, l) in [
            ("EQ, bass, balance, visualizer", "✔", "✔"),
            ("Normalization", "✔ Spotify gain data", "✔ ReplayGain tags"),
            ("Gapless", "✔", "✔"),
            ("Quality setting", "✔ 96/160/320", "original file"),
            ("Crossfade", "✖ not implemented", "✖ not implemented"),
            ("Exclusive (WASAPI) output", "✖ not supported by the audio backend", "✖"),
        ] {
            ui.label(f);
            ui.label(s);
            ui.label(l);
            ui.end_row();
        }
    });
}

fn lyrics(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Lyrics);
    let mut d = false;
    let l = &mut app.cfg.lyrics;
    widgets::section(ui, "Lyrics style");
    slider_f(ui, "Font size", &mut l.font_size, 10.0..=48.0, &mut d);
    slider_f(ui, "Line spacing", &mut l.line_spacing, 0.0..=40.0, &mut d);
    slider_f(ui, "Background opacity", &mut l.bg_opacity, 0.0..=1.0, &mut d);
    ui.horizontal(|ui| {
        ui.label("Text color");
        d |= ui.color_edit_button_srgb(&mut l.color).changed();
        ui.label("Highlight color");
        d |= ui.color_edit_button_srgb(&mut l.highlight).changed();
        d |= ui.checkbox(&mut l.bold_current, "Bold current line").changed();
    });
    ui.horizontal(|ui| {
        ui.label("Alignment");
        for (n, v) in [("Left", Align::Left), ("Center", Align::Center), ("Right", Align::Right)] {
            if ui.selectable_label(l.align == v, n).clicked() {
                l.align = v;
                d = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Timing offset (ms)");
        d |= ui.add(egui::DragValue::new(&mut l.offset_ms).range(-10_000..=10_000).speed(10)).changed();
    });
    widgets::note(ui, "Lyrics come from LRCLIB. Translations are not available from this source. Lyric fonts follow the interface font (Appearance).");
    if d {
        app.mark_dirty();
    }
}

fn visualizer(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Visualizer);
    let mut d = false;
    let v = &mut app.cfg.visualizer;
    widgets::section(ui, "Visualizer options");
    ui.horizontal(|ui| {
        ui.label("Mode");
        for (n, m) in [("Spectrum bars", VisMode::Bars), ("Waveform", VisMode::Waveform), ("Circular", VisMode::Circular), ("Minimal", VisMode::Minimal)] {
            if ui.selectable_label(v.mode == m, n).clicked() {
                v.mode = m;
                d = true;
            }
        }
    });
    widgets::note(ui, "All modes share the same analyzer; RAM cost is the same for every mode (one 2048-point FFT, ~50 KB).");
    slider_u(ui, "FPS", &mut v.fps, 5..=144, &mut d);
    slider_f(ui, "Sensitivity", &mut v.sensitivity, 0.2..=3.0, &mut d);
    slider_f(ui, "Smoothing", &mut v.smoothing, 0.0..=0.95, &mut d);
    let mut bars = v.bars as u32;
    slider_u(ui, "Bars", &mut bars, 8..=128, &mut d);
    v.bars = bars as usize;
    slider_f(ui, "Height (size)", &mut v.height, 40.0..=400.0, &mut d);
    ui.horizontal(|ui| {
        ui.label("Colors");
        d |= ui.color_edit_button_srgb(&mut v.color_a).changed();
        d |= ui.color_edit_button_srgb(&mut v.color_b).changed();
    });
    widgets::note(ui, "Position: on the Now Playing page, and optionally in the player bar (“Visualizer in Player Bar”).");
    if d {
        app.mark_dirty();
    }
}

fn discord(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Discord);
    widgets::section(ui, "Discord application");
    ui.horizontal(|ui| {
        ui.label("Application ID");
        if ui.add(egui::TextEdit::singleline(&mut app.settings.discord_app_id).desired_width(220.0)).changed() {
            app.cfg.services.discord_app_id = app.settings.discord_app_id.trim().to_string();
            app.mark_dirty();
        }
    });
    widgets::note(ui, "Discord shows the application's name as the activity title. Create a free application named “Aqua Glass” at discord.com/developers/applications and paste its Application ID. Nothing is sent anywhere except to your local Discord app.");
    if app.cfg.active("discord") && app.settings.discord_app_id.trim().is_empty() {
        widgets::warn(ui, "Rich Presence is enabled but no Application ID is set.");
    }
}

fn shortcuts_cat(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Shortcuts);
    for e in app.ints.hotkey_errors.clone() {
        widgets::warn(ui, &e);
    }
    widgets::section(ui, "Shortcut editor");
    widgets::note(ui, "Format: Ctrl+Alt+P, Shift+F13, MediaPlayPause … Macro pads that send key combinations (e.g. F13–F24) work directly. Leave empty to unbind. Ctrl+, always opens Settings.");
    let conflicts = shortcuts::conflicts(&app.cfg.shortcuts);
    let mut d = false;
    egui::Grid::new("sc").num_columns(3).striped(true).show(ui, |ui| {
        for a in Action::ALL {
            ui.label(a.label());
            let mut s = app.cfg.shortcuts.get(a.id()).cloned().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(170.0)).changed() {
                app.cfg.shortcuts.insert(a.id().into(), s.clone());
                d = true;
            }
            let status = if s.trim().is_empty() {
                RichText::new("unbound").weak()
            } else {
                match Binding::parse(&s) {
                    Err(e) => RichText::new(format!("✖ {e}")).color(widgets::impact_color(features::Impact::VeryHigh)),
                    Ok(b) => {
                        if conflicts.iter().any(|(_, v)| v.iter().any(|x| x == a.id())) {
                            RichText::new("⚠ conflicts with another action").color(widgets::impact_color(features::Impact::High))
                        } else if let Some(r) = shortcuts::reserved_conflict(&b) {
                            RichText::new(format!("⚠ {r}")).color(widgets::impact_color(features::Impact::High))
                        } else {
                            RichText::new("✔").color(widgets::impact_color(features::Impact::Low))
                        }
                    }
                }
            };
            ui.label(status);
            ui.end_row();
        }
    });
    if ui.button("Reset all shortcuts").clicked() {
        app.cfg.shortcuts = shortcuts::default_bindings();
        d = true;
    }
    if d {
        app.mark_dirty();
    }
}

fn mini(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::MiniPlayer);
    let mut d = false;
    widgets::section(ui, "Mini-player");
    ui.horizontal(|ui| {
        ui.label("Layout");
        for (n, v) in [("Horizontal", MiniLayout::Horizontal), ("Square", MiniLayout::Square), ("Compact", MiniLayout::Compact)] {
            if ui.selectable_label(app.cfg.mini.layout == v, n).clicked() {
                app.cfg.mini.layout = v;
                d = true;
            }
        }
    });
    slider_f(ui, "Opacity", &mut app.cfg.mini.opacity, 0.2..=1.0, &mut d);
    if ui.button(if app.mini_open.load(std::sync::atomic::Ordering::Relaxed) { "Close mini-player" } else { "Open mini-player" }).clicked() {
        app.do_action(Action::ToggleMiniPlayer, &ui.ctx().clone());
    }
    widgets::note(ui, "The mini-player remembers its position and size.");
    widgets::section(ui, "Desktop overlay");
    let o = &mut app.cfg.overlay;
    slider_f(ui, "X position", &mut o.pos[0], 0.0..=3840.0, &mut d);
    slider_f(ui, "Y position", &mut o.pos[1], 0.0..=2160.0, &mut d);
    slider_f(ui, "Width", &mut o.size[0], 200.0..=1200.0, &mut d);
    slider_f(ui, "Height", &mut o.size[1], 50.0..=400.0, &mut d);
    slider_f(ui, "Opacity", &mut o.opacity, 0.1..=1.0, &mut d);
    slider_f(ui, "Font size", &mut o.font_size, 10.0..=48.0, &mut d);
    d |= ui.checkbox(&mut o.always_on_top, "Always on top").changed();
    if d {
        app.mark_dirty();
    }
}

const SOLOIST_DOWNLOADS: &str = "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";
const SOLOIST_KEY_PAGE: &str = "https://developer.spotify.com/dashboard/soloist";

/// "Spotify playback" card: backend choice and Spotify Soloist setup. Returns "dirty".
fn spotify_playback_settings(app: &mut AquaApp, ui: &mut Ui) -> bool {
    use crate::config::SpotifyBackend;
    let mut d = false;
    ui.add_space(6.0);
    ui.label(RichText::new("Spotify playback").strong());
    ui.horizontal(|ui| {
        for b in [SpotifyBackend::Soloist, SpotifyBackend::Librespot] {
            if ui.selectable_label(app.cfg.services.spotify_backend == b, b.label()).clicked() {
                app.cfg.services.spotify_backend = b;
                d = true;
            }
        }
    });
    if app.cfg.services.spotify_backend == SpotifyBackend::Librespot {
        widgets::note(ui, "librespot is an unofficial open-source player built into Aqua Glass. Since late 2025 Spotify refuses its audio-decryption keys for many accounts (songs then skip without playing); if that happens, switch to Spotify Soloist. Spotify's terms restrict third-party clients — use at your own discretion.");
        return d;
    }
    widgets::note(ui, "Spotify Soloist is Spotify's official headless player. It runs as a separate background program (no window) and plays the audio; Aqua Glass stays your only music interface and keeps its queue, Similarity Radio and controls. EQ, visualizer and normalization don't apply to Soloist's audio. Aqua Glass never bundles or redistributes Soloist.");
    egui::CollapsingHeader::new(RichText::new("Set up Spotify Soloist (one time, about 5 minutes)").strong())
        .default_open(app.cfg.services.soloist_path.trim().is_empty() || crate::security::get(crate::security::keys::SOLOIST_API_KEY).is_none())
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("1. Download the Windows x86_64 build from Spotify and extract the ZIP (e.g. to C:\\Tools\\Soloist).");
                if ui.small_button("Open downloads page").clicked() {
                    let _ = open::that_detached(SOLOIST_DOWNLOADS);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("2. Generate your Soloist API key (needs Spotify Premium; it's for you only — never share it).");
                if ui.small_button("Open API key page").clicked() {
                    let _ = open::that_detached(SOLOIST_KEY_PAGE);
                }
            });
            ui.label("3. Choose soloist.exe and store the key below. Aqua Glass starts Soloist in the background.");
            ui.label("4. Pair once: open Spotify on your phone or PC (same network), tap the devices icon and choose the device name below. Soloist remembers it.");
            widgets::note(ui, "Soloist builds stop working 90 days after their build date. Aqua Glass shows the days left here and warns two weeks ahead; to update, download the new ZIP and replace soloist.exe (keep the same folder setting so it stays paired).");
        });
    // soloist.exe
    ui.horizontal(|ui| {
        ui.label("soloist.exe");
        d |= ui.add(egui::TextEdit::singleline(&mut app.cfg.services.soloist_path).hint_text("C:\\Tools\\Soloist\\soloist.exe").desired_width(320.0)).changed();
        if ui.button("Browse…").clicked() {
            if let Some(p) = super::pages::pick_open(&["exe"]) {
                app.cfg.services.soloist_path = p.to_string_lossy().to_string();
                d = true;
            }
        }
    });
    let path = app.cfg.services.soloist_path.trim().to_string();
    if !path.is_empty() {
        if app.settings.soloist_version.as_ref().map(|(p, _)| p != &path).unwrap_or(true) {
            let r = crate::audio::soloist::version(std::path::Path::new(&path));
            app.settings.soloist_version = Some((path.clone(), r));
        }
        match &app.settings.soloist_version.as_ref().unwrap().1 {
            Ok((v, built)) => {
                let left = crate::audio::soloist::days_left(*built, crate::util::now_unix());
                let ver = v.split_whitespace().nth(1).unwrap_or("").to_string();
                if left < 0 {
                    widgets::warn(ui, &format!("Soloist {ver} has expired. Download the current build and replace soloist.exe."));
                } else if left <= 14 {
                    widgets::warn(ui, &format!("Soloist {ver} expires in {left} day(s) — download the current build soon and replace soloist.exe."));
                } else {
                    widgets::ok(ui, &format!("Soloist {ver} · expires in {left} days"));
                }
            }
            Err(e) => widgets::warn(ui, &format!("Can't use this file: {e}")),
        }
    }
    // API key (Credential Manager)
    ui.horizontal(|ui| {
        ui.label("API key");
        let have = crate::security::get(crate::security::keys::SOLOIST_API_KEY).is_some();
        ui.add(egui::TextEdit::singleline(&mut app.settings.soloist_key_input).password(true).hint_text(if have { "stored in Credential Manager" } else { "paste your Soloist API key" }).desired_width(260.0));
        if ui.add_enabled(!app.settings.soloist_key_input.trim().is_empty(), egui::Button::new("Save securely")).clicked() {
            match crate::security::set(crate::security::keys::SOLOIST_API_KEY, app.settings.soloist_key_input.trim()) {
                Ok(()) => app.toast("Soloist API key stored in Windows Credential Manager.", false),
                Err(e) => app.toast(format!("Could not store the key: {e}"), true),
            }
            app.settings.soloist_key_input.clear();
            app.ensure_soloist(true);
        }
        if have && ui.small_button("Remove").clicked() {
            crate::security::delete(crate::security::keys::SOLOIST_API_KEY);
            app.ensure_soloist(true);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Device name");
        d |= ui.add(egui::TextEdit::singleline(&mut app.cfg.services.soloist_device_name).desired_width(180.0)).changed();
        if ui.button("Restart Soloist").clicked() {
            app.ensure_soloist(true);
        }
    });
    d |= ui.checkbox(&mut app.cfg.services.soloist_manage, "Start and stop Soloist with Aqua Glass (off = use a Soloist you run yourself with --ws 127.0.0.1:0)").changed();
    ui.horizontal(|ui| {
        ui.label("Soloist data folder");
        let hint = crate::util::data_dir().join("soloist").to_string_lossy().to_string();
        d |= ui.add(egui::TextEdit::singleline(&mut app.cfg.services.soloist_data_dir).hint_text(hint).desired_width(320.0)).changed();
    });
    if let Some(pid) = app.state.helper_pid {
        if let Some(b) = crate::perf::process_memory_bytes(pid) {
            ui.label(RichText::new(format!("Soloist is running (process {pid}) · {:.0} MB", b as f64 / 1_048_576.0)).weak());
        }
    }
    widgets::note(ui, "Security: Soloist's local control socket has no password, so Aqua Glass always binds it to 127.0.0.1 and refuses other addresses. Soloist only accepts its API key as a command-line argument, so other programs on this PC that can list processes could read it while Soloist runs. Soloist also announces itself on your local network so the Spotify app can find it for pairing.");
    d
}

fn services(app: &mut AquaApp, ui: &mut Ui) {
    let mut d = false;
    // Spotify
    widgets::section(ui, "Spotify");
    d |= ui.checkbox(&mut app.cfg.services.spotify_enabled, "Enable Spotify").changed();
    widgets::note(ui, "RAM: LOW when signed out. Disabled providers do no background activity.");
    if app.cfg.services.spotify_enabled {
        ui.label(format!("Status: {}", app.sp_status));
        let backend_name = match app.cfg.services.spotify_backend {
            crate::config::SpotifyBackend::Soloist => "Spotify Soloist",
            crate::config::SpotifyBackend::Librespot => "librespot",
        };
        match &app.state.spotify {
            crate::audio::engine::SpotifyStatus::Ready => widgets::ok(ui, &format!("Premium playback ready ({backend_name})")),
            crate::audio::engine::SpotifyStatus::Unavailable(e) => widgets::warn(ui, &format!("Playback unavailable: {e}")),
            crate::audio::engine::SpotifyStatus::Connecting => {
                ui.label(format!("Connecting playback ({backend_name})…"));
            }
            _ => {}
        }
        ui.horizontal(|ui| {
            if app.spotify.is_signed_in() {
                if ui.button("Disconnect…").clicked() {
                    app.confirm = Some(Confirm::DisconnectSpotify);
                }
                if ui.button(format!("{} Sync now", icon::SYNC())).clicked() {
                    app.sync_now(ProviderKind::Spotify, false);
                }
            } else if ui.button("Sign in with Spotify").clicked() {
                app.spotify_login();
            }
        });
        // live rate-limit status
        let rl = crate::services::ratelimit::status("spotify.com");
        if let Some(left) = rl.cooldown_left {
            widgets::warn(ui, &format!("Spotify asked Aqua Glass to slow down — requests resume in {} s. Playback is not affected.", left.as_secs().max(1)));
        } else if rl.total_429 > 0 {
            widgets::note(ui, &format!("Spotify rate-limited this session {} time(s); Aqua Glass paused as requested each time.", rl.total_429));
        }
        d |= spotify_playback_settings(app, ui);
        if app.cfg.services.spotify_backend == crate::config::SpotifyBackend::Librespot && app.spotify.is_signed_in() && !app.spotify.playback_signed_in() {
            ui.horizontal(|ui| {
                widgets::warn(ui, "Playback isn't signed in yet (needed once when using your own app).");
                if ui.button("Sign in for playback").clicked() {
                    let api = app.spotify.clone();
                    app.jobs.spawn("Spotify: playback sign-in", move |h| {
                        let m = match api.login_playback() {
                            Ok(()) => super::jobs::Msg::Toast("Playback signed in — restart Aqua Glass to connect playback.".into(), false),
                            Err(e) => super::jobs::Msg::Toast(format!("Playback sign-in failed: {e}"), true),
                        };
                        h.send(m);
                    });
                }
            });
        }
        let own = !app.cfg.services.spotify_client_id.trim().is_empty();
        let open = rl.total_429 > 0 || own;
        egui::CollapsingHeader::new(RichText::new(if own { "Your own Spotify app (in use)" } else { "Use your own Spotify app — recommended if you see rate limiting" }).strong())
            .default_open(open)
            .show(ui, |ui| {
                widgets::note(ui, "Spotify counts Web API requests per app. Without your own app, Aqua Glass uses librespot's public app, whose quota is shared with every librespot-based player — the most common cause of rate limiting. With your own app, library, search and sync use your private quota; Premium playback keeps using librespot.");
                ui.label(RichText::new("One-time setup (about 3 minutes):").strong());
                ui.label("1. Open developer.spotify.com/dashboard and sign in (the app owner needs Spotify Premium since Feb 2026).");
                ui.label(format!("2. Create app › any name › Redirect URI: http://127.0.0.1:{}/login › tick “Web API” › Save.", app.cfg.services.spotify_redirect_port));
                ui.label("3. Open the app's Settings, copy the Client ID and paste it below.");
                ui.label("4. Restart Aqua Glass and sign in again (twice in a row: once for your app, once for playback).");
                ui.horizontal(|ui| {
                    ui.label("Client ID");
                    d |= ui.add(egui::TextEdit::singleline(&mut app.cfg.services.spotify_client_id).hint_text("empty = librespot's shared app").desired_width(300.0)).changed();
                    if ui.small_button("Open dashboard").clicked() {
                        let _ = open::that_detached("https://developer.spotify.com/dashboard");
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Redirect port");
                    d |= ui.add(egui::DragValue::new(&mut app.cfg.services.spotify_redirect_port).range(1024..=65535)).changed();
                });
                let configured = app.cfg.services.spotify_client_id.trim().to_string();
                let active_custom = !app.spotify.uses_default_client();
                if configured.is_empty() == active_custom {
                    widgets::warn(ui, "Restart Aqua Glass to apply the client ID change, then sign in again.");
                }
                widgets::note(ui, "Development-Mode apps allow up to 5 users — plenty for personal use. Nothing about your account changes; you can delete the app any time.");
            });
        widgets::note(ui, "Playback requires Spotify Premium (enforced by Spotify). Library import, sync, export and transfer work with Free accounts.");
    }

    // Local
    widgets::section(ui, "Local files");
    d |= ui.checkbox(&mut app.cfg.services.local_enabled, "Enable local files").changed();
    if app.cfg.services.local_enabled {
        let mut remove = None;
        for (i, f) in app.cfg.services.local_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(f);
                if ui.small_button("✖").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            app.cfg.services.local_folders.remove(i);
            d = true;
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.settings.status).hint_text("C:\\Users\\you\\Music").desired_width(280.0));
            if ui.button("Add folder").clicked() {
                let p = if app.settings.status.trim().is_empty() { super::pages::pick_folder().map(|p| p.to_string_lossy().to_string()) } else { Some(app.settings.status.trim().to_string()) };
                if let Some(p) = p {
                    if std::path::Path::new(&p).is_dir() {
                        app.cfg.services.local_folders.push(p);
                        app.settings.status.clear();
                        d = true;
                        app.mark_dirty();
                        app.scan_local();
                    } else {
                        app.toast(format!("Not a folder: {p}"), true);
                    }
                }
            }
            if ui.button("Rescan").clicked() {
                app.scan_local();
            }
        });
        widgets::note(ui, "MP3, FLAC, Ogg Vorbis, WAV and AAC/M4A are played natively through Aqua Glass' audio engine.");
    }
    if d {
        app.mark_dirty();
    }
}

fn library(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Library);
    let mut d = false;
    widgets::section(ui, "Synchronization");
    let mut iv = app.cfg.sync.interval_min;
    slider_u(ui, "Automatic sync interval (minutes)", &mut iv, 15..=1440, &mut d);
    app.cfg.sync.interval_min = iv;
    d |= ui.checkbox(&mut app.cfg.sync.include_followed_playlists, "Include playlists I follow (not only my own)").changed();
    for p in [ProviderKind::Spotify] {
        widgets::section(ui, &format!("{} library", p.label()));
        let (last, err, mut il, mut ia, mut ir, conflicts) = {
            let mut s = app.library.lock().unwrap();
            let l = s.lib(p);
            (l.last_sync, l.last_error.clone(), l.import_liked, l.import_albums, l.import_artists, l.conflicts.clone())
        };
        let running = app.jobs.is_running(&format!("Sync {}", p.label()));
        ui.horizontal(|ui| {
            ui.label(format!("Last successful sync: {}", crate::util::fmt_unix(last)));
            if running {
                ui.spinner();
                ui.label("syncing…");
            } else if ui.button(format!("{} Sync Now", icon::SYNC())).clicked() {
                app.sync_now(p, false);
            }
        });
        if !err.is_empty() {
            widgets::error(ui, &err);
        }
        let mut ch = false;
        ch |= ui.checkbox(&mut il, if p == ProviderKind::Spotify { "Import Liked Songs" } else { "Import Liked Music" }).changed();
        if p == ProviderKind::Spotify {
            ch |= ui.checkbox(&mut ia, "Import saved albums").changed();
            ch |= ui.checkbox(&mut ir, "Import followed artists").changed();
        }
        if ch {
            let mut s = app.library.lock().unwrap();
            let l = s.lib(p);
            l.import_liked = il;
            l.import_albums = ia;
            l.import_artists = ir;
            s.save_index(p);
        }
        ui.horizontal(|ui| {
            if ui.button("Import all playlists").clicked() {
                {
                    let mut s = app.library.lock().unwrap();
                    for e in s.lib(p).playlists.iter_mut() {
                        e.imported = true;
                        e.meta.snapshot.clear();
                    }
                }
                app.sync_now(p, true);
            }
            if ui.button("Choose playlists to import…").clicked() {
                app.navigate(Page::Library);
            }
        });
        if !conflicts.is_empty() {
            ui.label(RichText::new("Needs review").strong());
            for c in conflicts {
                ui.horizontal(|ui| {
                    match c.kind {
                        crate::library::ConflictKind::RemovedUpstream => widgets::warn(ui, &format!("“{}” was deleted or unfollowed on {}. The local copy is kept.", c.name, p.label())),
                        crate::library::ConflictKind::LargeShrink { before, after } => widgets::warn(ui, &format!("“{}” shrank from {before} to {after} tracks on {}. The previous copy was backed up.", c.name, p.label())),
                    }
                    if ui.small_button("Keep / acknowledge").clicked() {
                        let mut s = app.library.lock().unwrap();
                        s.lib(p).conflicts.retain(|x| x != &c);
                        s.save_index(p);
                    }
                    if matches!(c.kind, crate::library::ConflictKind::RemovedUpstream) && ui.small_button("Remove local copy").clicked() {
                        app.confirm = Some(Confirm::RemoveImported(p, c.playlist_id.clone(), c.name.clone()));
                    }
                });
            }
        }
    }
    widgets::note(ui, "Sync is read-only: Aqua Glass never deletes or edits tracks or playlists on your accounts. Unchanged playlists are skipped using their snapshot IDs to save API calls; 429 rate limits are honoured.");
    if d {
        app.mark_dirty();
    }
}

fn transfer(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Transfer);
    if ui.button(format!("{} Open Playlist Transfer", icon::TRANSFER())).clicked() {
        app.navigate(Page::Transfer);
    }
    widgets::note(ui, "Transfers never modify the source playlist. A JSON backup of the source and a log of each transfer are written to the transfers folder.");
    if ui.button("Open transfers folder").clicked() {
        let _ = crate::util::ensure_dir(&crate::transfer::transfers_dir());
        let _ = open::that_detached(crate::transfer::transfers_dir());
    }
}

fn performance(app: &mut AquaApp, ui: &mut Ui) {
    let cur = app.monitor.current_mb();
    let peak = crate::perf::mb(app.monitor.peak_private_ws);
    let target = app.cfg.perf.ram_target_mb as f32;
    let accent = app.accent();
    widgets::card(ui, None, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("RAM usage").size(12.0).color(ui.visuals().weak_text_color()));
                ui.label(RichText::new(format!("{cur:.0} MB / {target:.0} MB")).size(24.0).strong());
            });
            ui.add_space(30.0);
            let stat = |ui: &mut Ui, k: &str, v: String| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(k).size(12.0).color(ui.visuals().weak_text_color()));
                    ui.label(RichText::new(v).size(17.0).strong());
                });
                ui.add_space(22.0);
            };
            let rem = target - cur;
            stat(ui, "Remaining", if rem >= 0.0 { format!("{rem:.0} MB") } else { format!("{:.0} MB over", -rem) });
            stat(ui, "Peak", format!("{peak:.0} MB"));
            stat(ui, "CPU", format!("{:.1} %", app.monitor.cpu_percent));
            stat(
                ui,
                "Memory saving",
                match (app.cfg.active("auto_optimizer"), app.pressure) {
                    (false, _) => "Off".into(),
                    (true, crate::perf::Pressure::Normal) => "On · idle".into(),
                    (true, crate::perf::Pressure::Elevated) => "On · trimming caches".into(),
                    (true, crate::perf::Pressure::Over) => "On · saving hard".into(),
                },
            );
        });
        widgets::budget_bar(ui, cur, target, accent);
        if !app.monitor.last.private_exact {
            widgets::warn(ui, "This system does not report private memory; values show the full working set (including shared system libraries), which overstates Aqua Glass' own usage.");
        }
        widgets::sparkline(ui, &app.monitor.history.iter().copied().collect::<Vec<_>>(), target, accent);
        widgets::note(ui, "Measured live from the operating system (private working set).");
        if let Some(pid) = app.state.helper_pid {
            if let Some((private, ws)) = crate::perf::process_memory(pid) {
                let so = crate::perf::mb(private);
                let total = cur + so;
                ui.add_space(6.0);
                let what = if private == ws { format!("{so:.0} MB working set (this system doesn't report private memory)") } else { format!("{so:.0} MB private · {:.0} MB working set", crate::perf::mb(ws)) };
                ui.label(RichText::new(format!("Spotify Soloist (separate process): {what}")).strong());
                ui.label(format!("Aqua Glass + Soloist: {total:.0} MB of the {target:.0} MB target"));
                if total > target {
                    widgets::warn(ui, "Together the two processes are above the target. The target applies to Aqua Glass itself; Soloist's memory is managed by Spotify's program.");
                }
            }
        }
    });
    widgets::card(ui, Some("Caches and background work"), |ui| {
        let (art_n, art_b) = {
            let a = app.artwork.lock().unwrap();
            (a.len(), a.used_bytes)
        };
        egui::Grid::new("perf_caches").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            ui.label("Artwork cache (memory)");
            ui.label(format!("{art_n} images, {}", crate::util::fmt_bytes(art_b as u64)));
            ui.end_row();
            ui.label("Artwork cache (disk)");
            ui.label(crate::util::fmt_bytes(crate::artwork::disk_usage()));
            ui.end_row();
            ui.label("Library metadata in memory");
            ui.label(crate::util::fmt_bytes(app.library.lock().unwrap().loaded_bytes_estimate() as u64));
            ui.end_row();
            ui.label("Listening history");
            ui.label(format!("{} plays", app.history.lock().unwrap().events.len()));
            ui.end_row();
        });
        let mut tasks = vec!["audio output".to_string(), "playback engine".into(), "artwork loader".into()];
        if app.state.spotify == crate::audio::engine::SpotifyStatus::Ready {
            tasks.push("Spotify session".into());
        }
        if let Some(r) = app.radio_status() {
            tasks.push(if r.active { "Similarity Radio (active)".into() } else { "Similarity Radio (idle)".into() });
        }
        if app.ints.discord.lock().unwrap().is_some() {
            tasks.push("Discord presence".into());
        }
        if app.ints.hotkeys.is_some() {
            tasks.push("global hotkeys".into());
        }
        if app.cfg.active("auto_sync") {
            tasks.push(format!("auto-sync every {} min", app.cfg.sync.interval_min));
        }
        for j in app.jobs.active.lock().unwrap().values() {
            tasks.push(format!("{} ({}s)", j.label, j.started.elapsed().as_secs()));
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Background tasks").strong());
        widgets::note(ui, &tasks.join(" · "));
        ui.horizontal(|ui| {
            if ui.button("Clear artwork cache").clicked() {
                app.artwork.lock().unwrap().clear();
                crate::artwork::clear_disk();
            }
            if ui.button("Trim memory now").on_hover_text("Release caches and ask Windows to trim the working set").clicked() {
                app.artwork.lock().unwrap().clear();
                app.view.live.clear();
                app.library.lock().unwrap().evict_except(None);
                crate::perf::trim_working_set();
            }
        });
    });
    egui::CollapsingHeader::new(RichText::new("Detailed Diagnostics").strong()).default_open(false).show(ui, |ui| {
        let m = &app.monitor;
        egui::Grid::new("perf").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            let row = |ui: &mut Ui, k: &str, v: String| {
                ui.label(k);
                ui.label(RichText::new(v).monospace());
                ui.end_row();
            };
            row(ui, "Private working set", format!("{:.1} MB", crate::perf::mb(m.last.private_ws)));
            row(ui, "Private bytes (commit)", format!("{:.1} MB", crate::perf::mb(m.last.private_bytes)));
            row(ui, "Working set (incl. shared)", format!("{:.1} MB", crate::perf::mb(m.last.working_set)));
            row(ui, "Rendering FPS", format!("{:.0} (repaints only on change)", app.fps));
            row(ui, "Memory pressure", format!("{:?}", app.pressure));
            row(ui, "Helper processes", "none (all in-process)".into());
            let est = features::combined_estimate(&|id| app.cfg.active(id));
            row(ui, "Estimated cost of enabled features", format!("+{:.0}–{:.0} MB (estimate, shared resources counted once)", est.0, est.1));
        });
        for l in &app.optimizer_log {
            widgets::note(ui, &format!("optimizer: {l}"));
        }
    });
    ui.add_space(8.0);
    widgets::section(ui, "RAM target");
    ui.horizontal_wrapped(|ui| {
        for t in [100, 150, 200, 250, 300] {
            if ui.selectable_label(app.cfg.perf.ram_target_mb == t, format!("{t} MB")).clicked() {
                app.cfg.perf.ram_target_mb = t;
                app.mark_dirty();
            }
        }
        ui.label("Custom:");
        if ui.add(egui::DragValue::new(&mut app.cfg.perf.ram_target_mb).range(50..=4096).suffix(" MB")).changed() {
            app.mark_dirty();
        }
    });
    widgets::note(ui, "A soft target: Aqua Glass warns and optimizes but never stops playback, kills itself or discards your data to meet it.");

    widgets::section(ui, "Performance presets");
    ui.horizontal_wrapped(|ui| {
        for p in Preset::ALL {
            if p == Preset::Custom {
                ui.selectable_label(app.cfg.preset == p, p.label()).on_hover_text("Your manual configuration");
                continue;
            }
            if ui.selectable_label(app.cfg.preset == p, p.label()).clicked() {
                app.cfg.apply_preset(p);
                app.mark_dirty();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.settings.new_preset_name).hint_text("preset name").desired_width(150.0));
        if ui.button("Save current as preset").clicked() && !app.settings.new_preset_name.trim().is_empty() {
            app.cfg.user_presets.insert(app.settings.new_preset_name.trim().to_string(), app.cfg.features.clone());
            app.settings.new_preset_name.clear();
            app.mark_dirty();
        }
    });
    let names: Vec<String> = app.cfg.user_presets.keys().cloned().collect();
    for n in names {
        ui.horizontal(|ui| {
            ui.label(&n);
            if ui.small_button("Apply").clicked() {
                app.cfg.apply_user_preset(&n);
                app.mark_dirty();
            }
            if ui.small_button("Export…").clicked() {
                let def = crate::util::data_dir().join("exports").join(format!("preset-{n}.toml"));
                let p = super::pages::pick_save(&def, "toml").unwrap_or(def);
                let mut t = toml::Table::new();
                t.insert("features".into(), toml::Value::try_from(&app.cfg.user_presets[&n]).unwrap_or(toml::Value::Table(Default::default())));
                let _ = crate::util::atomic_write(&p, toml::to_string_pretty(&t).unwrap_or_default().as_bytes());
                app.toast(format!("Preset exported to {}", p.display()), false);
            }
            if ui.small_button("Delete").clicked() {
                app.cfg.user_presets.remove(&n);
                app.mark_dirty();
            }
        });
    }
    if ui.button("Import preset…").clicked() {
        if let Some(p) = super::pages::pick_open(&["toml"]) {
            let r: anyhow::Result<()> = (|| {
                let s = std::fs::read_to_string(&p)?;
                let t: toml::Table = toml::from_str(&s)?;
                let f: std::collections::BTreeMap<String, bool> = t.get("features").cloned().ok_or_else(|| anyhow::anyhow!("no [features]"))?.try_into()?;
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("Imported".into());
                app.cfg.user_presets.insert(name, f);
                Ok(())
            })();
            match r {
                Ok(()) => app.mark_dirty(),
                Err(e) => app.toast(format!("Invalid preset: {e}"), true),
            }
        }
    }

    widgets::section(ui, "Artwork");
    let mut d = false;
    slider_u(ui, "Artwork resolution (max px)", &mut app.cfg.perf.artwork_max_px, 64..=1024, &mut d);
    slider_u(ui, "Artwork memory cache (MB)", &mut app.cfg.perf.artwork_cache_mb, 2..=256, &mut d);
    ui.horizontal(|ui| {
        if ui.button("Clear artwork cache (memory + disk)").clicked() {
            app.artwork.lock().unwrap().clear();
            crate::artwork::clear_disk();
        }
        if ui.button("Trim memory now").on_hover_text("Release caches and ask Windows to trim the working set").clicked() {
            app.artwork.lock().unwrap().clear();
            app.view.live.clear();
            app.library.lock().unwrap().evict_except(None);
            crate::perf::trim_working_set();
        }
    });
    if d {
        app.mark_dirty();
    }
    widgets::section(ui, "Optional performance features");
    toggles(app, ui, Category::Performance);
    widgets::section(ui, "Measured feature costs on this PC (real measurements)");
    if app.measurements.by_feature.is_empty() {
        widgets::note(ui, "No measurements yet. Each time you enable a feature, Aqua Glass compares memory before and ~6 s after and records the difference here.");
    }
    let ms: Vec<(String, crate::perf::Measurement)> = app.measurements.by_feature.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    egui::Grid::new("meas").striped(true).show(ui, |ui| {
        for (k, m) in ms {
            ui.label(features::get(&k).map(|f| f.name).unwrap_or(&k));
            ui.label(format!("+{:.1} MB", m.delta_mb));
            ui.label(format!("{} sample(s)", m.samples));
            ui.label(crate::features::Impact::classify(m.delta_mb).label());
            ui.end_row();
        }
    });
    if ui.small_button("Clear measurements").clicked() {
        app.measurements = Default::default();
        app.measurements.save();
    }
}

fn privacy(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Privacy);
    widgets::section(ui, "Network activity");
    egui::Grid::new("net").striped(true).show(ui, |ui| {
        for (host, why, when) in [
            ("accounts.spotify.com, api.spotify.com", "Spotify sign-in, library, search, playlist creation", "Spotify enabled and signed in"),
            ("Spotify access points (librespot)", "Premium audio streaming", "when playing Spotify tracks"),
            ("i.scdn.co", "album artwork", "when covers are shown"),
            ("lrclib.net", "lyrics lookup (title, artist, album, duration)", "“Lyrics” enabled"),
            ("Local Discord app (IPC, no network)", "Rich Presence", "“Discord Rich Presence” enabled"),
        ] {
            ui.label(host);
            ui.label(why);
            ui.label(RichText::new(when).weak());
            ui.end_row();
        }
    });
    widgets::note(ui, "No telemetry, analytics or advertising SDKs. Your listening history is never sent anywhere except the service you play from (and Discord, if you enable it). Tokens are stored in Windows Credential Manager and are never logged or exported.");
    widgets::section(ui, "Data");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Clear artwork cache").clicked() {
            app.artwork.lock().unwrap().clear();
            crate::artwork::clear_disk();
            app.toast("Artwork cache cleared", false);
        }
        if ui.button("Clear lyrics cache").clicked() {
            app.lyrics.cache.clear();
        }
        if ui.button("Clear playback history (session)").clicked() {
            let _ = std::fs::remove_file(crate::util::data_dir().join("session.json"));
            app.library.lock().unwrap().lib(ProviderKind::Spotify).recent.clear();
        }
        if app.spotify.is_signed_in() && ui.button("Disconnect Spotify…").clicked() {
            app.confirm = Some(Confirm::DisconnectSpotify);
        }
        if ui.button("Open data folder").clicked() {
            let _ = open::that_detached(crate::util::data_dir());
        }
    });
    if !crate::security::persistent() {
        widgets::warn(ui, "Development build: credentials are kept in memory only (Credential Manager is Windows-only).");
    }
}

fn advanced(app: &mut AquaApp, ui: &mut Ui) {
    toggles(app, ui, Category::Advanced);
    egui::Grid::new("adv").num_columns(2).show(ui, |ui| {
        ui.label("Version");
        ui.label(env!("CARGO_PKG_VERSION"));
        ui.end_row();
        ui.label("Settings file");
        ui.label(Config::path().display().to_string());
        ui.end_row();
        ui.label("Data folder");
        ui.label(crate::util::data_dir().display().to_string());
        ui.end_row();
        ui.label("Compiled-in modules");
        let mut f = Vec::new();
        if cfg!(feature = "spotify-playback") {
            f.push("spotify-playback");
        }
        if cfg!(feature = "discord") {
            f.push("discord");
        }
        if cfg!(feature = "local-files") {
            f.push("local-files");
        }
        if cfg!(feature = "accessibility") {
            f.push("accessibility");
        }
        ui.label(f.join(", "));
        ui.end_row();
    });
    widgets::note(ui, "Turning a feature off at runtime stops its work and frees what it can. To leave a module out of the program entirely, build with cargo features (see README: e.g. --no-default-features --features local-files).");
    for e in app.ints.errors.clone() {
        widgets::warn(ui, &e);
    }
    ui.horizontal(|ui| {
        if ui.button("Restore all defaults…").clicked() {
            app.confirm = Some(Confirm::ResetAll);
        }
        if ui.button("Open settings file").clicked() {
            let _ = open::that_detached(Config::path());
        }
    });
}
