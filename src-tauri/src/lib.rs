//! ViuMic desktop app: one window and a tray icon over the engine. The engine keeps running
//! while the window is hidden; closing the window hides it, and Quit (tray) exits.

#[cfg(windows)]
mod devices;

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
/// Check the Windows mute state every this many ticks (~200 ms): mute keys feel instant.
const MUTE_EVERY: u32 = 6;
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
    /// The chosen microphone. Picked once on the first run (never VB-CABLE's own output),
    /// then always this one: if it's unplugged, ViuMic waits for it instead of switching.
    input: Option<String>,
    enabled: bool,
    /// Maximum noise attenuation in dB; at or above [`UNLIMITED_DB`] it removes all it can.
    strength_db: f32,
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
    /// Connected microphones, without VB-CABLE's own output.
    mics: Vec<String>,
    error: Option<String>,
    cable_missing: bool,
    /// The cable is a default speaker (for apps, media or calls): that sound vanishes into it.
    feed_is_speaker: bool,
    /// "ViuMic Microphone" is Windows' default mic, so mute keys and "Default" apps use it.
    mic_is_default: bool,
    /// What apps call the clean mic ("ViuMic Microphone" once renamed).
    app_mic: Option<String>,
    /// The user muted their mic or ViuMic's in Windows (mute key, taskbar); ViuMic sends silence.
    muted: bool,
    /// ViuMic's playback into the cable is hidden from screen sharing (exclusive mode).
    private_output: bool,
    /// Why it isn't, when Windows refused.
    private_note: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Meters {
    in_peak: f32,
    out_peak: f32,
    latency_ms: f32,
    /// ViuMic's share of the whole processor, in percent, as Task Manager counts it.
    cpu_percent: f32,
}

/// Process CPU usage the way Task Manager reports it: CPU time used since the last sample,
/// divided by the wall time and the number of logical processors.
struct CpuMeter {
    last_cpu: Duration,
    last_wall: Instant,
    percent: f32,
}

impl CpuMeter {
    /// Re-sample at most this often: Windows counts CPU time in ~15.6 ms ticks.
    const EVERY: Duration = Duration::from_millis(500);

    fn new() -> Self {
        Self {
            last_cpu: process_cpu_time(),
            last_wall: Instant::now(),
            percent: 0.0,
        }
    }

    fn percent(&mut self) -> f32 {
        let wall = self.last_wall.elapsed();
        if wall >= Self::EVERY {
            let cpu = process_cpu_time();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
            let used = cpu.saturating_sub(self.last_cpu).as_secs_f32();
            self.percent = 100.0 * used / (wall.as_secs_f32() * cores);
            self.last_cpu = cpu;
            self.last_wall = Instant::now();
        }
        self.percent
    }
}

/// Kernel + user time this process has used so far.
#[cfg(windows)]
fn process_cpu_time() -> Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    let ticks = |t: FILETIME| (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64;
    // SAFETY: plain out-parameters for the current process's pseudo-handle.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    match ok {
        Ok(()) => Duration::from_nanos((ticks(kernel) + ticks(user)) * 100),
        Err(_) => Duration::ZERO,
    }
}

#[cfg(not(windows))]
fn process_cpu_time() -> Duration {
    Duration::ZERO
}

struct App {
    settings: Settings,
    engine: Option<Engine>,
    error: Option<String>,
    last_start: Option<Instant>,
    meters: Option<Channel<Meters>>,
    settings_path: PathBuf,
    muted: bool,
}

impl App {
    fn view(&self) -> View {
        let d = engine::list_devices().unwrap_or_default();
        View {
            settings: self.settings.clone(),
            cable_missing: !d.has_cable(),
            feed_is_speaker: speaker_is_cable(&d),
            mic_is_default: mic_is_default(),
            muted: self.muted,
            private_output: self.engine.as_ref().is_some_and(|e| e.private_output),
            private_note: self.engine.as_ref().and_then(|e| e.private_note.clone()),
            app_mic: d
                .inputs
                .iter()
                .find(|n| n.contains(CABLE))
                .map(|n| n.split(" (").next().unwrap_or(n).to_string()),
            mics: d
                .inputs
                .into_iter()
                .filter(|n| !n.contains(CABLE))
                .collect(),
            error: self.error.clone(),
        }
    }

    fn start(&mut self) {
        self.engine = None; // release the devices before reopening them
        self.last_start = Some(Instant::now());
        // Best effort: an unrenamed cable still works, it's just called "CABLE Output".
        #[cfg(windows)]
        let _ = devices::brand();
        let devices = engine::list_devices().unwrap_or_default();
        let Some(mic) = self.settings.input.clone().or_else(|| first_pick(&devices)) else {
            self.error = Some("No microphone found. Plug one in".into());
            return;
        };
        if self.settings.input.is_none() {
            self.settings.input = Some(mic.clone());
            self.save();
        }
        if !devices.inputs.contains(&mic) {
            self.error = Some(format!(
                "“{mic}” isn't connected. ViuMic switches back to it as soon as it's plugged in"
            ));
            return;
        }
        let s = &self.settings;
        let config = Config {
            input: Some(mic),
            output: None,
            model: match s.quality {
                Quality::Best => Model::DpdfNet,
                Quality::Light => Model::RNNoise,
            },
            strength_db: s.strength_db,
            enabled: s.enabled,
            hum_hz: s.hum_hz,
        };
        match Engine::start(&config) {
            Ok(e) => {
                e.set_muted(self.muted);
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
        }
        self.save();
    }

    // ponytail: a failed save is silent (settings stay applied for this run); surface it if
    // users report settings not sticking.
    fn save(&self) {
        if let Some(dir) = self.settings_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&self.settings) {
            let _ = std::fs::write(&self.settings_path, json);
        }
    }
}

/// First-run pick: the Windows default mic, unless that's VB-CABLE's own output (installing
/// the cable makes it the default), else the first real mic.
fn first_pick(d: &engine::Devices) -> Option<String> {
    d.default_input
        .clone()
        .filter(|n| !n.contains(CABLE))
        .or_else(|| d.inputs.iter().find(|n| !n.contains(CABLE)).cloned())
}

/// Whether the user muted their chosen mic or ViuMic's in Windows: the laptop mute key, the
/// taskbar, or Sound settings. ViuMic then sends silence: muting must also mute screen shares.
fn user_muted(mic: Option<&str>) -> bool {
    #[cfg(windows)]
    return devices::any_muted(|n| Some(n) == mic || n.contains(CABLE));
    #[cfg(not(windows))]
    {
        let _ = mic;
        false
    }
}

fn mic_is_default() -> bool {
    #[cfg(windows)]
    return devices::defaults().mic_is_viumic;
    #[cfg(not(windows))]
    true
}

/// The cable as a default speaker for any role (apps, media, calls).
fn speaker_is_cable(d: &engine::Devices) -> bool {
    #[cfg(windows)]
    {
        let _ = d;
        devices::defaults().feed_is_speaker
    }
    #[cfg(not(windows))]
    d.default_output.as_ref().is_some_and(|n| n.contains(CABLE))
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

/// One click: "ViuMic Microphone" becomes Windows' default mic for every role.
#[tauri::command]
fn make_default_mic(app: State<'_, Mutex<App>>) -> Result<View, String> {
    #[cfg(windows)]
    devices::make_default_mic().map_err(|e| e.to_string())?;
    Ok(lock(&app).view())
}

/// One click: no speaker role points into the cable any more.
#[tauri::command]
fn fix_speaker(app: State<'_, Mutex<App>>) -> Result<View, String> {
    #[cfg(windows)]
    devices::fix_speaker().map_err(|e| e.to_string())?;
    Ok(lock(&app).view())
}

#[tauri::command]
fn watch_meters(app: State<'_, Mutex<App>>, channel: Channel<Meters>) {
    lock(&app).meters = Some(channel);
}

/// Background loop: starts the engine, restarts it after device errors, and feeds the meters.
fn supervise(handle: AppHandle) {
    let mut cpu = CpuMeter::new();
    let mut tick = 0u32;
    loop {
        std::thread::sleep(TICK);
        tick = tick.wrapping_add(1);
        let state = handle.state::<Mutex<App>>();
        let mut a = lock(&state);
        let mut changed = false;
        if tick.is_multiple_of(MUTE_EVERY) {
            let muted = user_muted(a.settings.input.as_deref());
            if muted != a.muted {
                a.muted = muted;
                if let Some(e) = &a.engine {
                    e.set_muted(muted);
                }
                changed = true;
            }
        }
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
                cpu_percent: cpu.percent(),
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
            watch_meters,
            make_default_mic,
            fix_speaker
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
                muted: false,
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
