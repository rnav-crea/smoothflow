use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub cleanup_model: String,
    pub auto_punctuation: bool,
    pub remove_fillers: bool,
    pub auto_paste: bool,
    pub launch_on_startup: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.groq.com/openai/v1".into(),
            api_key: String::new(),
            model: "whisper-large-v3".into(),
            cleanup_model: "llama-3.1-8b-instant".into(),
            auto_punctuation: true,
            remove_fillers: true,
            auto_paste: true,
            launch_on_startup: false,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        p.push("smoothflow.json");
        p
    }

    pub fn load() -> Self {
        let path = Self::path();
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
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
        assert_eq!(c.cleanup_model, "llama-3.1-8b-instant");
        assert!(c.auto_punctuation);
        assert!(c.remove_fillers);
        assert!(c.auto_paste);
        assert!(!c.launch_on_startup);
        assert!(c.api_key.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let c = Config {
            api_base_url: "https://example.com/api".into(),
            api_key: "sk-test".into(),
            model: "whisper-1".into(),
            cleanup_model: "gpt-4o-mini".into(),
            auto_punctuation: false,
            remove_fillers: false,
            auto_paste: false,
            launch_on_startup: true,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.api_base_url, "https://example.com/api");
        assert_eq!(back.api_key, "sk-test");
        assert!(!back.auto_punctuation);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let path = Config::path();
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
            auto_punctuation: false,
            remove_fillers: true,
            auto_paste: true,
            launch_on_startup: false,
        };
        c.save();
        let loaded = Config::load();
        assert_eq!(loaded.api_base_url, "https://custom.example.com");
        assert_eq!(loaded.api_key, "key-roundtrip");
        assert_eq!(loaded.model, "whisper-custom");
        assert!(!loaded.auto_punctuation);
        assert!(loaded.remove_fillers);
        assert!(loaded.auto_paste);
    }
}
