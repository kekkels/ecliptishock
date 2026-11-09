#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ecliptishock_core::config::Config;
use ecliptishock_core::engine::{self, Control, EngineHandle, StartOpts};
use ecliptishock_core::report::{Reporter, Status, UiEvent};
use ecliptishock_core::tail::find_log_dir;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    cfg: Mutex<Config>,
    cfg_path: PathBuf,
    load_error: Option<String>,
    engine: EngineHandle,
    status: Arc<Mutex<Option<Status>>>,
    tray_ok: AtomicBool,
}

#[derive(Serialize)]
struct BootInfo {
    config_path: String,
    load_error: Option<String>,
}

#[tauri::command]
fn boot_info(state: State<AppState>) -> BootInfo {
    BootInfo { config_path: state.cfg_path.display().to_string(), load_error: state.load_error.clone() }
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Option<Status> {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(cfg: Config, state: State<AppState>) -> Result<(), String> {
    let cfg = cfg.finalize().map_err(|e| format!("{e:#}"))?;
    cfg.save(&state.cfg_path).map_err(|e| format!("{e:#}"))?;
    *state.cfg.lock().unwrap() = cfg.clone();
    state.engine.send(Control::SetConfig(Box::new(cfg)));
    Ok(())
}

/// Header toggles. `shared_fate` and `dry_run` are persisted; `paused` is runtime only.
#[tauri::command]
fn set_flag(name: String, on: bool, state: State<AppState>) -> Result<(), String> {
    match name.as_str() {
        "paused" => state.engine.send(Control::SetPaused(on)),
        "shared_fate" | "dry_run" => {
            let mut cfg = state.cfg.lock().unwrap();
            if name == "shared_fate" {
                cfg.group.shared_fate = on;
            } else {
                cfg.dry_run = on;
            }
            cfg.save(&state.cfg_path).map_err(|e| format!("{e:#}"))?;
            if name == "shared_fate" {
                state.engine.send(Control::SetShared(on));
            } else {
                state.engine.send(Control::SetConfig(Box::new(cfg.clone())));
            }
        }
        other => return Err(format!("unknown flag '{other}'")),
    }
    Ok(())
}

#[tauri::command]
fn test_shock(tier: String, state: State<AppState>) {
    state.engine.send(Control::Test(tier));
}

#[tauri::command]
fn detect_log_dir() -> Option<String> {
    find_log_dir().map(|p| p.display().to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Best effort: some Linux desktops have no tray. Returns whether one was created.
fn build_tray(app: &tauri::App) -> tauri::Result<bool> {
    let Some(icon) = app.default_window_icon().cloned() else { return Ok(false) };
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause / resume", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &pause, &quit])?;
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Ecliptishock")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "pause" => {
                if let Some(s) = app.try_state::<AppState>() {
                    s.engine.send(Control::TogglePaused);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(true)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let cfg_path = match std::env::var_os("ECLIPTISHOCK_CONFIG") {
                Some(p) => PathBuf::from(p),
                None => app.path().app_config_dir()?.join("config.toml"),
            };
            let (cfg, load_error) = if cfg_path.exists() {
                match Config::load(&cfg_path) {
                    Ok(c) => (c, None),
                    // Keep the broken file untouched until the user saves; run on defaults meanwhile.
                    Err(e) => (Config::default(), Some(format!("{e:#}"))),
                }
            } else {
                let c = Config::default();
                let _ = c.save(&cfg_path);
                (c, None)
            };

            let status: Arc<Mutex<Option<Status>>> = Arc::new(Mutex::new(None));
            let store = status.clone();
            let handle = app.handle().clone();
            let report: Reporter = Arc::new(move |ev: UiEvent| {
                if let UiEvent::Status(s) = &ev {
                    *store.lock().unwrap() = Some(s.clone());
                }
                let _ = handle.emit("engine", &ev);
            });
            let (engine_handle, rx) = engine::channel();
            tauri::async_runtime::spawn(engine::run(
                StartOpts { cfg: cfg.clone(), log_dir: None, shared_override: None, force_dry_run: false },
                report,
                rx,
            ));

            app.manage(AppState {
                cfg: Mutex::new(cfg),
                cfg_path,
                load_error,
                engine: engine_handle,
                status,
                tray_ok: AtomicBool::new(false),
            });

            match build_tray(app) {
                Ok(true) => app.state::<AppState>().tray_ok.store(true, Ordering::Relaxed),
                Ok(false) => {}
                Err(e) => eprintln!("tray unavailable: {e}"),
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides to the tray (so shocks keep working) - but only if a tray exists.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if let Some(s) = window.app_handle().try_state::<AppState>() {
                    if s.tray_ok.load(Ordering::Relaxed) {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            boot_info,
            get_config,
            get_status,
            save_config,
            set_flag,
            test_shock,
            detect_log_dir,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Ecliptishock app");
}
