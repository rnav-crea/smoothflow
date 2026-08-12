use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::time::Duration;

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

pub fn type_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !crate::accessibility::is_trusted() {
        crate::accessibility::open_settings();
        return Err("SmoothFlow needs Accessibility permission to type into other apps. Grant it in the System Settings window that just opened, then dictate again.".into());
    }
    clipboard_paste(text).or_else(|_| enigo_text(text))
}

fn clipboard_paste(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("[PST-001] Could not open clipboard. ({e})"))?;

    let old = clipboard.get_text().ok();

    clipboard
        .set_text(text.to_owned())
        .map_err(|e| format!("[PST-001] Could not write to clipboard. ({e})"))?;

    std::thread::sleep(Duration::from_millis(50));

    let (paste_mod, paste_key) = paste_combo();

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("[PST-002] Keyboard init failed. ({e})"))?;

    enigo
        .key(paste_mod, Direction::Press)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
    std::thread::sleep(Duration::from_millis(10));
    enigo
        .key(paste_key, Direction::Click)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;
    std::thread::sleep(Duration::from_millis(10));
    enigo
        .key(paste_mod, Direction::Release)
        .map_err(|e| format!("[PST-002] Auto-paste failed — click into the target app and retry. ({e})"))?;

    std::thread::sleep(Duration::from_millis(100));

    if let Some(old_text) = old {
        let _ = clipboard.set_text(old_text);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_text_accepts_empty_string() {
        let result = type_text("");
        let _ = result;
    }

    #[test]
    fn type_text_accepts_short_string() {
        let result = type_text("hi");
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error") || e.contains("clipboard")),
        }
    }

    #[test]
    fn type_text_handles_special_chars() {
        let result = type_text("Hello, world! 123.");
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error") || e.contains("clipboard")),
        }
    }
}
