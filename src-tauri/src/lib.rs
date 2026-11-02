//! ViuMic desktop app: one window and a tray icon over the engine. The engine keeps running
//! while the window is hidden; closing the window hides it, and Quit (tray) exits.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};
use viumic_engine::{self as engine, Config, Engine, Model, UNLIMITED_DB};

/// How often meters go to the window (and device errors are checked).
const TICK: Duration = Duration::from_millis(33);
/// Wait between attempts to (re)start the engine after a device error.
const RETRY: Duration = Duration::from_secs(2);
/// Device names containing this belong to VB-CABLE.
const CABLE: &str = "VB-Audio";
/// Passed by the "Start with Windows" entry: start in the tray, window hidden.
const HIDDEN_ARG: &str = "--hidden";

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Quality {
    Best,
    Light,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    /// Microphone name; `None` = the Windows default (skipping VB-CABLE's own output).
    input: Option<String>,
    enabled: bool,
    /// Maximum noise attenuation in dB; at or above [`UNLIMITED_DB`] it removes all it can.
    strength_db: f32,
    /// Deeper voice, in semitones (0 = off).
    depth: f32,
    quality: Quality,
    /// Mains hum to remove (50 or 60 Hz), `None` = off.
    hum_hz: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input: None,
            enabled: true,
            strength_db: UNLIMITED_DB,
            depth: 0.0,
            quality: Quality::Best,
            hum_hz: Some(50.0),
        }
    }
}

impl Settings {
    /// Device, model and filters need a new engine; the rest applies live.
    fn needs_restart(&self, old: &Settings) -> bool {
        self.input != old.input || self.quality != old.quality || self.hum_hz != old.hum_hz
    }
}

/// Everything the window shows, except the live meters.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    settings: Settings,
    mics: Vec<String>,
    /// The mic in use right now.
    mic: Option<String>,
    error: Option<String>,
    cable_missing: bool,
    /// Windows plays all sound into the cable, so the user hears nothing.
    cable_is_speaker: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Meters {
    in_peak: f32,
    out_peak: f32,
    latency_ms: f32,
    load: f32,
}

struct App {
    settings: Settings,
    engine: Option<Engine>,
    error: Option<String>,
    last_start: Option<Instant>,
    meters: Option<Channel<Meters>>,
    settings_path: PathBuf,
}

impl App {
    fn view(&self) -> View {
        let d = engine::list_devices().unwrap_or_default();
        View {
            settings: self.settings.clone(),
            cable_missing: !d.has_cable(),
            cable_is_speaker: d.default_output.is_some_and(|n| n.contains(CABLE)),
            mics: d
                .inputs
                .into_iter()
                .filter(|n| !n.contains(CABLE))
                .collect(),
            mic: self.engine.as_ref().map(|e| e.input_name.clone()),
            error: self.error.clone(),
        }
    }

    fn start(&mut self) {
        self.engine = None; // release the devices before reopening them
        self.last_start = Some(Instant::now());
        let s = &self.settings;
        let mut input = s.input.clone();
        if input.is_none() {
            // Installing VB-CABLE makes its output the default mic, which would feed ViuMic its
            // own output. Fall back to the first real mic.
            let d = engine::list_devices().unwrap_or_default();
            if d.default_input
                .as_deref()
                .is_some_and(|n| n.contains(CABLE))
            {
                input = d.inputs.into_iter().find(|n| !n.contains(CABLE));
            }
        }
        let config = Config {
            input,
            output: None,
            model: match s.quality {
                Quality::Best => Model::DpdfNet,
                Quality::Light => Model::RNNoise,
            },
            strength_db: s.strength_db,
            enabled: s.enabled,
            depth: s.depth,
            hum_hz: s.hum_hz,
        };
        match Engine::start(&config) {
            Ok(e) => {
                self.engine = Some(e);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn apply(&mut self, new: Settings) {
        let restart = self.engine.is_none() || new.needs_restart(&self.settings);
        self.settings = new;
        if restart {
            self.start();
        } else if let Some(e) = &self.engine {
            e.set_enabled(self.settings.enabled);
            e.set_strength(self.settings.strength_db);
            e.set_depth(self.settings.depth);
        }
        // ponytail: a failed save is silent (settings stay applied for this run); surface it
        // if users report settings not sticking.
        if let Some(dir) = self.settings_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&self.settings) {
            let _ = std::fs::write(&self.settings_path, json);
        }
    }
}

fn lock<'a>(app: &'a State<'_, Mutex<App>>) -> MutexGuard<'a, App> {
    app.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
fn get_state(app: State<'_, Mutex<App>>) -> View {
    lock(&app).view()
}

#[tauri::command]
fn set_settings(handle: AppHandle, app: State<'_, Mutex<App>>, settings: Settings) -> View {
    let view = {
        let mut a = lock(&app);
        a.apply(settings);
        a.view()
    };
    sync_tray(&handle, view.settings.enabled);
    view
}

#[tauri::command]
fn watch_meters(app: State<'_, Mutex<App>>, channel: Channel<Meters>) {
    lock(&app).meters = Some(channel);
}

/// Background loop: starts the engine, restarts it after device errors, and feeds the meters.
fn supervise(handle: AppHandle) {
    loop {
        std::thread::sleep(TICK);
        let state = handle.state::<Mutex<App>>();
        let mut a = lock(&state);
        let mut changed = false;
        if let Some(err) = a.engine.as_ref().and_then(Engine::take_error) {
            // A lost device kills the streams: restart. Model hiccups don't; just show them.
            if err.starts_with("audio device") {
                a.engine = None;
            }
            a.error = Some(err);
            changed = true;
        }
        if a.engine.is_none() && a.last_start.is_none_or(|t| t.elapsed() >= RETRY) {
            a.start();
            changed = true;
        }
        if let (Some(e), Some(channel)) = (&a.engine, &a.meters) {
            let s = e.stats();
            let meters = Meters {
                in_peak: s.in_peak,
                out_peak: s.out_peak,
                latency_ms: s.latency_ms,
                load: s.load,
            };
            if channel.send(meters).is_err() {
                a.meters = None; // window reloaded or closed; it subscribes again
            }
        }
        if changed {
            let view = a.view();
            drop(a);
            let _ = handle.emit("state", view);
        }
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn sync_tray(app: &AppHandle, enabled: bool) {
    if let Some(item) = app.try_state::<CheckMenuItem<Wry>>() {
        let _ = item.set_checked(enabled);
    }
}

fn toggle_from_tray(app: &AppHandle) {
    let state = app.state::<Mutex<App>>();
    let view = {
        let mut a = lock(&state);
        let mut s = a.settings.clone();
        s.enabled = !s.enabled;
        a.apply(s);
        a.view()
    };
    sync_tray(app, view.settings.enabled);
    let _ = app.emit("state", view);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch just brings the running window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_settings,
            watch_meters
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings: Settings = std::fs::read(&settings_path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();

            let toggle = CheckMenuItem::with_id(
                app,
                "toggle",
                "Noise removal",
                true,
                settings.enabled,
                None::<&str>,
            )?;
            let open = MenuItem::with_id(app, "open", "Open ViuMic", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit ViuMic", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&toggle, &open, &PredefinedMenuItem::separator(app)?, &quit],
            )?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("ViuMic")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_from_tray(app),
                    "open" => show(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            app.manage(toggle);

            app.manage(Mutex::new(App {
                settings,
                engine: None,
                error: None,
                last_start: None,
                meters: None,
                settings_path,
            }));
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("viumic-supervisor".into())
                .spawn(move || supervise(handle))?;

            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ViuMic");
}
