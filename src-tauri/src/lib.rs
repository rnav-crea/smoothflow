mod audio;
mod config;
mod postprocess;
mod text_injection;
mod transcription;

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
    state.recorder.lock().unwrap().start()?;
    let _ = app.emit("recording-state", true);
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
    let text = transcription::transcribe(&samples, sample_rate, &config)?;
    println!("CMD: transcript received: {:?}", &text[..text.len().min(50)]);
    let text = postprocess::postprocess(&text, &config);
    
    let _ = app.emit("transcript-result", text.clone());
    
    if !text.is_empty() {
        println!("CMD: typing text...");
        text_injection::type_text(&text)?;
        println!("CMD: typed OK");
    }
    Ok(text)
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<Config, ()> {
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
fn update_config(app: tauri::AppHandle, state: tauri::State<AppState>, new_config: Config) -> Result<(), String> {
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
                    if !shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::Space) {
                        return;
                    }
                    let state = app.state::<AppState>();
                    eprintln!("HOTKEY: event state={:?}", event.state);
                    match event.state {
                        ShortcutState::Pressed => {
                            if !state.recorder.lock().unwrap().is_recording() {
                                if let Err(e) = state.recorder.lock().unwrap().start() {
                                    eprintln!("start_recording error: {e}");
                                } else {
                                    let _ = app.emit("recording-state", true);
                                    if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
                                        match overlay.show() {
                                            Ok(_) => println!("HOTKEY: overlay shown"),
                                            Err(e) => eprintln!("HOTKEY: overlay show error: {e}"),
                                        }
                                    } else {
                                        eprintln!("HOTKEY: overlay not available in state");
                                    }
                                }
                            }
                        }
                        ShortcutState::Released => {
                            let mut recorder = state.recorder.lock().unwrap();
                            if recorder.is_recording() {
                                let samples = recorder.stop();
                                let sample_rate = recorder.sample_rate();
                                drop(recorder);
                                let _ = app.emit("recording-state", false);

                                if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
                                    match overlay.hide() {
                                        Ok(_) => println!("HOTKEY: overlay hidden"),
                                        Err(e) => eprintln!("HOTKEY: overlay hide error: {e}"),
                                    }
                                }

                                let config = state.config.lock().unwrap();
                                match transcription::transcribe(&samples, sample_rate, &config) {
                                    Ok(text) => {
                                        let text = postprocess::postprocess(&text, &config);
                                        let _ = app.emit("transcript-result", text.clone());
                                        if !text.is_empty() {
                                            text_injection::type_text(&text).unwrap_or_else(|e| eprintln!("type_text error: {e}"));
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("transcription error: {e}");
                                        let _ = app.emit("transcription-error", e);
                                    }
                                }
                            }
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
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::Space,
            );
            match app.global_shortcut().register(shortcut) {
                Ok(_) => println!("SUCCESS: Registered Ctrl+Shift+Space global shortcut!"),
                Err(e) => eprintln!("ERROR: Failed to register Ctrl+Shift+Space global shortcut: {e}"),
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
                Err(e) => eprintln!("SETUP: overlay creation error: {e}"),
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

