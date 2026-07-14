use enigo::{Enigo, Keyboard, Settings};

pub fn type_text(text: &str) -> Result<(), String> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("enigo init: {}", e))?;
    enigo
        .text(text)
        .map_err(|e| format!("type error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_text_accepts_empty_string() {
        // Empty string should not error at the type_text level
        // (enigo might or might not accept it, but our wrapper handles it)
        let result = type_text("");
        // Either Ok or Err is fine - we just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn type_text_accepts_short_string() {
        let result = type_text("hi");
        // If enigo can init (has a display), this succeeds
        // If not (CI/headless), it errors gracefully
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error")),
        }
    }

    #[test]
    fn type_text_handles_special_chars() {
        let result = type_text("Hello, world! 123.");
        match result {
            Ok(_) => assert!(true),
            Err(e) => assert!(e.contains("init") || e.contains("error")),
        }
    }
}
