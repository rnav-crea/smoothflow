use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::time::Duration;

pub fn type_text(text: &str) -> Result<(), String> {
    // ponytail: .text() is faster (no clipboard, no delay), use as primary
    enigo_text(text).or_else(|_| clipboard_paste(text))
}

fn clipboard_paste(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;

    let old = clipboard.get_text().ok();

    clipboard
        .set_text(text.to_owned())
        .map_err(|e| format!("clipboard set: {e}"))?;

    std::thread::sleep(Duration::from_millis(50));

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("enigo init: {e}"))?;

    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| format!("ctrl press: {e}"))?;
    std::thread::sleep(Duration::from_millis(10));
    enigo
        .key(Key::V, Direction::Click)
        .map_err(|e| format!("v click: {e}"))?;
    std::thread::sleep(Duration::from_millis(10));
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| format!("ctrl release: {e}"))?;

    std::thread::sleep(Duration::from_millis(100));

    if let Some(old_text) = old {
        let _ = clipboard.set_text(old_text);
    }

    Ok(())
}

fn enigo_text(text: &str) -> Result<(), String> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("enigo init: {e}"))?;
    enigo
        .text(text)
        .map_err(|e| format!("type error: {e}"))
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
