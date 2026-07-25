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

struct AppState {
    recorder: Mutex<AudioRecorder>,
    config: Mutex<Config>,
    overlay: Mutex<Option<WebviewWindow>>,
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
    let launch_on_startup = new_config.launch_on_startup;
    *state.config.lock().unwrap() = new_config;
    state.config.lock().unwrap().save();

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
                    use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
                    if !shortcut.matches(Modifiers::CONTROL, Code::Space) {
                        return;
                    }
                    let state = app.state::<AppState>();
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
            use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, GlobalShortcutExt};
            
            let shortcut = Shortcut::new(
                Some(Modifiers::CONTROL),
                Code::Space,
            );
            match app.global_shortcut().register(shortcut) {
                Ok(_) => println!("SUCCESS: Registered Ctrl+Space global shortcut!"),
                Err(e) => println!("ERROR: Failed to register Ctrl+Space global shortcut: {e}"),
            }

            let overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("overlay.html".into()),
            )
            .title("")
            .inner_size(220.0, 60.0)
            .always_on_top(true)
            .decorations(false)
            .transparent(true)
            .skip_taskbar(true)
            .center()
            .build();

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

