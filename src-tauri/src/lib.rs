mod audio;
pub mod config;
mod postprocess;
mod text_injection;
pub mod transcription;

use audio::AudioRecorder;
use config::Config;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

struct AppState {
    recorder: Mutex<AudioRecorder>,
    config: Mutex<Config>,
    overlay: Mutex<Option<WebviewWindow>>,
}

fn parse_hotkey_str(s: &str) -> Result<(Modifiers, Code), String> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.len() < 2 {
        return Err(format!("Invalid hotkey '{}'. Use 'Ctrl+Space'", s));
    }
    let mut mods = Modifiers::empty();
    for m in &parts[..parts.len() - 1] {
        match *m {
            "Ctrl" | "Control" => mods |= Modifiers::CONTROL,
            "Alt" | "Option" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            "Win" | "Meta" | "Super" | "Logo" => mods |= Modifiers::SUPER,
            _ => return Err(format!("Unknown modifier: {}", m)),
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
                _ => return Err(format!("Unknown key: {}", key)),
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
                n => return Err(format!("Unknown F-key: F{}", n)),
            }
        }
        _ => return Err(format!("Unknown key: {}", key)),
    };
    Ok((mods, code))
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    println!("CMD: start_recording called");
    let mut recorder = state.recorder.lock().unwrap();
    if recorder.is_recording() {
        println!("CMD: already recording, ignoring");
        return Ok(());
    }
    let peak_level = recorder.peak_level.clone();
    let recording_flag = recorder.recording.clone();
    recorder.start()?;
    drop(recorder);
    let _ = app.emit("recording-state", true);

    // ponytail: polling thread for live VU meter
    let app_clone = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(80));
            if !*recording_flag.lock().unwrap() { break; }
            let raw = peak_level.load(std::sync::atomic::Ordering::Relaxed);
            let level = raw as f32 / 100000.0;
            let _ = app_clone.emit("audio-level", level);
        }
        let _ = app_clone.emit("audio-level", 0.0f32);
    });

    println!("CMD: start_recording done");
    Ok(())
}

#[tauri::command]
fn stop_recording(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<String, String> {
    println!("CMD: stop_recording called");
    let mut recorder = state.recorder.lock().unwrap();
    let samples = recorder.stop();
    let sample_rate = recorder.sample_rate();
    drop(recorder);
    println!("CMD: got {} samples at {} Hz", samples.len(), sample_rate);
    let _ = app.emit("recording-state", false);
    
    let config = state.config.lock().unwrap();
    println!("CMD: transcribing...");
    let raw = transcription::transcribe(&samples, sample_rate, &config)?;
    println!("CMD: transcript received: {:?}", &raw[..raw.len().min(50)]);
    let _ = app.emit("raw-transcript", raw.clone());
    let text = postprocess::postprocess(&raw, &config);
    
    let _ = app.emit("transcript-result", text.clone());
    
    if !text.is_empty() {
        if config.auto_paste {
            println!("CMD: typing text...");
            text_injection::type_text(&text)?;
            println!("CMD: typed OK");
        } else {
            println!("CMD: auto-paste disabled, showing in transcript only");
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
        return Err("API key cannot be empty".into());
    }
    if new_config.model.is_empty() {
        return Err("Model name cannot be empty".into());
    }
    let old_hotkey = state.config.lock().unwrap().hotkey.clone();
    let new_hotkey = new_config.hotkey.clone();
    let launch_on_startup = new_config.launch_on_startup;
    *state.config.lock().unwrap() = new_config;
    state.config.lock().unwrap().save();

    if old_hotkey != new_hotkey {
        if let Ok((om, oc)) = parse_hotkey_str(&old_hotkey) {
            let _ = app.global_shortcut().unregister(Shortcut::new(Some(om), oc));
        }
        match parse_hotkey_str(&new_hotkey) {
            Ok((nm, nc)) => {
                app.global_shortcut().register(Shortcut::new(Some(nm), nc))
                    .map_err(|e| format!("Failed to register hotkey '{}': {}", new_hotkey, e))?;
                println!("HOTKEY: re-registered to {}", new_hotkey);
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
                    println!("HOTKEY: event state={:?}", event.state);
                    match event.state {
                        ShortcutState::Pressed => {
                            let started = {
                                let mut recorder = state.recorder.lock().unwrap();
                                if recorder.is_recording() {
                                    false
                                } else {
                                    match recorder.start() {
                                        Ok(_) => true,
                                        Err(e) => {
                                            println!("start_recording error: {e}");
                                            false
                                        }
                                    }
                                }
                            };
                            if started {
                                let _ = app.emit("recording-state", true);
                                if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
                                    match overlay.show() {
                                        Ok(_) => println!("HOTKEY: overlay shown"),
                                        Err(e) => println!("HOTKEY: overlay show error: {e}"),
                                    }
                                } else {
                                    println!("HOTKEY: overlay not available in state");
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
                                    Ok(_) => println!("HOTKEY: overlay hidden"),
                                    Err(e) => println!("HOTKEY: overlay hide error: {e}"),
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
                                    let _ = app_clone.emit("transcript-result", text.clone());
                                    if config.auto_paste {
                                        if !text.is_empty() {
                                            println!("HOTKEY: auto-paste enabled, typing...");
                                            if let Err(e) = text_injection::type_text(&text) {
                                                println!("HOTKEY: type_text error: {e}");
                                                let _ = app_clone.emit("transcription-error", format!("Auto-paste failed: {e}"));
                                            }
                                        } else {
                                            println!("HOTKEY: auto-paste enabled but text is empty, skipping");
                                        }
                                    } else {
                                        println!("HOTKEY: auto-paste disabled in config");
                                    }
                                    Ok::<_, String>(())
                                }));
                                if let Err(e) = result {
                                    let msg = match e.downcast::<String>() {
                                        Ok(s) => s.to_string(),
                                        Err(_) => "unknown panic in transcription thread".into(),
                                    };
                                    println!("TRANSCRIPTION PANIC: {msg}");
                                    let _ = app_clone.emit("transcription-error", msg);
                                }
                            });
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(AppState {
            recorder: Mutex::new(AudioRecorder::new()),
            config: Mutex::new(Config::load()),
            overlay: Mutex::new(None),
        })
        .setup(|app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            let state = app.state::<AppState>();
            let hotkey_str = state.config.lock().unwrap().hotkey.clone();
            drop(state);
            match parse_hotkey_str(&hotkey_str) {
                Ok((m, c)) => {
                    match app.global_shortcut().register(Shortcut::new(Some(m), c)) {
                        Ok(_) => println!("SUCCESS: Registered {} global shortcut!", hotkey_str),
                        Err(e) => println!("ERROR: Failed to register {}: {}", hotkey_str, e),
                    }
                }
                Err(e) => println!("ERROR: Invalid hotkey '{}': {}", hotkey_str, e),
            }

            let overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("overlay.html".into()),
            )
            .title("")
            .inner_size(120.0, 36.0)
            .always_on_top(true)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .skip_taskbar(true);

            let overlay = if let Some(monitor) = app.primary_monitor().ok().flatten() {
                let scale_factor = monitor.scale_factor();
                let logical_width = monitor.size().width as f64 / scale_factor;
                let x = (logical_width - 120.0) / 2.0;
                let y = 10.0;
                overlay.position(x, y).build()
            } else {
                overlay.build()
            };

            let state = app.state::<AppState>();
            match &overlay {
                Ok(w) => { let _ = w.hide(); println!("SETUP: overlay created"); }
                Err(e) => println!("SETUP: overlay creation error: {e}"),
            }
            *state.overlay.lock().unwrap() = overlay.ok();

            println!("SETUP: app ready — tray, overlay, hotkey all configured");
            
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
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            get_config,
            update_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

