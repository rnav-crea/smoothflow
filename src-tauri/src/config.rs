use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(test)]
fn config_dir() -> PathBuf {
    // ponytail: per-thread subdir so parallel tests don't collide (a shared
    // dir breaks save_and_load_roundtrip vs the delete/corrupt tests); one
    // test always runs on one thread, so Config::path() stays consistent
    let mut p = std::env::temp_dir();
    p.push(format!("smoothflow-test-{:?}", std::thread::current().id()));
    let _ = std::fs::create_dir_all(&p);
    p
}

// --- OS credential vault (VULN-003) ---
// The API key lives in the OS credential manager, never in smoothflow.json.
// Test builds use in-memory shims so tests stay deterministic and never
// write to a real Windows vault.

#[cfg(not(test))]
pub fn store_secret(key: &str) -> Result<(), String> {
    keyring::Entry::new("SmoothFlow", "api_key")
        .and_then(|entry| entry.set_password(key))
        .map_err(|e| format!("[CFG-001] Could not save API key to Windows Credential Manager. ({e})"))
}

#[cfg(not(test))]
pub fn load_secret() -> Option<String> {
    keyring::Entry::new("SmoothFlow", "api_key")
        .and_then(|entry| entry.get_password())
        .ok()
}

#[cfg(test)]
pub fn store_secret(key: &str) -> Result<(), String> {
    TEST_VAULT.with(|v| v.replace(Some(key.to_string())));
    Ok(())
}

#[cfg(test)]
pub fn load_secret() -> Option<String> {
    TEST_VAULT.with(|v| v.borrow().clone())
}

// ponytail: thread-local test vault — cargo runs tests in parallel threads,
// so a shared static would leak keys between tests; TLS isolates each test.
#[cfg(test)]
thread_local! {
    static TEST_VAULT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(test))]
fn config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let mut p = PathBuf::from(appdata);
        p.push("SmoothFlow");
        let _ = std::fs::create_dir_all(&p);
        p
    } else {
        // ponytail: fallback to home dir if APPDATA not set (macOS / Linux)
        std::env::home_dir().unwrap_or_default().join("SmoothFlow")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub cleanup_model: String,
    // old smoothflow.json files predate this field — load them with the new
    // default instead of failing the whole parse
    #[serde(default = "default_cleanup_fallback_model")]
    pub cleanup_fallback_model: String,
    pub auto_punctuation: bool,
    pub remove_fillers: bool,
    pub auto_paste: bool,
    pub launch_on_startup: bool,
    pub dictionary: Vec<String>,
    pub hotkey: String,
    pub overlay_position: String,
    // free-text subject context that biases transcription vocabulary
    #[serde(default)]
    pub dictation_context: String,
}

fn default_cleanup_fallback_model() -> String {
    "qwen/qwen3.6-27b".into()
}

/// Per-OS default hotkey. macOS defaults to the bare Fn key, which is handled
/// by a CGEventTap (the global-shortcut crate can't map Fn); everything else
/// uses Alt+Space.
fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") { "Fn" } else { "Alt+Space" }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.groq.com/openai/v1".into(),
            api_key: String::new(),
            model: "whisper-large-v3".into(),
            cleanup_model: "openai/gpt-oss-20b".into(),
            cleanup_fallback_model: "qwen/qwen3.6-27b".into(),
            auto_punctuation: true,
            remove_fillers: true,
            auto_paste: true,
            launch_on_startup: false,
            dictionary: Vec::new(),
            hotkey: default_hotkey().into(),
            overlay_position: "bottom".into(),
            dictation_context: String::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let mut p = config_dir();
        p.push("smoothflow.json");
        p
    }

    pub fn load() -> Self {
        let path = Self::path();
        let mut config: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // VULN-003 migration: legacy plaintext key in the JSON moves into the
        // vault, then the file is rewritten without it. If the vault write
        // fails, keep the key in memory and leave the file untouched rather
        // than silently dropping the user's key.
        if !config.api_key.is_empty() && store_secret(&config.api_key).is_ok() {
            config.save();
        }
        // Always restore the key from the vault into memory (also on app
        // restarts after migration, when the JSON api_key is already empty).
        if let Some(key) = load_secret() {
            config.api_key = key;
        }
        config
    }

    pub fn save(&self) {
        // VULN-003: the key lives in the vault, never in the JSON on disk.
        let mut stripped = self.clone();
        stripped.api_key = String::new();
        if let Ok(s) = serde_json::to_string_pretty(&stripped) {
            let _ = std::fs::write(Self::path(), s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_values() {
        let c = Config::default();
        assert_eq!(c.api_base_url, "https://api.groq.com/openai/v1");
        assert_eq!(c.model, "whisper-large-v3");
        assert_eq!(c.cleanup_model, "openai/gpt-oss-20b");
        assert_eq!(c.cleanup_fallback_model, "qwen/qwen3.6-27b");
        assert!(c.auto_punctuation);
        assert!(c.remove_fillers);
        assert!(c.auto_paste);
        assert!(!c.launch_on_startup);
        assert!(c.api_key.is_empty());
        assert!(c.dictionary.is_empty());
        assert_eq!(c.hotkey, default_hotkey());
        assert_eq!(c.overlay_position, "bottom");
        assert!(c.dictation_context.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let c = Config {
            api_base_url: "https://example.com/api".into(),
            api_key: "sk-test".into(),
            model: "whisper-1".into(),
            cleanup_model: "gpt-4o-mini".into(),
            cleanup_fallback_model: String::new(),
            auto_punctuation: false,
            remove_fillers: false,
            auto_paste: false,
            launch_on_startup: true,
            dictionary: vec!["foo".into(), "bar".into()],
            hotkey: "Alt+Shift+T".into(),
            overlay_position: "top".into(),
            dictation_context: "quarterly review".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.api_base_url, "https://example.com/api");
        assert_eq!(back.api_key, "sk-test");
        assert!(!back.auto_punctuation);
        assert_eq!(back.dictionary, vec!["foo", "bar"]);
        assert_eq!(back.hotkey, "Alt+Shift+T");
        assert_eq!(back.overlay_position, "top");
    }

    #[test]
    fn old_json_without_fallback_field_loads_with_default() {
        // Config files written before cleanup_fallback_model existed must
        // deserialize with the new default rather than failing the whole parse.
        let json = r#"{"api_base_url":"https://api.groq.com/openai/v1","api_key":"","model":"whisper-large-v3-turbo","cleanup_model":"openai/gpt-oss-20b","auto_punctuation":true,"remove_fillers":true,"auto_paste":true,"launch_on_startup":false,"dictionary":[],"hotkey":"Meta+Space","overlay_position":"bottom"}"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.cleanup_fallback_model, "qwen/qwen3.6-27b");
        assert_eq!(c.cleanup_model, "openai/gpt-oss-20b");
    }

    #[test]
    fn load_missing_file_returns_default() {        let path = Config::path();
        let backup = std::fs::read_to_string(&path).ok();
        let _ = std::fs::remove_file(&path);
        let c = Config::load();
        assert_eq!(c.api_base_url, Config::default().api_base_url);
        if let Some(data) = backup {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(data.as_bytes()).unwrap();
        }
    }

    #[test]
    fn load_corrupt_file_returns_default() {
        let path = Config::path();
        let backup = std::fs::read_to_string(&path).ok();
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"not valid json{").unwrap();
        }
        let c = Config::load();
        assert_eq!(c.api_base_url, Config::default().api_base_url);
        if let Some(data) = backup {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(data.as_bytes()).unwrap();
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let c = Config {
            api_base_url: "https://custom.example.com".into(),
            api_key: "key-roundtrip".into(),
            model: "whisper-custom".into(),
            cleanup_model: String::new(),
            cleanup_fallback_model: "qwen-fallback".into(),
            auto_punctuation: false,
            remove_fillers: true,
            auto_paste: true,
            launch_on_startup: false,
            dictionary: vec!["term".into()],
            hotkey: "Ctrl+Shift+Space".into(),
            overlay_position: "bottom".into(),
            dictation_context: "my notes".into(),
        };
        c.save();
        let loaded = Config::load();
        assert_eq!(loaded.api_base_url, "https://custom.example.com");
        assert!(loaded.api_key.is_empty(), "api_key must not persist to disk (VULN-003)");
        assert_eq!(loaded.model, "whisper-custom");
        assert_eq!(loaded.cleanup_fallback_model, "qwen-fallback");
        assert!(!loaded.auto_punctuation);
        assert!(loaded.remove_fillers);
        assert!(loaded.auto_paste);
        assert_eq!(loaded.dictionary, vec!["term"]);
        assert_eq!(loaded.hotkey, "Ctrl+Shift+Space");
        assert_eq!(loaded.overlay_position, "bottom");
        assert_eq!(loaded.dictation_context, "my notes");
    }

    #[test]
    fn vault_key_is_restored_on_load() {
        // Regression: after migration the JSON has an empty api_key; load()
        // must still pull the key back out of the vault on every app start.
        assert!(store_secret("vault-key").is_ok());
        let loaded = Config::load();
        assert_eq!(loaded.api_key, "vault-key");
    }
}
