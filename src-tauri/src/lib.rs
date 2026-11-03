mod accessibility;
mod audio;
pub mod config;
mod history;
#[cfg(target_os = "macos")]
mod hotkey;
mod llm_local;
pub mod local_models;
pub mod offline;
pub mod postprocess;
mod stt_local;
mod text_injection;
mod text_rules;
pub mod transcription;
mod wav;

use audio::AudioRecorder;
use config::Config;
use history::HistoryUpdatePayload;
use serde::Serialize;
use std::path::Path;
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
#[macro_export]
macro_rules! sf_log {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            let line = format!($($arg)*);
            println!("{line}");
        }
    };
}

/// Frontmost app at the moment the hotkey started recording (macOS only).
/// Captured before the overlay steals focus so we can reactivate the target
/// document right before auto-paste fires Cmd+V.
#[cfg(target_os = "macos")]
static LAST_FRONT_APP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

struct AppState {
    recorder: Mutex<AudioRecorder>,
    config: Mutex<Config>,
    overlay: Mutex<Option<WebviewWindow>>,
    /// Bumped by `cancel_transcription`. A run captures the value when it
    /// starts and aborts at each checkpoint once the value has moved.
    ///
    /// A generation counter rather than a kill: `spawn_blocking`/`std::thread`
    /// work cannot be interrupted, so a cancelled run finishes its current
    /// blocking call and then discards its result at the next checkpoint.
    cancel_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// True from the moment recording stops until the run finishes. Gates the
    /// hotkey so a press during transcription cancels instead of starting a
    /// second recording.
    transcribing: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Clears `transcribing` however the run ends — normal return, early return,
/// or a panic caught by the hotkey path's `catch_unwind`.
struct TranscribingGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for TranscribingGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Pre-warming is per leg, so a user pays only for the engine they actually use.
/// Speech-to-text and cleanup are now independent: a cloud-speech +
/// local-cleanup user warms only the LLM, a local-speech + no-cleanup user warms
/// only the helper, and a fully cloud user warms nothing.
fn should_warm_stt(config: &Config) -> bool {
    config.stt_local
}

/// Local cleanup only. `"cloud"` and `"off"` must never start llama-server.
fn should_warm_llm(config: &Config) -> bool {
    config.cleanup_engine == "local"
}

/// Start the engines this config actually uses in the background so the
/// Parakeet (~4 s) and llama-server (~9 s) model loads overlap the user speaking.
/// Must run off the UI thread — a warm is a few seconds of blocking work.
fn start_offline_engines(app: &tauri::AppHandle) {
    let config = app.state::<AppState>().config.lock().unwrap().clone();
    let stt = should_warm_stt(&config);
    let llm = should_warm_llm(&config);
    if !stt && !llm {
        return;
    }
    sf_log!("ENGINES: pre-warming local engines (stt={stt}, llm={llm})");
    std::thread::spawn(move || {
        if stt {
            stt_local::prewarm(&config);
        }
        if llm {
            llm_local::prewarm(&config);
        }
    });
}

/// Streaming flush pacing. `on_partial` can fire per token (~6/s here) and
/// pasting every token interleaves badly, so a buffer is flushed at a sentence
/// boundary or at this size — a slow reader never waits more than one sentence.
const FLUSH_CHARS: usize = 80;

fn should_flush(pending: &str) -> bool {
    pending.chars().count() >= FLUSH_CHARS
        || matches!(
            pending.trim_end().chars().last(),
            Some('.' | '!' | '?' | '\n')
        )
}

/// macOS: the overlay made SmoothFlow active, so hand focus back to the app
/// that was frontmost when recording started and let it settle before a paste.
#[cfg(target_os = "macos")]
fn reactivate_target_app() {
    let front = LAST_FRONT_APP.lock().unwrap().clone();
    if let Some(name) = front {
        if !name.is_empty() && name != "SmoothFlow" {
            sf_log!("HOTKEY: reactivating frontmost app: {}", name);
            let _ = std::process::Command::new("open").arg("-a").arg(&name).spawn();
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn reactivate_target_app() {}

fn type_text_timed(app: &tauri::AppHandle, tag: &str, text: &str) -> Result<(), String> {
    let t_inj = std::time::Instant::now();
    let result = text_injection::type_text(app, text);
    sf_log!("{tag}: type_text took {:?}", t_inj.elapsed());
    result
}

/// Postprocess a raw transcript and put the result into the focused field.
/// Shared by both dictation exits so the two paths cannot drift.
///
/// Only a local cleanup + auto-paste streams. Every other combination —
/// including cloud cleanup and cleanup off — is the one-shot path. `on_ready`
/// runs the moment the final text exists and before the paste, which is where
/// history and the UI update have always happened.
fn cleanup_and_paste(
    app: &tauri::AppHandle,
    raw: &str,
    config: &Config,
    tag: &str,
    on_ready: &mut dyn FnMut(&str),
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    let context = transcription::active_window_title();
    let streamed = config.cleanup_engine == "local" && config.auto_paste;
    if streamed {
        reactivate_target_app();
    }
    let t_pp = std::time::Instant::now();

    if !streamed {
        let text = postprocess::postprocess_with_context(raw, config, &context);
        sf_log!("{tag}: postprocess took {:?}", t_pp.elapsed());
        // Cancelled while the text was being cleaned: no history row, no paste.
        if cancelled() {
            return Ok(String::new());
        }
        on_ready(&text);
        if config.auto_paste && !text.is_empty() {
            reactivate_target_app();
            type_text_timed(app, tag, &text)?;
        }
        return Ok(text);
    }

    let mut session = text_injection::PasteSession::begin();
    let mut pending = String::new();
    let mut pasted_any = false;
    let mut flushes = 0usize;
    let text = postprocess::postprocess_with_context_streaming(
        raw,
        config,
        &context,
        &mut |chunk| {
            pending.push_str(chunk);
            if should_flush(&pending) && !cancelled() && session.append(&pending).is_ok() {
                pasted_any = true;
                flushes += 1;
                pending.clear();
            }
        },
    );
    // Cancelled mid-stream: stop appending. Text already in the field stays —
    // it cannot be un-typed, and deleting it would clobber whatever the user
    // has typed since.
    let cancelled_now = cancelled();
    if !cancelled_now && !pending.is_empty() && session.append(&pending).is_ok() {
        pasted_any = true;
        flushes += 1;
    }
    session.finish();
    sf_log!("{tag}: postprocess took {:?}", t_pp.elapsed());

    if cancelled_now {
        return Ok(String::new());
    }

    if pasted_any {
        // The text is already in the field — a one-shot paste here would
        // duplicate the whole dictation.
        sf_log!("{tag}: streamed cleanup into the field in {flushes} flushes");
        on_ready(&text);
        return Ok(text);
    }

    // `on_partial` never fired: casual fast path, empty transcript, or the LLM
    // dying before its first token. Nothing was pasted, so paste it once.
    on_ready(&text);
    if !text.is_empty() {
        type_text_timed(app, tag, &text)?;
    }
    Ok(text)
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
            if !*recording.lock().unwrap() {
                break;
            }
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

/// The overlay accepts clicks only while a transcription is running, so its ✕
/// is usable. Everywhere else it stays click-through.
fn set_overlay_clickable(app: &tauri::AppHandle, clickable: bool) {
    if let Some(overlay) = app.state::<AppState>().overlay.lock().unwrap().as_ref() {
        let _ = overlay.set_ignore_cursor_events(!clickable);
    }
}

/// End of a run, whatever the outcome. `hide` is false on the error path:
/// `show_error` just put the bubble up and the overlay hides itself after a
/// few seconds, but it must stop swallowing clicks either way.
fn overlay_idle(app: &tauri::AppHandle, hide: bool) {
    let _ = app.emit("transcription-state", "idle");
    set_overlay_clickable(app, false);
    if !hide {
        return;
    }
    if let Some(overlay) = app.state::<AppState>().overlay.lock().unwrap().as_ref() {
        let _ = overlay.hide();
    }
}

/// Single cancel exit for both transcription paths, so they cannot drift.
/// A cancelled run leaves no trace: no history row (so the clip has no owner
/// and is deleted), no paste, no error toast.
fn finish_cancelled(app: &tauri::AppHandle, audio_file: Option<&str>) {
    if let Some(name) = audio_file {
        history::remove_audio(name);
    }
    sf_log!("TRANSCRIBE: cancelled, discarding this run");
    overlay_idle(app, true);
}

/// Cancel the transcription that is currently running, if any. Called by the
/// overlay's ✕ button and by the global hotkey. Returns whether anything was
/// actually in flight.
///
/// The run itself is not killed — it is flagged and discards its result at its
/// next checkpoint. Synchronous on purpose: two atomics plus an emit, the same
/// microsecond class as `get_config`.
#[tauri::command]
fn cancel_transcription(app: tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    if !state.transcribing.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    state
        .cancel_gen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    sf_log!("TRANSCRIBE: cancel requested");
    overlay_idle(&app, true);
    true
}

fn parse_hotkey_str(s: &str) -> Result<(Modifiers, Code), String> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return Err(format!(
            "[CFG-005] Invalid hotkey '{}'. Use format like 'Ctrl+Space' or a bare key like 'Fn'.",
            s
        ));
    }
    let mut mods = Modifiers::empty();
    for m in &parts[..parts.len() - 1] {
        match m.trim().to_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "win" | "meta" | "super" | "logo" | "cmd" | "command" => mods |= Modifiers::SUPER,
            _ => {
                return Err(format!(
                    "[CFG-005] Invalid hotkey '{}'. Unknown modifier '{}'.",
                    s,
                    m.trim()
                ))
            }
        }
    }
    // key_raw keeps the user's original text for error messages; key is trimmed+lowercased for matching.
    let key_raw = parts[parts.len() - 1].trim();
    let key = key_raw.to_lowercase();
    let code = match key.as_str() {
        "space" => Code::Space,
        "tab" => Code::Tab,
        "enter" => Code::Enter,
        "escape" | "esc" => Code::Escape,
        "backspace" => Code::Backspace,
        "fn" => Code::Fn,
        "delete" => Code::Delete,
        "insert" => Code::Insert,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "arrowup" | "up" => Code::ArrowUp,
        "arrowdown" | "down" => Code::ArrowDown,
        "arrowleft" | "left" => Code::ArrowLeft,
        "arrowright" | "right" => Code::ArrowRight,
        "backquote" | "`" => Code::Backquote,
        "minus" | "-" => Code::Minus,
        "equal" | "=" => Code::Equal,
        "semicolon" | ";" => Code::Semicolon,
        "quote" | "'" => Code::Quote,
        "comma" | "," => Code::Comma,
        "period" | "." => Code::Period,
        "slash" | "/" => Code::Slash,
        "backslash" | "\\" => Code::Backslash,
        "bracketleft" | "[" => Code::BracketLeft,
        "bracketright" | "]" => Code::BracketRight,
        _ if key.len() == 1 => {
            let c = key.chars().next().unwrap().to_ascii_uppercase();
            match c {
                '0'..='9' => [
                    Code::Digit0,
                    Code::Digit1,
                    Code::Digit2,
                    Code::Digit3,
                    Code::Digit4,
                    Code::Digit5,
                    Code::Digit6,
                    Code::Digit7,
                    Code::Digit8,
                    Code::Digit9,
                ][c.to_digit(10).unwrap() as usize],
                'A'..='Z' => [
                    Code::KeyA,
                    Code::KeyB,
                    Code::KeyC,
                    Code::KeyD,
                    Code::KeyE,
                    Code::KeyF,
                    Code::KeyG,
                    Code::KeyH,
                    Code::KeyI,
                    Code::KeyJ,
                    Code::KeyK,
                    Code::KeyL,
                    Code::KeyM,
                    Code::KeyN,
                    Code::KeyO,
                    Code::KeyP,
                    Code::KeyQ,
                    Code::KeyR,
                    Code::KeyS,
                    Code::KeyT,
                    Code::KeyU,
                    Code::KeyV,
                    Code::KeyW,
                    Code::KeyX,
                    Code::KeyY,
                    Code::KeyZ,
                ][(c as u8 - b'A') as usize],
                _ => {
                    return Err(format!(
                        "[CFG-005] Invalid hotkey. Unknown key '{}'.",
                        key_raw
                    ))
                }
            }
        }
        _ if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => {
            match key[1..].parse::<u8>().unwrap() {
                1 => Code::F1,
                2 => Code::F2,
                3 => Code::F3,
                4 => Code::F4,
                5 => Code::F5,
                6 => Code::F6,
                7 => Code::F7,
                8 => Code::F8,
                9 => Code::F9,
                10 => Code::F10,
                11 => Code::F11,
                12 => Code::F12,
                13 => Code::F13,
                14 => Code::F14,
                15 => Code::F15,
                16 => Code::F16,
                17 => Code::F17,
                18 => Code::F18,
                19 => Code::F19,
                20 => Code::F20,
                21 => Code::F21,
                22 => Code::F22,
                23 => Code::F23,
                24 => Code::F24,
                n => return Err(format!("[CFG-005] Invalid hotkey. Unknown F-key F{}.", n)),
            }
        }
        _ => {
            return Err(format!(
                "[CFG-005] Invalid hotkey. Unknown key '{}'.",
                key_raw
            ))
        }
    };
    Ok((mods, code))
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    sf_log!("CMD: start_recording called");
    accessibility::request_once();
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
async fn stop_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    sf_log!("CMD: stop_recording called");
    let (samples, sample_rate) = {
        let mut recorder = state.recorder.lock().unwrap();
        let samples = recorder.stop();
        let sample_rate = recorder.sample_rate();
        (samples, sample_rate)
    };
    sf_log!("CMD: got {} samples at {} Hz", samples.len(), sample_rate);
    let _ = app.emit("recording-state", false);
    let config = state.config.lock().unwrap().clone();

    // Flag + token are taken on this thread, before the work is dispatched: a
    // cancel that lands while the task is still queued must be seen by the run,
    // and a hotkey press in that gap must cancel instead of starting a second
    // recording.
    state
        .transcribing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let token = state.cancel_gen.load(std::sync::atomic::Ordering::SeqCst);
    let guard = TranscribingGuard(state.transcribing.clone());
    let cancel_gen = state.cancel_gen.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let cancelled = || cancel_gen.load(std::sync::atomic::Ordering::SeqCst) != token;
        let t0 = std::time::Instant::now();
        // Clips are off by default (privacy contract): the WAV is only written
        // when the user turned `keep_audio_clips` on, and it never leaves the
        // machine. A failed write returns None and never fails the dictation.
        let audio_file = if config.keep_audio_clips {
            history::save_audio(&samples, sample_rate)
        } else {
            None
        };
        // Set once `history::add` succeeds in `on_ready`. If cleanup/paste fails
        // before that, the clip still has no owner row — see the `Err` arm.
        let mut row_written = false;
        // (a) cancelled while this task was queued behind other blocking work.
        if cancelled() {
            finish_cancelled(&app, audio_file.as_deref());
            return Ok(String::new());
        }
        sf_log!("CMD: transcribing...");
        let raw = match transcription::transcribe_any(&samples, sample_rate, &config) {
            Ok(r) => r,
            Err(e) => {
                // A cancel during STT is not a failure: no "failed" row, no
                // error toast, no clip left behind.
                if cancelled() {
                    finish_cancelled(&app, audio_file.as_deref());
                    return Ok(String::new());
                }
                // STT failed, but the clip survives: attach it to a text-less
                // history row so the user can retry the transcription from the
                // History tab instead of losing the audio. A history write
                // failure is logged and never masks the transcription error.
                if let Some(name) = audio_file.as_deref() {
                    match history::add_failed(
                        name,
                        Some(config.model.as_str()),
                        config.offline_mode,
                    ) {
                        Ok(entry) => {
                            let _ = app.emit(
                                "history-updated",
                                HistoryUpdatePayload::Added { entry },
                            );
                        }
                        Err(err) => {
                            sf_log!("HISTORY: could not record the failed dictation: {err}")
                        }
                    }
                }
                show_error(&app, "transcription-error", e.clone());
                // Error exit: the bubble stays up (it hides itself) but the
                // overlay must stop swallowing clicks.
                overlay_idle(&app, false);
                return Err(e);
            }
        };
        // (b) the transcript arrived, but the user cancelled while it was
        // being fetched. Discard it: no history row, no paste.
        if cancelled() {
            finish_cancelled(&app, audio_file.as_deref());
            return Ok(String::new());
        }
        sf_log!(
            "CMD: transcript received in {:?}: {:?}",
            t0.elapsed(),
            &raw[..raw.len().min(50)]
        );
        let _ = app.emit("raw-transcript", raw.clone());

        if !config.auto_paste {
            sf_log!("CMD: auto-paste disabled, showing in transcript only");
        }

        let mut on_ready = |text: &str| {
            // (c) cancelled before the final text existed: no history row and
            // no `transcript-result` emit.
            if cancelled() {
                return;
            }
            // macOS: near-silent audio (typically missing Mic permission) yields
            // empty text — surface it instead of silently doing nothing.
            #[cfg(target_os = "macos")]
            if text.is_empty() {
                show_error(&app, "transcription-error", "No speech detected. Check SmoothFlow's Microphone permission in System Settings → Privacy & Security → Microphone, then try again.".into());
}

            // Persist non-empty dictations to history (independent of auto-paste).
            // `transcription_text` is the raw STT output; the cleaned text the
            // user actually got is `post_processed_text`.
            if !text.is_empty() {
                match history::add(history::NewDictation {
                    transcription_text: raw.as_str(),
                    post_processed_text: Some(text),
                    post_process_prompt: None,
                    post_process_requested: config.cleanup_engine != "off",
                    audio_file: audio_file.as_deref(),
                    source_name: None,
                    model: Some(config.model.as_str()),
                    offline: config.offline_mode,
                }) {
                    Ok(entry) => {
                        row_written = true;
                        let _ = app.emit(
                            "history-updated",
                            HistoryUpdatePayload::Added { entry },
                        );
                    }
                    Err(e) => sf_log!("HISTORY: could not record the dictation: {e}"),
                }
            }

            let _ = app.emit("transcript-result", text.to_string());
        };

        let result = cleanup_and_paste(&app, &raw, &config, "CMD", &mut on_ready, &cancelled);
        // (e) belt and braces: a cancel that landed during cleanup/paste, or an
        // error raised after it, takes the cancel exit instead of the normal
        // Ok/Err handling. `row_written` covers the one race where
        // `history::add` already succeeded — that clip has an owner row, so it
        // stays on disk.
        if cancelled() {
            let clip = if row_written {
                None
            } else {
                audio_file.as_deref()
            };
            finish_cancelled(&app, clip);
            return Ok(String::new());
        }
        let outcome = match result {
            Ok(text) => {
                // A silent dictation never reaches history (the `!text.is_empty()`
                // guard in `on_ready`), so its clip has no owner — drop it rather
                // than leave a file nothing points at.
                if text.is_empty() {
                    if let Some(name) = audio_file.as_deref() {
                        history::remove_audio(name);
                    }
                }
                Ok(text)
            }
            Err(e) => {
                // STT succeeded but cleanup/paste failed. If no row was written
                // the clip would leak on disk with nothing pointing at it — give
                // it a failed row instead, so it stays retryable.
                if !row_written {
                    if let Some(name) = audio_file.as_deref() {
                        match history::add_failed(
                            name,
                            Some(config.model.as_str()),
                            config.offline_mode,
                        ) {
                            Ok(entry) => {
                                let _ = app.emit(
                                    "history-updated",
                                    HistoryUpdatePayload::Added { entry },
                                );
                            }
                            Err(err) => {
                                sf_log!("HISTORY: could not record the failed dictation: {err}")
                            }
                        }
                    }
                }
                show_error(&app, "transcription-error", e.clone());
                Err(e)
            }
        };
        // Success returns the overlay to idle; the error path keeps the bubble
        // up (it hides itself) but stops swallowing clicks.
        overlay_idle(&app, outcome.is_ok());
        outcome
    })
    .await
    .map_err(|e| format!("[CMD-001] The transcription task failed: {e}"))?
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<Config, ()> {
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
fn update_config(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    mut new_config: Config,
) -> Result<(), String> {
    // A stale UI (or a hand-edited file) can send a value that is not one of
    // "off" | "local" | "cloud". Normalise before validating, so the checks
    // below and the saved file always agree.
    new_config.normalize();
    // The cloud speech leg needs an endpoint, a key and a model name. A local
    // STT user needs none of them.
    if !new_config.stt_local {
        if new_config.api_key.is_empty() {
            return Err("[CFG-002] API key cannot be empty. Enter a valid key in Settings.".into());
        }
        if new_config.model.is_empty() {
            return Err(
                "[CFG-003] Model name cannot be empty. Enter a model (e.g. whisper-large-v3)."
                    .into(),
            );
        }
        if !new_config.api_base_url.starts_with("https://") {
            return Err(
                "[CFG-004] API base URL must use HTTPS (e.g. https://api.groq.com/openai/v1)."
                    .into(),
            );
        }
    }
    // Cloud cleanup is a second, independent call to the same provider.
    if new_config.cleanup_engine == "cloud" && new_config.api_key.is_empty() {
        return Err(
            "[CFG-005] Cloud AI cleanup needs an API key. Add your API key in Settings, or switch AI cleanup to Local or Off."
                .into(),
        );
    }
    // VULN-003: the key goes into the OS vault before anything is saved;
    // Config::save() strips api_key from the JSON written to disk.
    if !new_config.api_key.is_empty() {
        config::store_secret(&new_config.api_key)?;
    }
    let new_hotkey = new_config.hotkey.clone();
    let launch_on_startup = new_config.launch_on_startup;
    let old_hotkey = {
        let config = state.config.lock().unwrap();
        let old = config.hotkey.clone();
        drop(config);
        old
    };

    if old_hotkey != new_hotkey {
        let (nm, nc) = parse_hotkey_str(&new_hotkey)?;
        if cfg!(target_os = "macos") && nm.is_empty() && nc == Code::Fn {
            #[cfg(target_os = "macos")]
            hotkey::start_fn_tap(app.clone())
                .map_err(|e| format!("Failed to register hotkey '{}': {}", new_hotkey, e))?;
            // Old hotkey was a global-shortcut chord — drop it.
            if let Ok((om, oc)) = parse_hotkey_str(&old_hotkey) {
                let _ = app
                    .global_shortcut()
                    .unregister(Shortcut::new(Some(om), oc));
            }
        } else {
            // Leaving the Fn tap (macOS) — stop it before registering a chord.
            #[cfg(target_os = "macos")]
            if hotkey::is_running() {
                hotkey::stop_fn_tap();
            }
            app.global_shortcut().register(Shortcut::new(Some(nm), nc)).map_err(|e| {
                let mut msg = format!("Failed to register hotkey '{}': {}", new_hotkey, e);
                if cfg!(target_os = "macos") {
                    msg.push_str(" — this combination may be reserved by macOS (Spotlight owns Cmd+Space); try Option+Space or Ctrl+Opt");
                }
                msg
            })?;
            if let Ok((om, oc)) = parse_hotkey_str(&old_hotkey) {
                let _ = app
                    .global_shortcut()
                    .unregister(Shortcut::new(Some(om), oc));
            }
        }
        sf_log!("HOTKEY: re-registered to {}", new_hotkey);
    }

    {
        let mut config = state.config.lock().unwrap();
        *config = new_config;
        config.save();
    }

    // Reads the just-saved config, so each leg warms only if it is enabled.
    start_offline_engines(&app);

    if launch_on_startup {
        let _ = app.autolaunch().enable();
    } else {
        let _ = app.autolaunch().disable();
    }

    Ok(())
}

// History commands. All synchronous on purpose: each one is a single SQLite
// statement against a ≤100-row table, so this is the "microsecond" class
// AGENTS.md allows on the main thread (like `get_config`).
#[tauri::command]
fn get_history(
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<history::HistoryPage, String> {
    history::list(cursor, limit)
}

#[tauri::command]
fn get_history_stats() -> Result<history::HistoryStats, String> {
    history::stats()
}

#[tauri::command]
fn delete_history_entry(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    history::delete(id)?;
    let _ = app.emit("history-updated", HistoryUpdatePayload::Deleted { id });
    Ok(())
}

#[tauri::command]
fn toggle_history_saved(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let saved = history::toggle_saved(id)?;
    let _ = app.emit("history-updated", HistoryUpdatePayload::Toggled { id, saved });
    Ok(())
}

#[tauri::command]
fn clear_history(app: tauri::AppHandle) -> Result<(), String> {
    history::clear()?;
    let _ = app.emit("history-updated", HistoryUpdatePayload::Cleared);
    Ok(())
}

/// Stats are derived from the table, so "reset stats" can only mean deleting
/// every dictation. Same effect as `clear_history`, kept as its own command
/// because the UI exposes it separately.
#[tauri::command]
fn reset_stats(app: tauri::AppHandle) -> Result<(), String> {
    history::reset()?;
    let _ = app.emit("history-updated", HistoryUpdatePayload::Cleared);
    Ok(())
}

/// Absolute path of a stored clip, for the History player.
///
/// Synchronous on purpose: this is a path join, not file I/O. The frontend
/// feeds the result to `convertFileSrc`, which the asset protocol resolves.
#[tauri::command]
fn get_audio_file_path(file_name: String) -> Result<String, String> {
    history::audio_path(&file_name)?
        .to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "[HIST-012] Invalid audio file path".to_string())
}

/// Re-run speech-to-text and AI cleanup over a history entry's saved clip.
///
/// Phase 3's reason for existing: when the cleanup model fails, `finish()` in
/// `postprocess.rs` silently degrades to the deterministic-rules text, and the
/// only way to get a clean transcript used to be to speak the whole thing
/// again. The clip makes that unnecessary — the audio is already on disk, so
/// the transcript can be regenerated instead of re-dictated.
///
/// Deliberately does **not** auto-paste. This runs from a click inside the
/// settings window, so the focused field is SmoothFlow's own UI; typing into
/// whatever happened to be focused would be a surprise. The refreshed text
/// lands in the row (and the copy button stays available). See
/// `cleanup_and_paste` for the pasting path used by live dictation.
///
/// `spawn_blocking`: a cloud STT call plus an LLM round-trip is hundreds of
/// milliseconds to seconds, which is exactly the class of work AGENTS.md says
/// must not sit on the async runtime's worker threads.
#[tauri::command]
async fn redictate_from_clip(
    app: tauri::AppHandle,
    id: i64,
) -> Result<history::HistoryEntry, String> {
    // Read the clip off the async runtime too — a multi-MB file plus header
    // validation is file I/O, however fast it usually is.
    let (samples, sample_rate) = {
        // Two `?`s on purpose: the outer unwraps the `spawn_blocking` join
        // handle, the inner unwraps `load_clip` itself.
        let loaded = tauri::async_runtime::spawn_blocking(move || history::load_clip(id))
            .await
            .map_err(|e| format!("[HIST-016] The clip read task failed: {e}"))?;
        loaded?.ok_or_else(|| {
            "[HIST-016] This dictation has no saved audio clip. Turn on \
             \"Save audio clips\" in Settings before dictating to enable re-dictation."
                .to_string()
        })?
    };

    let config = app.state::<AppState>().config.lock().unwrap().clone();

    let updated = {
        let run = tauri::async_runtime::spawn_blocking(move || {
            let t0 = std::time::Instant::now();
            sf_log!(
                "RE-DICTATE: re-transcribing {} samples at {} Hz",
                samples.len(),
                sample_rate
            );
            let raw = transcription::transcribe_any(&samples, sample_rate, &config)?;
            sf_log!("RE-DICTATE: transcript received in {:?}", t0.elapsed());

            let context = transcription::active_window_title();
            let text = postprocess::postprocess_with_context(&raw, &config, &context);
            if text.is_empty() {
                return Err("[HIST-017] No speech detected in the saved clip.".to_string());
            }

            // The new raw and the text cleaned from it move together, so the
            // row never pairs one run's raw transcript with another's result.
            history::update_after_redictation(id, &raw, &text)
        })
        .await
        .map_err(|e| format!("[HIST-016] The re-dictation task failed: {e}"))?;
        run?
    };

    let _ = app.emit(
        "history-updated",
        HistoryUpdatePayload::Updated {
            entry: updated.clone(),
        },
    );
    let _ = app.emit("transcript-result", updated.post_processed_text.clone().unwrap_or_default());
    Ok(updated)
}

/// Turn a user-picked WAV file into a History row.
///
/// History only: nothing is pasted, and the file is not copied — the row keeps
/// its name in `source_name` and has no `audio_file`, so it cannot be played
/// or re-dictated. That is deliberate: the picked file belongs to the user and
/// may be deleted at any time, so a row that pointed at it would rot.
///
/// WAV only: `hound` is already a dependency, so a picked file is decoded with
/// no new crate and no platform media stack. An M4A/MP3 has to be converted
/// first — that is the user-visible limit of this command.
///
/// `spawn_blocking`: decoding plus an STT round-trip plus cleanup is seconds of
/// work, which AGENTS.md forbids on the async runtime's worker threads.
#[tauri::command]
async fn transcribe_audio_file(
    app: tauri::AppHandle,
    path: String,
) -> Result<history::HistoryEntry, String> {
    let config = app.state::<AppState>().config.lock().unwrap().clone();

    let entry = {
        let run = tauri::async_runtime::spawn_blocking(move || {
            let p = std::path::PathBuf::from(&path);
            if !p.is_file() {
                return Err(format!("[FILE-001] {path} is no longer there"));
            }
            let is_wav = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("wav"))
                .unwrap_or(false);
            if !is_wav {
                return Err(format!(
                    "[FILE-002] Only WAV files can be transcribed, and {path} is not one"
                ));
            }

            let (samples, sample_rate) = wav::read_wav_mono(&p)?;
            sf_log!(
                "FILE: transcribing {} samples at {} Hz from {path}",
                samples.len(),
                sample_rate
            );
            let raw = transcription::transcribe_any(&samples, sample_rate, &config)?;

            // No active window for a picked file, so cleanup gets no context —
            // the field-context hint only makes sense for live dictation.
            let text = postprocess::postprocess_with_context(&raw, &config, "");
            if text.is_empty() {
                return Err("[FILE-003] No speech detected in that file.".to_string());
            }

            let file_name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("audio file");
            history::add(history::NewDictation {
                transcription_text: &raw,
                post_processed_text: Some(&text),
                post_process_prompt: None,
                post_process_requested: config.cleanup_engine != "off",
                audio_file: None,
                source_name: Some(file_name),
                model: Some(config.model.as_str()),
                offline: config.offline_mode,
            })
        })
        .await
        .map_err(|e| format!("[FILE-004] The file transcription task failed: {e}"))?;
        run?
    };

    // The same two events a dictation emits, so the History list and the
    // transcript panel update live instead of needing a refetch.
    let _ = app.emit(
        "history-updated",
        HistoryUpdatePayload::Added {
            entry: entry.clone(),
        },
    );
    let _ = app.emit(
        "transcript-result",
        entry.post_processed_text.clone().unwrap_or_default(),
    );
    Ok(entry)
}

#[tauri::command]
async fn inject_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    // ponytail: spawn_blocking, not #[command(async)] — enigo has no async API,
    // and a blocking call on a tokio worker starves the runtime for its timeout.
    tauri::async_runtime::spawn_blocking(move || text_injection::type_text(&app, &text))
        .await
        .map_err(|e| format!("[INJ-001] The text injection task failed: {e}"))?
}

#[tauri::command]
async fn test_api_connection(base_url: String, api_key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || transcription::verify_api(&base_url, &api_key))
        .await
        .map_err(|e| format!("[NET-001] The connection test task failed: {e}"))?
}

#[derive(Clone, Serialize)]
struct ModelDownloadProgress<'a> {
    id: &'a str,
    status: &'a str,
    message: &'a str,
}

#[tauri::command]
fn get_offline_status(state: tauri::State<AppState>) -> Result<offline::ModelStatus, String> {
    let custom_dir = state.config.lock().unwrap().models_dir.clone();
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();
    offline::status(&config_dir, &custom_dir)
}

/// Where models are read from right now. Separate from `get_offline_status`
/// so the existing settings payload stays byte-identical.
#[derive(Clone, Serialize)]
pub struct ModelsLocation {
    /// Absolute path files are actually loaded from.
    pub resolved: String,
    /// True when `models_dir` in config overrides the default location.
    pub is_custom: bool,
}

fn models_location(config_dir: &Path, models_dir: &str) -> ModelsLocation {
    ModelsLocation {
        resolved: offline::models_dir(config_dir, models_dir)
            .display()
            .to_string(),
        is_custom: !models_dir.trim().is_empty(),
    }
}

#[tauri::command]
fn get_models_dir(state: tauri::State<AppState>) -> Result<ModelsLocation, String> {
    let custom_dir = state.config.lock().unwrap().models_dir.clone();
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();
    Ok(models_location(&config_dir, &custom_dir))
}

#[tauri::command]
async fn download_models(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    stt_version: String,
    llm_size: String,
) -> Result<offline::ModelStatus, String> {
    let custom_dir = state.config.lock().unwrap().models_dir.clone();
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();

    // ponytail: spawn_blocking, not #[command(async)] — a multi-minute download
    // on a tokio worker starves every other async task; the blocking pool won't.
    tauri::async_runtime::spawn_blocking(move || {
        offline::download_bundle(
            &stt_version,
            &llm_size,
            &config_dir,
            &custom_dir,
            |id, status, message| {
                let _ = app.emit(
                    "model-download-progress",
                    ModelDownloadProgress {
                        id,
                        status,
                        message,
                    },
                );
            },
        )?;
        offline::status(&config_dir, &custom_dir)
    })
    .await
    .map_err(|e| format!("[DL-001] The model download task failed: {e}"))?
}

#[tauri::command]
async fn delete_models(state: tauri::State<'_, AppState>) -> Result<offline::ModelStatus, String> {
    let custom_dir = state.config.lock().unwrap().models_dir.clone();
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        offline::delete(&config_dir, &custom_dir)?;
        offline::status(&config_dir, &custom_dir)
    })
    .await
    .map_err(|e| format!("[DL-002] The model delete task failed: {e}"))?
}

/// Text-in / text-out check for the on-device cleanup model: deterministic rules
/// first, then local LLM cleanup. STT is audio-only, so it is not covered here.
#[tauri::command]
async fn test_local_pipeline(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<String, String> {
    let config = state.config.lock().unwrap().clone();
    if config.cleanup_engine != "local" {
        return Err(
            "[OFF-001] Enable Local AI cleanup first — this tests the local cleanup model only."
                .into(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || {
        let cleaned = llm_local::cleanup(&text, &config, "")?;
        Ok(postprocess::postprocess(&cleaned, &config))
    })
    .await
    .map_err(|e| format!("[OFF-002] The local pipeline test task failed: {e}"))?
}

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// Start recording + show the overlay. Shared by the global-shortcut handler
/// and the macOS Fn event tap. Returns whether recording actually started.
pub(crate) fn hotkey_start(app: &tauri::AppHandle) -> bool {
    accessibility::request_once();
    let state = app.state::<AppState>();
    // A run is still in flight. The press arm cancels it instead of starting
    // another; this is the second line of defence so two recordings can never
    // overlap.
    if state.transcribing.load(std::sync::atomic::Ordering::SeqCst) {
        sf_log!("HOTKEY: transcription in flight, not starting a second recording");
        return false;
    }
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
        // Read the cleanup model's instruction sheet for THIS window while the
        // user is still speaking: prefill costs ~14-27 s and the title must be
        // read before the overlay can steal focus. Best-effort, never blocks
        // recording.
        {
            let config = state.config.lock().unwrap().clone();
            if should_warm_llm(&config) {
                let context = transcription::active_window_title();
                std::thread::spawn(move || llm_local::prime_for_category(&config, &context));
            }
        }
        #[cfg(target_os = "macos")]
        {
            // Capture the frontmost app BEFORE the overlay steals focus, so the
            // target document can be reactivated right before auto-paste.
            let auto_paste = {
                let config = state.config.lock().unwrap();
                config.auto_paste
            };
            if auto_paste {
                let handle = std::thread::spawn(|| {
                    let out = std::process::Command::new("osascript")
                        .args(["-e", r#"tell application "System Events" to get name of first application process whose frontmost is true"#])
                        .output();
                    match out {
                        Ok(o) if o.status.success() => {
                            let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !name.is_empty() {
                                *LAST_FRONT_APP.lock().unwrap() = Some(name.clone());
                                sf_log!("HOTKEY: captured frontmost app: {}", name);
                            } else {
                                sf_log!("HOTKEY: osascript returned empty frontmost app name");
                            }
                        }
                        Ok(o) => sf_log!("HOTKEY: osascript exited with status {}", o.status),
                        Err(e) => sf_log!("HOTKEY: osascript failed: {e}"),
                    }
                });
                let _ = handle.join();
            }
        }
        if let Some(overlay) = state.overlay.lock().unwrap().as_ref() {
            match overlay.show() {
                Ok(_) => sf_log!("HOTKEY: overlay shown"),
                Err(e) => sf_log!("HOTKEY: overlay show error: {e}"),
            }
        } else {
            sf_log!("HOTKEY: overlay not available in state");
        }
    }
    started
}

/// Stop recording, keep the overlay up, and kick off the transcription thread.
/// Shared by the global-shortcut handler and the macOS Fn event tap.
pub(crate) fn hotkey_finalize(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let mut recorder = state.recorder.lock().unwrap();
    if !recorder.is_recording() {
        return;
    }
    let t_stop = std::time::Instant::now();
    let samples = recorder.stop();
    let sample_rate = recorder.sample_rate();
    drop(recorder);
    sf_log!("PERF: recorder.stop() took {:?}", t_stop.elapsed());
    let _ = app.emit("recording-state", false);

    // The overlay stays visible while transcribing: its ✕ is the only way to
    // abort a run, so it must be clickable. The idle transition hides it.
    let _ = app.emit("transcription-state", "working");
    set_overlay_clickable(app, true);

    let config = state.config.lock().unwrap().clone();
    // Same as the command path: flag + token before the work is dispatched.
    state
        .transcribing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let token = state.cancel_gen.load(std::sync::atomic::Ordering::SeqCst);
    let guard = TranscribingGuard(state.transcribing.clone());
    let cancel_gen = state.cancel_gen.clone();
    let app_clone = app.clone();

    // ponytail: spawn to unblock shortcut thread during HTTP calls
    std::thread::spawn(move || {
        let _guard = guard;
        let cancelled = || cancel_gen.load(std::sync::atomic::Ordering::SeqCst) != token;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Same clip contract as the command path: written only when the
            // user opted in, never allowed to fail the dictation.
            let audio_file = if config.keep_audio_clips {
                history::save_audio(&samples, sample_rate)
            } else {
                None
            };
            // Set once `history::add` succeeds in `on_ready`; see the `Err` arm.
            let mut row_written = false;
            // (a) cancelled while this thread was being scheduled.
            if cancelled() {
                finish_cancelled(&app_clone, audio_file.as_deref());
                return Ok(());
            }
            let t_encode = std::time::Instant::now();
            let raw = match transcription::transcribe_any(&samples, sample_rate, &config) {
                Ok(r) => r,
                Err(e) => {
                    // A cancel during STT is not a failure: no "failed" row.
                    if cancelled() {
                        finish_cancelled(&app_clone, audio_file.as_deref());
                        return Ok(());
                    }
                    // Same as the command path: keep the clip by giving it a
                    // text-less row the user can re-transcribe from History.
                    if let Some(name) = audio_file.as_deref() {
                        match history::add_failed(
                            name,
                            Some(config.model.as_str()),
                            config.offline_mode,
                        ) {
                            Ok(entry) => {
                                let _ = app_clone.emit(
                                    "history-updated",
                                    HistoryUpdatePayload::Added { entry },
                                );
                            }
                            Err(err) => sf_log!(
                                "HISTORY: could not record the failed dictation: {err}"
                            ),
                        }
                    }
                    return Err(e);
                }
            };
            // (b) the transcript arrived, but the user cancelled meanwhile.
            if cancelled() {
                finish_cancelled(&app_clone, audio_file.as_deref());
                return Ok(());
            }
            sf_log!("PERF: STT (total) took {:?}", t_encode.elapsed());
            let _ = app_clone.emit("raw-transcript", raw.clone());
            if !config.auto_paste {
                sf_log!("HOTKEY: auto-paste disabled in config");
            }
            let mut on_ready = |text: &str| {
                // (c) cancelled before the final text existed: no history row
                // and no `transcript-result` emit.
                if cancelled() {
                    return;
                }
                #[cfg(target_os = "macos")]
                if text.is_empty() {
                    show_error(&app_clone, "transcription-error", "No speech detected. Check SmoothFlow's Microphone permission in System Settings → Privacy & Security → Microphone, then try again.".into());
                }
                // Raw STT text in `transcription_text`, cleaned text in
                // `post_processed_text` — same contract as the command path.
                if !text.is_empty() {
                    match history::add(history::NewDictation {
                        transcription_text: raw.as_str(),
                        post_processed_text: Some(text),
                        post_process_prompt: None,
                        post_process_requested: config.cleanup_engine != "off",
                        audio_file: audio_file.as_deref(),
                        source_name: None,
                        model: Some(config.model.as_str()),
                        offline: config.offline_mode,
                    }) {
                        Ok(entry) => {
                            row_written = true;
                            let _ = app_clone.emit(
                                "history-updated",
                                HistoryUpdatePayload::Added { entry },
                            );
                        }
                        Err(e) => sf_log!("HISTORY: could not record the dictation: {e}"),
                    }
                }
                let _ = app_clone.emit("transcript-result", text.to_string());
            };
            let result =
                cleanup_and_paste(&app_clone, &raw, &config, "PERF", &mut on_ready, &cancelled);
            // (e) belt and braces: a cancel that landed during cleanup/paste,
            // or an error raised after it, takes the cancel exit. `row_written`
            // covers the race where `history::add` already succeeded — that
            // clip has an owner row, so it stays on disk.
            if cancelled() {
                let clip = if row_written {
                    None
                } else {
                    audio_file.as_deref()
                };
                finish_cancelled(&app_clone, clip);
                return Ok(());
            }
            match result {
                Ok(text) if text.is_empty() => {
                    if config.auto_paste {
                        sf_log!("HOTKEY: auto-paste enabled but text is empty, skipping");
                    }
                    // Silent dictation: no history row, so the clip has no owner.
                    if let Some(name) = audio_file.as_deref() {
                        history::remove_audio(name);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    sf_log!("HOTKEY: type_text error: {e}");
                    // Cleanup/paste failed before a row was written: keep the
                    // clip on a text-less row so it stays retryable.
                    if !row_written {
                        if let Some(name) = audio_file.as_deref() {
                            match history::add_failed(
                                name,
                                Some(config.model.as_str()),
                                config.offline_mode,
                            ) {
                                Ok(entry) => {
                                    let _ = app_clone.emit(
                                        "history-updated",
                                        HistoryUpdatePayload::Added { entry },
                                    );
                                }
                                Err(err) => sf_log!(
                                    "HISTORY: could not record the failed dictation: {err}"
                                ),
                            }
                        }
                    }
                    show_error(&app_clone, "transcription-error", e);
                }
            }
            Ok::<_, String>(())
        }));
        // Success hides the pill; an error keeps the bubble up (it hides
        // itself) but stops swallowing clicks. A cancelled run already went
        // idle in `finish_cancelled`.
        let success = matches!(&result, Ok(Ok(())));
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
        if !cancelled() {
            overlay_idle(&app_clone, success);
        }
    });
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
                        if !shortcut.matches(m, c) {
                            return;
                        }
                    } else {
                        return;
                    }
                    let toggle_mode = cfg.hotkey_mode == "toggle";
                    drop(cfg);
                    sf_log!("HOTKEY: event state={:?}", event.state);
                    match event.state {
                        ShortcutState::Pressed => {
                            // A press while a run is in flight aborts it rather
                            // than starting a second recording on top of it.
                            if state.transcribing.load(std::sync::atomic::Ordering::SeqCst) {
                                cancel_transcription(app.clone());
                                return;
                            }
                            if toggle_mode {
                                // Toggle shortcut: first press starts, the next
                                // press stops. The release below is deliberately
                                // ignored, or hold logic would cut the dictation
                                // immediately.
                                let recording = state
                                    .recorder
                                    .lock()
                                    .unwrap()
                                    .is_recording();
                                if recording {
                                    hotkey_finalize(app);
                                } else {
                                    hotkey_start(app);
                                }
                            } else {
                                hotkey_start(app);
                            }
                        }
                        ShortcutState::Released => {
                            // Hold mode finalises on release. Toggle mode finished
                            // in the press arm above.
                            if !toggle_mode {
                                hotkey_finalize(app);
                            }
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            recorder: Mutex::new(AudioRecorder::new()),
            config: Mutex::new(Config::load()),
            overlay: Mutex::new(None),
            cancel_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            transcribing: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
        .setup(|app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            // Creates history.db (and imports the legacy history.json) before
            // the first dictation. Logs and continues on failure.
            history::init();

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
                    if cfg!(target_os = "macos") && m.is_empty() && c == Code::Fn {
                        // Bare Fn has no Carbon scancode — use the CGEventTap
                        // backend instead of the global-shortcut crate.
                        #[cfg(target_os = "macos")]
                        match hotkey::start_fn_tap(app.handle().clone()) {
                            Ok(_) => sf_log!("SUCCESS: Registered {} Fn event tap!", hotkey_str),
                            Err(e) => {
                                sf_log!("ERROR: Failed to register {}: {}", hotkey_str, e);
                                let _ = app.emit(
                                    "hotkey-error",
                                    format!("Failed to register hotkey '{}': {}", hotkey_str, e),
                                );
                            }
                        }
                    } else {
                        match app.global_shortcut().register(Shortcut::new(Some(m), c)) {
                            Ok(_) => sf_log!("SUCCESS: Registered {} global shortcut!", hotkey_str),
                            Err(e) => {
                                sf_log!("ERROR: Failed to register {}: {}", hotkey_str, e);
                                let _ = app.emit(
                                    "hotkey-error",
                                    format!("Failed to register hotkey '{}': {}", hotkey_str, e),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    sf_log!("ERROR: Invalid hotkey '{}': {}", hotkey_str, e);
                    let _ = app.emit(
                        "hotkey-error",
                        format!("Invalid hotkey '{}': {}", hotkey_str, e),
                    );
                }
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
                // Top-center, under the webcam — the fixed position.
                let y = 10.0;
                overlay.position(x, y).build()
            } else {
                overlay.build()
            };

            let state = app.state::<AppState>();
            match &overlay {
                Ok(w) => {
                    let _ = w.hide();
                    let _ = w.set_ignore_cursor_events(true);
                    // Clickable only while transcribing (see
                    // `set_overlay_clickable`), and never focusable: the ✕ can
                    // be clicked without the overlay activating and redirecting
                    // the auto-paste target into itself.
                    let _ = w.set_focusable(false);
                    sf_log!("SETUP: overlay created");
                }
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

            start_offline_engines(app.handle());

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
                .on_menu_event(|app, event| match event.id().as_ref() {
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
                        llm_local::shutdown();
                        app.exit(0);
                    }
                    _ => {}
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
                llm_local::shutdown();
                app.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            get_config,
            update_config,
            get_history,
            get_history_stats,
            delete_history_entry,
            toggle_history_saved,
            clear_history,
            reset_stats,
            get_audio_file_path,
            redictate_from_clip,
            transcribe_audio_file,
            cancel_transcription,
            inject_text,
            test_api_connection,
            get_offline_status,
            get_models_dir,
            download_models,
            delete_models,
            test_local_pipeline,
            local_models::list_local_models,
            local_models::inspect_stt_dir,
            local_models::scan_models_folder,
            local_models::open_models_folder,
            restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkey_cmd_alias() {
        let (mods, code) = parse_hotkey_str("Cmd+Space").expect("Cmd+Space should parse");
        assert!(mods.contains(Modifiers::SUPER));
        assert_eq!(code, Code::Space);
    }

    #[test]
    fn parse_hotkey_case_insensitive() {
        let (m1, c1) = parse_hotkey_str("ctrl+space").expect("lowercase should parse");
        let (m2, c2) = parse_hotkey_str("Ctrl+Space").expect("mixed case should parse");
        assert_eq!(m1, m2);
        assert_eq!(c1, c2);
        assert!(m1.contains(Modifiers::CONTROL));
        assert_eq!(c1, Code::Space);
    }

    #[test]
    fn parse_hotkey_whitespace_tolerant() {
        let (mods, code) = parse_hotkey_str(" Ctrl + Space ").expect("whitespace should parse");
        assert!(mods.contains(Modifiers::CONTROL));
        assert_eq!(code, Code::Space);
    }

    #[test]
    fn parse_hotkey_lowercase_fkey() {
        let (mods, code) = parse_hotkey_str("alt+f5").expect("lowercase F-key should parse");
        assert!(mods.contains(Modifiers::ALT));
        assert_eq!(code, Code::F5);
    }

    #[test]
    fn models_location_reports_the_resolved_dir_and_custom_flag() {
        let root = Path::new("C:/appdata/SmoothFlow");
        let loc = models_location(root, "");
        assert_eq!(loc.resolved, root.join("models").display().to_string());
        assert!(!loc.is_custom);
        // Whitespace-only is not a custom dir; it resolves to the default.
        assert!(!models_location(root, "   ").is_custom);
        let loc = models_location(root, "D:/elsewhere");
        assert_eq!(
            loc.resolved,
            Path::new("D:/elsewhere").display().to_string()
        );
        assert!(loc.is_custom);
    }

    #[test]
    fn parse_hotkey_lowercase_letter() {
        let (_, code) = parse_hotkey_str("a").expect("single lowercase letter should parse");
        assert_eq!(code, Code::KeyA);
    }

    #[test]
    fn each_engine_is_warmed_only_for_its_own_leg() {
        let cloud = Config::default();
        assert!(!should_warm_stt(&cloud), "cloud speech must not warm the STT helper");
        assert!(!should_warm_llm(&cloud), "cleanup off must not warm llama-server");

        let local_stt = Config {
            stt_local: true,
            ..Config::default()
        };
        assert!(should_warm_stt(&local_stt));
        assert!(!should_warm_llm(&local_stt));

        let local_llm = Config {
            cleanup_engine: "local".into(),
            ..Config::default()
        };
        assert!(!should_warm_stt(&local_llm));
        assert!(should_warm_llm(&local_llm));

        // Cloud speech + local cleanup: only the LLM warms.
        let both = Config {
            cleanup_engine: "cloud".into(),
            ..Config::default()
        };
        assert!(!should_warm_stt(&both));
        assert!(!should_warm_llm(&both));

        let full_local = Config {
            stt_local: true,
            cleanup_engine: "local".into(),
            ..Config::default()
        };
        assert!(should_warm_stt(&full_local));
        assert!(should_warm_llm(&full_local));
    }

    #[test]
    fn a_finished_sentence_flushes_however_it_arrives() {
        for ending in [".", "!", "?", "\n", ".\n", ".\r\n", ". "] {
            assert!(should_flush(&format!("All done{ending}")), "{ending:?}");
        }
    }

    #[test]
    fn a_short_mid_sentence_chunk_waits_for_more() {
        for chunk in ["and then", "", "and then I said the thing to", "1.5, 2.5"] {
            assert!(!should_flush(chunk), "{chunk:?}");
        }
    }

    #[test]
    fn the_threshold_is_inclusive_at_exactly_the_boundary() {
        let just_under = "w".repeat(FLUSH_CHARS - 1);
        let exactly = "w".repeat(FLUSH_CHARS);
        assert!(!should_flush(&just_under));
        assert!(should_flush(&exactly));
        assert!(should_flush(&"w".repeat(FLUSH_CHARS + 1)));
    }
}
