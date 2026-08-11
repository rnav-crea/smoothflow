mod audio;
pub mod config;
mod history;
mod postprocess;
mod text_injection;
pub mod transcription;

use audio::AudioRecorder;
use config::Config;
use history::History;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

/// Debug-only logging macro. Compiles to a no-op in release builds (VULN-006).
macro_rules! sf_log {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            println!($($arg)*);
        }
    };
}

struct AppState {
    recorder: Mutex<AudioRecorder>,
    config: Mutex<Config>,
    history: Mutex<History>,
    overlay: Mutex<Option<WebviewWindow>>,
}

fn spawn_vu_meter(
    app: &tauri::AppHandle,
    recording: std::sync::Arc<std::sync::Mutex<bool>>,
    peak_level: std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    let app_clone = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(80));
            if !*recording.lock().unwrap() { break; }
            let raw = peak_level.load(std::sync::atomic::Ordering::Relaxed);
            let level = raw as f32 / 100000.0;
            let _ = app_clone.emit("audio-level", level);
        }
        let _ = app_clone.emit("audio-level", 0.0f32);
    });
}

/// Show the floating overlay and broadcast an error so BOTH the main window
/// and the overlay bubble render it. Single source of truth for error display.
fn show_error(app: &tauri::AppHandle, event: &str, msg: String) {
    if let Some(overlay) = app.state::<AppState>().overlay.lock().unwrap().as_ref() {
        let _ = overlay.show();
    }
    let _ = app.emit(event, msg);
}

fn parse_hotkey_str(s: &str) -> Result<(Modifiers, Code), String> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.len() < 2 {
        return Err(format!("[CFG-005] Invalid hotkey '{}'. Use format like 'Ctrl+Space'.", s));
    }
    let mut mods = Modifiers::empty();
    for m in &parts[..parts.len() - 1] {
        match *m {
            "Ctrl" | "Control" => mods |= Modifiers::CONTROL,
            "Alt" | "Option" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            "Win" | "Meta" | "Super" | "Logo" => mods |= Modifiers::SUPER,
            _ => return Err(format!("[CFG-005] Invalid hotkey '{}'. Unknown modifier '{}'.", s, m)),
        }
    }
    let key = parts[parts.len() - 1];
    let code = match key {
        "Space" => Code::Space,
        "Tab" => Code::Tab,
        "Enter" => Code::Enter,
        "Escape" | "Esc" => Code::Escape,
        "Backspace" => Code::Backspace,
        "Delete" => Code::Delete,
        "Insert" => Code::Insert,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        "ArrowUp" | "Up" => Code::ArrowUp,
        "ArrowDown" | "Down" => Code::ArrowDown,
        "ArrowLeft" | "Left" => Code::ArrowLeft,
        "ArrowRight" | "Right" => Code::ArrowRight,
        "Backquote" | "`" => Code::Backquote,
        "Minus" | "-" => Code::Minus,
        "Equal" | "=" => Code::Equal,
        "Semicolon" | ";" => Code::Semicolon,
        "Quote" | "'" => Code::Quote,
        "Comma" | "," => Code::Comma,
        "Period" | "." => Code::Period,
        "Slash" | "/" => Code::Slash,
        "Backslash" | "\\" => Code::Backslash,
        "BracketLeft" | "[" => Code::BracketLeft,
        "BracketRight" | "]" => Code::BracketRight,
        _ if key.len() == 1 => {
            let c = key.chars().next().unwrap().to_ascii_uppercase();
            match c {
                '0'..='9' => [Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4,
                    Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9][c.to_digit(10).unwrap() as usize],
                'A'..='Z' => [Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE, Code::KeyF,
                    Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK, Code::KeyL,
                    Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ, Code::KeyR,
                    Code::KeyS, Code::KeyT, Code::KeyU, Code::KeyV, Code::KeyW, Code::KeyX,
                    Code::KeyY, Code::KeyZ][(c as u8 - b'A') as usize],
                _ => return Err(format!("[CFG-005] Invalid hotkey. Unknown key '{}'.", key)),
            }
        }
        _ if key.starts_with('F') && key[1..].parse::<u8>().is_ok() => {
            match key[1..].parse::<u8>().unwrap() {
                1 => Code::F1, 2 => Code::F2, 3 => Code::F3, 4 => Code::F4,
                5 => Code::F5, 6 => Code::F6, 7 => Code::F7, 8 => Code::F8,
                9 => Code::F9, 10 => Code::F10, 11 => Code::F11, 12 => Code::F12,
                13 => Code::F13, 14 => Code::F14, 15 => Code::F15, 16 => Code::F16,
                17 => Code::F17, 18 => Code::F18, 19 => Code::F19, 20 => Code::F20,
                21 => Code::F21, 22 => Code::F22, 23 => Code::F23, 24 => Code::F24,
                n => return Err(format!("[CFG-005] Invalid hotkey. Unknown F-key F{}.", n)),
            }
        }
        _ => return Err(format!("[CFG-005] Invalid hotkey. Unknown key '{}'.", key)),
    };
    Ok((mods, code))
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    sf_log!("CMD: start_recording called");
    let mut recorder = state.recorder.lock().unwrap();
    if recorder.is_recording() {
        sf_log!("CMD: already recording, ignoring");
        return Ok(());
    }
    let peak_level = recorder.peak_level.clone();
    let recording_flag = recorder.recording.clone();
    recorder.start().map_err(|e| {
        show_error(&app, "recording-error", e.clone());
        e
    })?;
    drop(recorder);
    let _ = app.emit("recording-state", true);

    // ponytail: polling thread for live VU meter
    spawn_vu_meter(&app, recording_flag, peak_level);

    sf_log!("CMD: start_recording done");
    Ok(())
}

#[tauri::command]
fn stop_recording(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<String, String> {
    sf_log!("CMD: stop_recording called");
    let mut recorder = state.recorder.lock().unwrap();
    let samples = recorder.stop();
    let sample_rate = recorder.sample_rate();
    drop(recorder);
    sf_log!("CMD: got {} samples at {} Hz", samples.len(), sample_rate);
    let _ = app.emit("recording-state", false);
    
    let config = state.config.lock().unwrap();
    sf_log!("CMD: transcribing...");
    let raw = match transcription::transcribe(&samples, sample_rate, &config) {
        Ok(r) => r,
        Err(e) => {
            show_error(&app, "transcription-error", e.clone());
            return Err(e);
        }
    };
    sf_log!("CMD: transcript received: {:?}", &raw[..raw.len().min(50)]);
    let _ = app.emit("raw-transcript", raw.clone());
    let text = postprocess::postprocess(&raw, &config);

    // Persist non-empty dictations to history (independent of auto-paste)
    {
        let mut h = state.history.lock().unwrap();
        if !text.is_empty() {
            h.push(&text);
            h.save();
        }
    }

    let _ = app.emit("transcript-result", text.clone());
    
    if !text.is_empty() {
        if config.auto_paste {
            sf_log!("CMD: typing text...");
            if let Err(e) = text_injection::type_text(&text) {
                show_error(&app, "transcription-error", e.clone());
                return Err(e);
            }
            sf_log!("CMD: typed OK");
        } else {
            sf_log!("CMD: auto-paste disabled, showing in transcript only");
        }
    }
    Ok(text)
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<Config, ()> {
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
fn update_config(app: tauri::AppHandle, state: tauri::State<AppState>, new_config: Config) -> Result<(), String> {
    if new_config.api_key.is_empty() {
        return Err("[CFG-002] API key cannot be empty. Enter a valid key in Settings.".into());
    }
    if new_config.model.is_empty() {
        return Err("[CFG-003] Model name cannot be empty. Enter a model (e.g. whisper-large-v3).".into());
    }
    if !new_config.api_base_url.starts_with("https://") {
        return Err("[CFG-004] API base URL must use HTTPS (e.g. https://api.groq.com/openai/v1).".into());
    }
    // VULN-003: the key goes into the OS vault before anything is saved;
    // Config::save() strips api_key from the JSON written to disk.
    if !new_config.api_key.is_empty() {
        config::store_secret(&new_config.api_key)?;
    }
    let new_hotkey = new_config.hotkey.clone();
    let launch_on_startup = new_config.launch_on_startup;
    let old_hotkey = {
        let mut config = state.config.lock().unwrap();
        let old = config.hotkey.clone();
        *config = new_config;
        config.save();
        old
    };

    if old_hotkey != new_hotkey {
        if let Ok((om, oc)) = parse_hotkey_str(&old_hotkey) {
            let _ = app.global_shortcut().unregister(Shortcut::new(Some(om), oc));
        }
        match parse_hotkey_str(&new_hotkey) {
            Ok((nm, nc)) => {
                app.global_shortcut().register(Shortcut::new(Some(nm), nc))
                    .map_err(|e| format!("Failed to register hotkey '{}': {}", new_hotkey, e))?;
                sf_log!("HOTKEY: re-registered to {}", new_hotkey);
            }
            Err(e) => return Err(e),
        }
    }

    if launch_on_startup {
        let _ = app.autolaunch().enable();
    } else {
        let _ = app.autolaunch().disable();
    }

    Ok(())
}

#[tauri::command]
fn get_history(state: tauri::State<AppState>) -> Result<History, ()> {
    Ok(state.history.lock().unwrap().clone())
}

#[tauri::command]
fn delete_history_entry(state: tauri::State<AppState>, index: usize) -> Result<(), String> {
    let mut history = state.history.lock().unwrap();
    if index >= history.entries.len() {
        return Err(format!("History entry index {index} out of bounds"));
    }
    history.entries.remove(index);
    history.save();
    Ok(())
}

#[tauri::command]
fn clear_history(state: tauri::State<AppState>) -> Result<(), String> {
    let mut history = state.history.lock().unwrap();
    history.entries.clear();
    history.save();
    Ok(())
}

#[tauri::command]
fn inject_text(text: String) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    text_injection::type_text(&text)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    let state = app.state::<AppState>();
                    let cfg = state.config.lock().unwrap();
                    if let Ok((m, c)) = parse_hotkey_str(&cfg.hotkey) {
                        if !shortcut.matches(m, c) { return; }
                    } else { return; }
                    drop(cfg);
                    sf_log!("HOTKEY: event state={:?}", event.state);
                    match event.state {
                        ShortcutState::Pressed => {
                            let started = {
                                let mut recorder = state.recorder.lock().unwrap();
                                if recorder.is_recording() {
                                    false
                                } else {
                                    let peak_level = recorder.peak_level.clone();
                                    let recording_flag = recorder.recording.clone();
                                    match recorder.start() {
                                        Ok(_) => {
                                            drop(recorder);
                                            let _ = app.emit("recording-state", true);
                                            spawn_vu_meter(app, recording_flag, peak_level);
                                            true
                                        }
                                        Err(e) => {
                                            drop(recorder);
                                            sf_log!("HOTKEY: start_recording error: {e}");
                                            show_error(app, "recording-error", e);
                                            false
                                        }
                                    }
                                }
                            };
                            if started {
                                if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
                                    match overlay.show() {
                                        Ok(_) => sf_log!("HOTKEY: overlay shown"),
                                        Err(e) => sf_log!("HOTKEY: overlay show error: {e}"),
                                    }
                                } else {
                                    sf_log!("HOTKEY: overlay not available in state");
                                }
                            }
                        }
                        ShortcutState::Released => {
                            let mut recorder = state.recorder.lock().unwrap();
                            if !recorder.is_recording() {
                                return;
                            }
                            let samples = recorder.stop();
                            let sample_rate = recorder.sample_rate();
                            drop(recorder);
                            let _ = app.emit("recording-state", false);

                            if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
                                match overlay.hide() {
                                    Ok(_) => sf_log!("HOTKEY: overlay hidden"),
                                    Err(e) => sf_log!("HOTKEY: overlay hide error: {e}"),
                                }
                            }

                            let config = state.config.lock().unwrap().clone();
                            let app_clone = app.clone();

                            // ponytail: spawn to unblock shortcut thread during HTTP calls
                            std::thread::spawn(move || {
                                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let raw = transcription::transcribe(&samples, sample_rate, &config)?;
                                    let _ = app_clone.emit("raw-transcript", raw.clone());
                                    let text = postprocess::postprocess(&raw, &config);
                                    let st = app_clone.state::<AppState>();
                                    let mut h = st.history.lock().unwrap();
                                    if !text.is_empty() {
                                        h.push(&text);
                                        h.save();
                                    }
                                    drop(h);
                                    let _ = app_clone.emit("transcript-result", text.clone());
                                    if config.auto_paste {
                                        if !text.is_empty() {
                                            sf_log!("HOTKEY: auto-paste enabled, typing...");
                                            if let Err(e) = text_injection::type_text(&text) {
                                                sf_log!("HOTKEY: type_text error: {e}");
                                                show_error(&app_clone, "transcription-error", e);
                                            }
                                        } else {
                                            sf_log!("HOTKEY: auto-paste enabled but text is empty, skipping");
                                        }
                                    } else {
                                        sf_log!("HOTKEY: auto-paste disabled in config");
                                    }
                                    Ok::<_, String>(())
                                }));
                                match result {
                                    Err(panic) => {
                                        let msg = match panic.downcast::<String>() {
                                            Ok(s) => s.to_string(),
                                            Err(_) => "unknown panic in transcription thread".into(),
                                        };
                                        sf_log!("TRANSCRIPTION PANIC: {msg}");
                                        show_error(&app_clone, "transcription-error", msg);
                                    }
                                    Ok(Err(e)) => {
                                        sf_log!("TRANSCRIPTION ERROR: {e}");
                                        show_error(&app_clone, "transcription-error", e);
                                    }
                                    Ok(Ok(())) => {}
                                }
                            });
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(AppState {
            recorder: Mutex::new(AudioRecorder::new()),
            config: Mutex::new(Config::load()),
            history: Mutex::new(History::load()),
            overlay: Mutex::new(None),
        })
        .setup(|app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            // Ensure main window is visible (Tauri 2 + decorations:false quirk on Windows)
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.set_focus();
                sf_log!("SETUP: main window shown");
            }

            let state = app.state::<AppState>();
            let hotkey_str = state.config.lock().unwrap().hotkey.clone();
            drop(state);
            match parse_hotkey_str(&hotkey_str) {
                Ok((m, c)) => {
                    match app.global_shortcut().register(Shortcut::new(Some(m), c)) {
                        Ok(_) => sf_log!("SUCCESS: Registered {} global shortcut!", hotkey_str),
                        Err(e) => sf_log!("ERROR: Failed to register {}: {}", hotkey_str, e),
                    }
                }
                Err(e) => sf_log!("ERROR: Invalid hotkey '{}': {}", hotkey_str, e),
            }

            let overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("overlay.html".into()),
            )
            .title("")
            .inner_size(800.0, 60.0)
            .always_on_top(true)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .skip_taskbar(true);

            let overlay = if let Some(monitor) = app.primary_monitor().ok().flatten() {
                let scale_factor = monitor.scale_factor();
                let logical_width = monitor.size().width as f64 / scale_factor;
                let x = (logical_width - 800.0) / 2.0;
                let y = 10.0;
                overlay.position(x, y).build()
            } else {
                overlay.build()
            };

            let state = app.state::<AppState>();
            match &overlay {
                Ok(w) => { let _ = w.hide(); let _ = w.set_ignore_cursor_events(true); sf_log!("SETUP: overlay created"); }
                Err(e) => sf_log!("SETUP: overlay creation error: {e}"),
            }
            *state.overlay.lock().unwrap() = overlay.ok();

            sf_log!("SETUP: app ready — tray, overlay, hotkey all configured");
            
            // Sync autostart with config
            {
                let config = state.config.lock().unwrap();
                if config.launch_on_startup {
                    let _ = app.autolaunch().enable();
                } else {
                    let _ = app.autolaunch().disable();
                }
            }

            // System tray
            let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
                .expect("failed to load tray icon");

            let settings_item = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&settings_item)
                .separator()
                .item(&quit_item)
                .build()?;

            TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "settings" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            if let Some(state) = app.try_state::<AppState>() {
                                let mut recorder = state.recorder.lock().unwrap();
                                if recorder.is_recording() {
                                    recorder.stop();
                                }
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let app = window.app_handle();
                if let Some(state) = app.try_state::<AppState>() {
                    let mut recorder = state.recorder.lock().unwrap();
                    if recorder.is_recording() {
                        recorder.stop();
                    }
                }
                app.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            get_config,
            update_config,
            get_history,
            delete_history_entry,
            clear_history,
            inject_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

