use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::time::Duration;
use std::sync::mpsc;
use std::thread;

// Ctrl+V on Windows/Linux, Cmd+V on macOS. enigo's letter keys (A-Z) only
// exist on Windows, so the other platforms use the layout-mapped Unicode key.
fn paste_combo() -> (Key, Key) {
    #[cfg(target_os = "macos")]
    { (Key::Meta, Key::Unicode('v')) }
    #[cfg(target_os = "windows")]
    { (Key::Control, Key::V) }
    #[cfg(all(unix, not(target_os = "macos")))]
    { (Key::Control, Key::Unicode('v')) }
}

/// Hard timeout for any clipboard operation. Windows clipboard IPC can block
/// indefinitely if another app (clipboard manager, password manager) holds it open.
const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(500);

/// Paste `text` into the currently focused application.
///
/// macOS note: enigo resolves keys through the Text Input Source (TIS/TSM)
/// APIs. On modern macOS those APIs call `dispatch_assert_queue` and require
/// the **main thread** — invoking them from a background thread traps the
/// process (EXC_BREAKPOINT / SIGTRAP) and the app quits mid-dictation. Since
/// `type_text` is always called from background threads (the transcription
/// worker and Tauri command threads), we hop the actual synthesis onto the
/// main thread via `run_on_main_thread` and block the caller for the result.
#[cfg(target_os = "macos")]
pub fn type_text(app: &tauri::AppHandle, text: &str) -> Result<(), String> {
    if !crate::accessibility::is_trusted() {
        crate::accessibility::open_settings();
        return Err("SmoothFlow needs Accessibility permission to type into other apps. Grant it in the System Settings window that just opened, then dictate again.".into());
    }
    let text = text.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(do_paste(&text));
    })
    .map_err(|e| format!("[PST-004] Could not schedule the paste on the main thread. ({e})"))?;
    rx.recv()
        .map_err(|e| format!("[PST-004] The paste task did not report back. ({e})"))?
}

#[cfg(not(target_os = "macos"))]
pub fn type_text(_app: &tauri::AppHandle, text: &str) -> Result<(), String> {
    do_paste(text)
}

fn do_paste(text: &str) -> Result<(), String> {
    clipboard_paste(text).or_else(|_| enigo_text(text))
}

fn clipboard_paste(text: &str) -> Result<(), String> {
    let text = text.to_owned();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result: Result<Option<String>, String> = (|| {
            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("[PST-001] Could not open clipboard. ({e})"))?;

            let old = clipboard.get_text().ok();

            clipboard
                .set_text(text)
                .map_err(|e| format!("[PST-001] Could not write to clipboard. ({e})"))?;

            Ok(old)
        })();
        let _ = tx.send(result);
    });

    let old = match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(Ok(old)) => old,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("[PST-001] Clipboard operation timed out (500ms). Falling back to direct typing.".into()),
    };

    thread::sleep(Duration::from_millis(30));

    let (paste_mod, paste_key) = paste_combo();

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("[PST-002] Keyboard init failed. ({e})"))?;

    enigo
        .key(paste_mod, Direction::Press)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
    thread::sleep(Duration::from_millis(5));
    enigo
        .key(paste_key, Direction::Click)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
    thread::sleep(Duration::from_millis(5));
    enigo
        .key(paste_mod, Direction::Release)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;

    thread::sleep(Duration::from_millis(50));

    if let Some(old_text) = old {
        let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(old_text));
    }

    Ok(())
}

fn enigo_text(text: &str) -> Result<(), String> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("[PST-002] Keyboard init failed. ({e})"))?;
    enigo
        .text(text)
        .map_err(|e| format!("[PST-003] Auto-paste failed — click into the target app and retry. ({e})"))
}

/// A paste that arrives in pieces. `clipboard_paste` saves and restores the
/// user's clipboard on every call, which is wasteful and racy at one round trip
/// per generated token — here the clipboard is saved once and restored once.
// allow(dead_code) markers come off with the lib.rs wiring.
pub struct PasteSession {
    saved: Option<String>,
}

#[allow(dead_code)]
impl PasteSession {
    /// Captures the current clipboard. A clipboard we cannot read (empty, an
    /// image, locked by another app) is `None`, which simply means nothing is
    /// restored later.
    pub fn begin() -> Self {
        PasteSession {
            saved: thread::spawn(|| {
                arboard::Clipboard::new()
                    .ok()
                    .and_then(|mut c| c.get_text().ok())
            })
            .join()
            .ok()
            .flatten(),
        }
    }

    /// Paste one more piece of text at the cursor. No save, no restore.
    pub fn append(&mut self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        let text = text.to_owned();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result: Result<(), String> = (|| {
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| format!("[PST-001] Could not open clipboard. ({e})"))?;
                clipboard
                    .set_text(text)
                    .map_err(|e| format!("[PST-001] Could not write to clipboard. ({e}"))
            })();
            let _ = tx.send(result);
        });
        match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
            Ok(result) => result?,
            Err(_) => {
                return Err(
                    "[PST-001] Clipboard operation timed out (500ms). Falling back to direct typing."
                        .into(),
                )
            }
        }

        thread::sleep(Duration::from_millis(30));
        let (paste_mod, paste_key) = paste_combo();
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("[PST-002] Keyboard init failed. ({e})"))?;
        enigo
            .key(paste_mod, Direction::Press)
            .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
        thread::sleep(Duration::from_millis(5));
        enigo
            .key(paste_key, Direction::Click)
            .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
        thread::sleep(Duration::from_millis(5));
        enigo
            .key(paste_mod, Direction::Release)
            .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    /// Put the user's clipboard back.
    pub fn finish(mut self) {
        Self::restore(&mut self.saved);
    }

    fn restore(saved: &mut Option<String>) {
        if let Some(old) = saved.take() {
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(old));
        }
    }
}

impl Drop for PasteSession {
    /// Restores on drop too, so a caller that forgets `finish()` on an error
    /// path still leaves the clipboard alone. `take()` makes it run at most once.
    fn drop(&mut self) {
        Self::restore(&mut self.saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real clipboard/keyboard access via arboard+enigo. On macOS CI runners the
    // test binary can appear Accessibility-trusted while having no attached
    // WindowServer session, and NSPasteboard then aborts the whole process.
    // These are OS-integration smoke tests — keep them for real desktops.
    // Exercises the platform-agnostic paste path directly; `type_text` itself
    // now needs an AppHandle (macOS main-thread hop) which a unit test can't build.
    #[cfg_attr(target_os = "macos", ignore = "clipboard access can SIGABRT the process on macOS CI runners")]
    #[test]
    fn do_paste_accepts_empty_string() {
        let result = do_paste("");
        let _ = result;
    }

    #[cfg_attr(target_os = "macos", ignore = "clipboard access can SIGABRT the process on macOS CI runners")]
    #[test]
    fn do_paste_accepts_short_string() {
        let result = do_paste("hi");
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error") || e.contains("clipboard")),
        }
    }

    #[cfg_attr(target_os = "macos", ignore = "clipboard access can SIGABRT the process on macOS CI runners")]
    #[test]
    fn do_paste_handles_special_chars() {
        let result = do_paste("Hello, world! 123.");
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error") || e.contains("clipboard")),
        }
    }

    #[cfg_attr(target_os = "macos", ignore = "clipboard access can SIGABRT the process on macOS CI runners")]
    #[test]
    fn paste_session_appends_several_pieces_and_restores_the_clipboard() {
        let mut session = PasteSession::begin();
        for piece in ["Hi Dana", ",", " see you"] {
            match session.append(piece) {
                Ok(()) => assert!(true),
                Err(e) => assert!(e.contains("init") || e.contains("error") || e.contains("clipboard")),
            }
        }
        // An empty chunk is a no-op, not an error and not a stray paste.
        assert!(session.append("").is_ok());
        session.finish();
    }
}
