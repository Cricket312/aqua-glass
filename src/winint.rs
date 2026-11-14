//! Windows integration: system tray, Windows media controls (SMTC) + media
//! keys, global hotkeys, start-with-Windows, toast notifications and the
//! Windows 11 acrylic backdrop. Every piece is created only when its feature is
//! enabled and dropped (unregistered) when disabled. On non-Windows builds
//! (development only) these are no-ops.

use crate::audio::engine::{Cmd, Engine, PlayerState};
use crate::shortcuts::Action;
use std::sync::mpsc;

/// Requests from integrations to the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum UiCmd {
    Show,
    OpenSettings,
    ToggleMini,
    Quit,
    Action(Action),
}

pub type Wake = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Run an action that doesn't need the UI directly on the engine (works while minimized).
pub fn engine_action(engine: &Engine, a: Action, vol_step: f32, seek_ms: i64) -> bool {
    let st = engine.state.read().unwrap();
    let vol = st.volume;
    drop(st);
    match a {
        Action::PlayPause => engine.send(Cmd::Toggle),
        Action::Next => engine.send(Cmd::Next),
        Action::Previous => engine.send(Cmd::Prev),
        Action::VolumeUp => engine.send(Cmd::SetVolume((vol + vol_step).min(1.0))),
        Action::VolumeDown => engine.send(Cmd::SetVolume((vol - vol_step).max(0.0))),
        Action::Mute => engine.send(Cmd::ToggleMute),
        Action::SeekForward => engine.send(Cmd::SeekRel(seek_ms)),
        Action::SeekBackward => engine.send(Cmd::SeekRel(-seek_ms)),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn icon_rgba(size: u32) -> Vec<u8> {
        // generated aqua droplet/circle icon
        let mut v = Vec::with_capacity((size * size * 4) as usize);
        let c = size as f32 / 2.0;
        for y in 0..size {
            for x in 0..size {
                let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt() / c;
                let a = ((1.0 - d) * size as f32 / 2.0).clamp(0.0, 1.0);
                let t = y as f32 / size as f32;
                v.extend_from_slice(&[(34.0 + 25.0 * t) as u8, (211.0 - 80.0 * t) as u8, (238.0 + 8.0 * t) as u8, (a * 255.0) as u8]);
            }
        }
        v
    }

    pub fn app_icon() -> eframe::egui::IconData {
        eframe::egui::IconData { rgba: icon_rgba(64), width: 64, height: 64 }
    }

    pub struct Tray {
        _icon: tray_icon::TrayIcon,
    }

    pub fn create_tray(engine: Engine, ui: mpsc::Sender<UiCmd>, wake: Wake) -> anyhow::Result<Tray> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        let menu = Menu::new();
        let play = MenuItem::with_id("play", "Play / Pause", true, None);
        let next = MenuItem::with_id("next", "Next", true, None);
        let prev = MenuItem::with_id("prev", "Previous", true, None);
        let show = MenuItem::with_id("show", "Show Aqua Glass", true, None);
        let mini = MenuItem::with_id("mini", "Mini-Player", true, None);
        let settings = MenuItem::with_id("settings", "Settings…", true, None);
        let quit = MenuItem::with_id("quit", "Quit", true, None);
        menu.append_items(&[&play, &next, &prev, &PredefinedMenuItem::separator(), &show, &mini, &settings, &PredefinedMenuItem::separator(), &quit])?;
        let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32)?;
        let tray = tray_icon::TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Aqua Glass").with_icon(icon).with_menu_on_left_click(false).build()?;
        let (ui2, wake2) = (ui.clone(), wake.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            match e.id().0.as_str() {
                "play" => engine.send(Cmd::Toggle),
                "next" => engine.send(Cmd::Next),
                "prev" => engine.send(Cmd::Prev),
                "show" => {
                    let _ = ui.send(UiCmd::Show);
                }
                "mini" => {
                    let _ = ui.send(UiCmd::ToggleMini);
                }
                "settings" => {
                    let _ = ui.send(UiCmd::OpenSettings);
                }
                "quit" => {
                    let _ = ui.send(UiCmd::Quit);
                }
                _ => {}
            }
            wake();
        }));
        tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
            if let tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = e {
                let _ = ui2.send(UiCmd::Show);
                wake2();
            }
        }));
        Ok(Tray { _icon: tray })
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            tray_icon::menu::MenuEvent::set_event_handler(None::<fn(tray_icon::menu::MenuEvent)>);
            tray_icon::TrayIconEvent::set_event_handler(None::<fn(tray_icon::TrayIconEvent)>);
        }
    }

    // ---- SMTC ----
    pub struct Media {
        pub controls: Arc<Mutex<Option<souvlaki::MediaControls>>>,
    }

    pub fn create_media(hwnd: isize, engine: Engine) -> anyhow::Result<Media> {
        use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};
        let mut c = MediaControls::new(PlatformConfig { display_name: "Aqua Glass", dbus_name: "aqua_glass", hwnd: Some(hwnd as *mut std::ffi::c_void) })
            .map_err(|e| anyhow::anyhow!("media controls: {e:?}"))?;
        let e2 = engine.clone();
        c.attach(move |ev: MediaControlEvent| match ev {
            MediaControlEvent::Play => e2.send(Cmd::Resume),
            MediaControlEvent::Pause => e2.send(Cmd::Pause),
            MediaControlEvent::Toggle => e2.send(Cmd::Toggle),
            MediaControlEvent::Next => e2.send(Cmd::Next),
            MediaControlEvent::Previous => e2.send(Cmd::Prev),
            MediaControlEvent::Stop => e2.send(Cmd::Pause),
            MediaControlEvent::Seek(SeekDirection::Forward) => e2.send(Cmd::SeekRel(10_000)),
            MediaControlEvent::Seek(SeekDirection::Backward) => e2.send(Cmd::SeekRel(-10_000)),
            MediaControlEvent::SeekBy(SeekDirection::Forward, d) => e2.send(Cmd::SeekRel(d.as_millis() as i64)),
            MediaControlEvent::SeekBy(SeekDirection::Backward, d) => e2.send(Cmd::SeekRel(-(d.as_millis() as i64))),
            MediaControlEvent::SetPosition(p) => e2.send(Cmd::Seek(p.0.as_millis() as u64)),
            MediaControlEvent::SetVolume(v) => e2.send(Cmd::SetVolume(v as f32)),
            _ => {}
        })
        .map_err(|e| anyhow::anyhow!("media controls: {e:?}"))?;
        let controls = Arc::new(Mutex::new(Some(c)));
        let c2 = controls.clone();
        let eng = engine.clone();
        let mut last_key = String::new();
        let mut last_playing = None;
        engine.add_listener(Box::new(move |st: &PlayerState| {
            let mut g = c2.lock().unwrap();
            let Some(c) = g.as_mut() else { return };
            if let Some(t) = &st.current {
                let key = t.key();
                if key != last_key {
                    last_key = key;
                    let artist = t.artist_line();
                    let cover = if t.artwork_url.starts_with("https://") { Some(t.artwork_url.as_str()) } else { None };
                    let _ = c.set_metadata(souvlaki::MediaMetadata {
                        title: Some(&t.title),
                        artist: Some(&artist),
                        album: Some(&t.album),
                        cover_url: cover,
                        duration: Some(std::time::Duration::from_millis(t.duration_ms)),
                    });
                }
            }
            if last_playing != Some(st.playing) {
                last_playing = Some(st.playing);
                let progress = Some(souvlaki::MediaPosition(std::time::Duration::from_millis(eng.position_ms())));
                let _ = c.set_playback(if st.playing { souvlaki::MediaPlayback::Playing { progress } } else { souvlaki::MediaPlayback::Paused { progress } });
            }
        }));
        Ok(Media { controls })
    }

    impl Drop for Media {
        fn drop(&mut self) {
            if let Some(mut c) = self.controls.lock().unwrap().take() {
                let _ = c.detach();
            }
        }
    }

    // ---- Global hotkeys ----
    pub struct Hotkeys {
        manager: global_hotkey::GlobalHotKeyManager,
        registered: Vec<global_hotkey::hotkey::HotKey>,
        pub errors: Vec<String>,
    }

    fn code(k: &str) -> Option<global_hotkey::hotkey::Code> {
        use global_hotkey::hotkey::Code;
        Some(match k {
            "Space" => Code::Space,
            "Enter" => Code::Enter,
            "Escape" => Code::Escape,
            "Tab" => Code::Tab,
            "Backspace" => Code::Backspace,
            "Insert" => Code::Insert,
            "Delete" => Code::Delete,
            "Home" => Code::Home,
            "End" => Code::End,
            "PageUp" => Code::PageUp,
            "PageDown" => Code::PageDown,
            "Left" => Code::ArrowLeft,
            "Right" => Code::ArrowRight,
            "Up" => Code::ArrowUp,
            "Down" => Code::ArrowDown,
            "Comma" => Code::Comma,
            "Period" => Code::Period,
            "Minus" => Code::Minus,
            "Plus" => Code::Equal,
            "Slash" => Code::Slash,
            "Semicolon" => Code::Semicolon,
            "MediaPlayPause" => Code::MediaPlayPause,
            "MediaNext" => Code::MediaTrackNext,
            "MediaPrevious" => Code::MediaTrackPrevious,
            "MediaStop" => Code::MediaStop,
            "VolumeUp" => Code::AudioVolumeUp,
            "VolumeDown" => Code::AudioVolumeDown,
            "VolumeMute" => Code::AudioVolumeMute,
            k if k.len() == 1 => {
                let c = k.chars().next()?;
                if c.is_ascii_digit() {
                    format!("Digit{c}").parse().ok()?
                } else {
                    format!("Key{c}").parse().ok()?
                }
            }
            k if k.starts_with('F') => k.parse().ok()?,
            _ => return None,
        })
    }

    pub fn create_hotkeys(bindings: &[(Action, crate::shortcuts::Binding)], engine: Engine, ui: mpsc::Sender<UiCmd>, wake: Wake, vol_step: f32, seek_ms: i64) -> anyhow::Result<Hotkeys> {
        use global_hotkey::hotkey::{HotKey, Modifiers};
        let manager = global_hotkey::GlobalHotKeyManager::new()?;
        let mut registered = Vec::new();
        let mut errors = Vec::new();
        let mut map = std::collections::HashMap::new();
        for (a, b) in bindings {
            let Some(c) = code(&b.key) else {
                errors.push(format!("{}: key {} cannot be a global hotkey", a.label(), b.key));
                continue;
            };
            let mut m = Modifiers::empty();
            if b.ctrl {
                m |= Modifiers::CONTROL;
            }
            if b.alt {
                m |= Modifiers::ALT;
            }
            if b.shift {
                m |= Modifiers::SHIFT;
            }
            if b.win {
                m |= Modifiers::SUPER;
            }
            let hk = HotKey::new(if m.is_empty() { None } else { Some(m) }, c);
            match manager.register(hk) {
                Ok(()) => {
                    map.insert(hk.id(), *a);
                    registered.push(hk);
                }
                Err(e) => errors.push(format!("{} ({}): {e} — probably used by another app", a.label(), b.to_string_canonical())),
            }
        }
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e: global_hotkey::GlobalHotKeyEvent| {
            if e.state != global_hotkey::HotKeyState::Pressed {
                return;
            }
            if let Some(a) = map.get(&e.id) {
                if !engine_action(&engine, *a, vol_step, seek_ms) {
                    let _ = ui.send(UiCmd::Action(*a));
                    wake();
                }
            }
        }));
        Ok(Hotkeys { manager, registered, errors })
    }

    impl Drop for Hotkeys {
        fn drop(&mut self) {
            let _ = self.manager.unregister_all(&self.registered);
            global_hotkey::GlobalHotKeyEvent::set_event_handler(None::<fn(global_hotkey::GlobalHotKeyEvent)>);
        }
    }

    // ---- autostart ----
    pub fn set_autostart(on: bool) -> anyhow::Result<()> {
        use winreg::enums::*;
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
        if on {
            let exe = std::env::current_exe()?;
            run.set_value("AquaGlass", &format!("\"{}\" --minimized", exe.display()))?;
        } else {
            let _ = run.delete_value("AquaGlass");
        }
        Ok(())
    }

    pub fn autostart_enabled() -> bool {
        use winreg::enums::*;
        winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .and_then(|k| k.get_value::<String, _>("AquaGlass"))
            .is_ok()
    }

    pub fn notify(title: &str, body: &str) {
        let (t, b) = (title.to_string(), body.to_string());
        std::thread::spawn(move || {
            let _ = tauri_winrt_notification::Toast::new(tauri_winrt_notification::Toast::POWERSHELL_APP_ID)
                .title(&t)
                .text1(&b)
                .sound(None)
                .duration(tauri_winrt_notification::Duration::Short)
                .show();
        });
    }

    /// Windows 11 22H2+: system backdrop (acrylic) + dark title bar.
    pub fn set_backdrop(hwnd: isize, acrylic: bool, dark: bool) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::*;
        unsafe {
            let h = HWND(hwnd as *mut _);
            let dark_v: i32 = dark as i32;
            let _ = DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_v as *const _ as _, 4);
            let kind: DWM_SYSTEMBACKDROP_TYPE = if acrylic { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_AUTO };
            let _ = DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as _, 4);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub struct Tray;
    pub struct Media;
    pub struct Hotkeys {
        pub errors: Vec<String>,
    }
    pub fn app_icon() -> eframe::egui::IconData {
        eframe::egui::IconData::default()
    }
    pub fn create_tray(_e: Engine, _u: mpsc::Sender<UiCmd>, _w: Wake) -> anyhow::Result<Tray> {
        anyhow::bail!("system tray is only implemented on Windows")
    }
    pub fn create_media(_h: isize, _e: Engine) -> anyhow::Result<Media> {
        anyhow::bail!("media controls are only implemented on Windows")
    }
    pub fn create_hotkeys(_b: &[(Action, crate::shortcuts::Binding)], _e: Engine, _u: mpsc::Sender<UiCmd>, _w: Wake, _v: f32, _s: i64) -> anyhow::Result<Hotkeys> {
        anyhow::bail!("global hotkeys are only implemented on Windows")
    }
    pub fn set_autostart(_on: bool) -> anyhow::Result<()> {
        anyhow::bail!("start with Windows is only available on Windows")
    }
    pub fn autostart_enabled() -> bool {
        false
    }
    pub fn notify(title: &str, body: &str) {
        log::info!("notification: {title} — {body}");
    }
    pub fn set_backdrop(_h: isize, _a: bool, _d: bool) {}
}

pub use imp::*;
