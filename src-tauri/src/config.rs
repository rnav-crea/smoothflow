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
        .map_err(|e| {
            format!("[CFG-001] Could not save API key to Windows Credential Manager. ({e})")
        })
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
    /// How the hotkey behaves: `"hold"` = press to start, release to stop
    /// (push-to-talk, today's behaviour). `"toggle"` = press once to start,
    /// press again to stop. Anything else normalises to `"hold"` — the
    /// behaviour every user has today.
    #[serde(default = "default_hotkey_mode")]
    pub hotkey_mode: String,
    pub overlay_position: String,
    /// Legacy single switch, kept only for backward compatibility and
    /// migration of old `smoothflow.json` files. It is serialized as-is but
    /// **never read for a decision** — `stt_local` and `cleanup_engine` are the
    /// two independent legs that replaced it.
    #[serde(default)]
    pub offline_mode: bool,
    /// Where speech-to-text runs: `true` = the local Parakeet helper on this
    /// machine, `false` = the configured cloud endpoint.
    #[serde(default)]
    pub stt_local: bool,
    /// Whether and where AI cleanup runs: `"off"` (deterministic rules only,
    /// the default), `"local"` (local llama-server) or `"cloud"` (the existing
    /// cloud cleanup endpoint). Anything else normalises to `"off"`.
    #[serde(default = "default_cleanup_engine")]
    pub cleanup_engine: String,
    #[serde(default = "default_stt_version")]
    pub stt_version: String,
    #[serde(default = "default_llm_model_size")]
    pub llm_model_size: String,
    #[serde(default)]
    pub models_dir: String,
    /// Absolute path to a user-supplied .gguf, used instead of the catalog
    /// download. Empty = download the catalog model for `llm_model_size`.
    #[serde(default)]
    pub custom_llm_path: String,
    /// Absolute path to a user-supplied speech model folder. Must contain a
    /// Parakeet bundle (encoder/decoder/joiner .onnx + tokens.txt). Empty = use
    /// the catalog model for `stt_version`.
    #[serde(default)]
    pub custom_stt_dir: String,
    #[serde(default)]
    pub auto_download_models: bool,
    #[serde(default = "default_casual_fast_path_words")]
    pub casual_fast_path_words: usize,
    #[serde(default)]
    pub keep_audio_clips: bool,
}

fn default_cleanup_fallback_model() -> String {
    "qwen/qwen3.6-27b".into()
}

fn default_stt_version() -> String {
    "v2".into()
}

fn default_llm_model_size() -> String {
    "4b".into()
}

/// Words at or below which local cleanup takes the deterministic rules instead
/// of the LLM, per `postprocess::fast_path_applies`.
///
/// 4 was far too low to matter: a 39-word dictation spent 6 s of CPU decode to
/// change 2 words, and it is the decode — not speech-to-text — that decides how
/// long a dictation feels. At 40 the same utterance skips inference entirely.
/// The trade-off is explicit: medium-length dictations lose the model's cleanup
/// and keep only the rules.
fn default_casual_fast_path_words() -> usize {
    40
}

fn default_cleanup_engine() -> String {
    "off".into()
}

/// The only three legal `cleanup_engine` values.
pub const CLEANUP_ENGINES: [&str; 3] = ["off", "local", "cloud"];

/// Fallback engine when a value is not one of `CLEANUP_ENGINES`. Never guesses
/// `"cloud"`: defaulting to a network call would violate the offline contract,
/// so an unrecognised value means "no model at all".
pub const CLEANUP_ENGINE_FALLBACK: &str = "off";

/// Normalise a `cleanup_engine` value, so a config file (or a UI payload) can
/// never smuggle in a fourth value. Never panics.
pub fn normalize_cleanup_engine(engine: &str) -> String {
    match engine {
        "off" => "off".to_string(),
        "local" => "local".to_string(),
        "cloud" => "cloud".to_string(),
        _ => CLEANUP_ENGINE_FALLBACK.to_string(),
    }
}

/// One-time migration of the retired `offline_mode` switch into the two legs.
///
/// Only applies to objects that have neither new key — a file that already
/// carries `stt_local` and/or `cleanup_engine` is taken verbatim, no matter what
/// the legacy field says.
///
/// `offline_mode: true`  → local speech + local cleanup.
/// `offline_mode: false` → cloud speech + cloud cleanup, so today's cloud users
///   do not silently lose AI cleanup on upgrade. A file missing the field
///   entirely was always a cloud file for the same reason.
fn migrate_split_switch(value: &mut serde_json::Value) {
    // A non-object cannot hold either shape; leave it to serde to fall back to
    // the defaults exactly as before.
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.contains_key("stt_local") || object.contains_key("cleanup_engine") {
        return;
    }
    let offline = object
        .get("offline_mode")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    object.insert("stt_local".into(), serde_json::Value::Bool(offline));
    object.insert(
        "cleanup_engine".into(),
        serde_json::Value::String(if offline { "local" } else { "cloud" }.to_string()),
    );
}

/// Raise a stored threshold of 4 to the current default.
///
/// 4 was only ever the shipped default: no setting exposes this field, so a
/// config file holding 4 came from the default, not from a choice. Without this
/// every existing install — the ones that actually feel the 6 s decode — would
/// keep 4 forever, because `Config::save` writes the value back out.
fn migrate_casual_fast_path(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object
        .get("casual_fast_path_words")
        .and_then(serde_json::Value::as_u64)
        == Some(4)
    {
        object.insert(
            "casual_fast_path_words".into(),
            serde_json::json!(default_casual_fast_path_words()),
        );
    }
}

/// Ctrl+Space everywhere by default — macOS reserves Fn/Globe for the Emoji &
/// Symbols picker, making it a bad default. Bare "Fn" remains user-selectable
/// and routes through the CGEventTap backend in hotkey.rs.
fn default_hotkey() -> &'static str {
    "Ctrl+Space"
}

fn default_hotkey_mode() -> String {
    "hold".into()
}

/// Normalise a `hotkey_mode` value. Unknown values fall back to `"hold"`,
/// which is the behaviour the app shipped with — never switching an existing
/// user's hotkey style silently.
pub fn normalize_hotkey_mode(mode: &str) -> String {
    match mode {
        "toggle" => "toggle".to_string(),
        _ => "hold".to_string(),
    }
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
            hotkey_mode: default_hotkey_mode(),
            overlay_position: "bottom".into(),
            offline_mode: false,
            stt_local: false,
            cleanup_engine: default_cleanup_engine(),
            stt_version: default_stt_version(),
            llm_model_size: default_llm_model_size(),
            models_dir: String::new(),
            custom_llm_path: String::new(),
            custom_stt_dir: String::new(),
            auto_download_models: false,
            casual_fast_path_words: default_casual_fast_path_words(),
            keep_audio_clips: false,
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
        let mut config: Self = match std::fs::read_to_string(&path) {
            Ok(raw) => Self::from_json(&raw),
            Err(_) => Self::default(),
        };

        // VULN-003 migration: legacy plaintext key in the JSON moves into the
        // vault, then the file is rewritten without it. If the vault write
        // fails, keep the key in memory and leave the file untouched rather
        // than silently dropping the user's key.
        if !config.api_key.is_empty() && store_secret(&config.api_key).is_ok() {
            let _ = config.save();
        }
        // Always restore the key from the vault into memory (also on app
        // restarts after migration, when the JSON api_key is already empty).
        if let Some(key) = load_secret() {
            config.api_key = key;
        }
        config
    }

    /// Parse a raw `smoothflow.json` body, migrating the retired `offline_mode`
    /// switch into the two independent legs. Unreadable input keeps the old
    /// behaviour: the defaults.
    fn from_json(raw: &str) -> Self {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return Self::default();
        };
        migrate_split_switch(&mut value);
        migrate_casual_fast_path(&mut value);
        let mut config: Self = serde_json::from_value(value).unwrap_or_default();
        config.normalize();
        config
    }

    /// Force `cleanup_engine` into its legal set. Called after deserialise and
    /// after `update_config`, so neither a hand-edited file nor a stale UI can
    /// produce a fourth value.
    pub fn normalize(&mut self) {
        self.cleanup_engine = normalize_cleanup_engine(&self.cleanup_engine);
        self.hotkey_mode = normalize_hotkey_mode(&self.hotkey_mode);
    }

    /// Persist the config. The key lives in the vault, never in the JSON on
    /// disk. The write goes through a temp file and a rename so the async scan
    /// worker can never observe a half-written file, and the caller learns
    /// whether the write actually landed.
    pub fn save(&self) -> Result<(), String> {
        let mut stripped = self.clone();
        stripped.api_key = String::new();
        let json = serde_json::to_string_pretty(&stripped)
            .map_err(|e| format!("[CFG-007] Could not serialise the config: {e}"))?;
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("[CFG-007] Could not create {}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .map_err(|e| format!("[CFG-007] Could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("[CFG-007] Could not replace {}: {e}", path.display()))?;
        Ok(())
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
        assert!(!c.offline_mode);
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "off");
        assert_eq!(c.stt_version, "v2");
        assert_eq!(c.llm_model_size, "4b");
        assert!(c.models_dir.is_empty());
        assert!(c.custom_llm_path.is_empty());
        assert!(c.custom_stt_dir.is_empty());
        assert!(!c.auto_download_models);
        assert_eq!(c.casual_fast_path_words, default_casual_fast_path_words());
        assert!(!c.keep_audio_clips);
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
            hotkey_mode: "toggle".into(),
            overlay_position: "top".into(),
            offline_mode: true,
            stt_local: true,
            cleanup_engine: "local".into(),
            stt_version: "v3".into(),
            llm_model_size: "2b".into(),
            models_dir: "D:/models".into(),
            custom_llm_path: "C:/my-model.gguf".into(),
            custom_stt_dir: "D:/parakeet-bundle".into(),
            auto_download_models: true,
            casual_fast_path_words: 7,
            keep_audio_clips: true,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.api_base_url, "https://example.com/api");
        assert_eq!(back.api_key, "sk-test");
        assert!(!back.auto_punctuation);
        assert_eq!(back.dictionary, vec!["foo", "bar"]);
        assert_eq!(back.hotkey, "Alt+Shift+T");
        assert_eq!(back.hotkey_mode, "toggle");
        assert_eq!(back.overlay_position, "top");
        assert!(back.offline_mode);
        assert!(back.stt_local);
        assert_eq!(back.cleanup_engine, "local");
        assert_eq!(back.stt_version, "v3");
        assert_eq!(back.llm_model_size, "2b");
        assert_eq!(back.models_dir, "D:/models");
        assert_eq!(back.custom_llm_path, "C:/my-model.gguf");
        assert_eq!(back.custom_stt_dir, "D:/parakeet-bundle");
        assert!(back.auto_download_models);
        assert_eq!(back.casual_fast_path_words, 7);
        assert!(back.keep_audio_clips);
    }

    #[test]
    fn old_json_without_fallback_field_loads_with_default() {
        // Config files written before cleanup_fallback_model existed must
        // deserialize with the new default rather than failing the whole parse.
        let json = r#"{"api_base_url":"https://api.groq.com/openai/v1","api_key":"","model":"whisper-large-v3-turbo","cleanup_model":"openai/gpt-oss-20b","auto_punctuation":true,"remove_fillers":true,"auto_paste":true,"launch_on_startup":false,"dictionary":[],"hotkey":"Meta+Space","overlay_position":"bottom"}"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.cleanup_fallback_model, "qwen/qwen3.6-27b");
        assert_eq!(c.cleanup_model, "openai/gpt-oss-20b");
        // fields added in Phase 1 load with their defaults from old files
        assert_eq!(c.casual_fast_path_words, default_casual_fast_path_words());
        assert!(!c.keep_audio_clips);
        assert!(!c.offline_mode);
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "off");
        assert_eq!(c.stt_version, "v2");
        assert_eq!(c.llm_model_size, "4b");
        assert!(c.models_dir.is_empty());
        assert!(c.custom_llm_path.is_empty());
        assert!(c.custom_stt_dir.is_empty());
        assert!(!c.auto_download_models);
    }

    /// The migration input cases, exercised without touching the real
    /// `smoothflow.json` on disk.
    fn parse(json: &str) -> Config {
        Config::from_json(json)
    }

    const LEGACY_BASE: &str = r#""api_base_url":"https://api.groq.com/openai/v1","api_key":"","model":"whisper-large-v3","cleanup_model":"openai/gpt-oss-20b","auto_punctuation":true,"remove_fillers":true,"auto_paste":true,"launch_on_startup":false,"dictionary":[],"hotkey":"Ctrl+Space","overlay_position":"bottom""#;

    #[test]
    fn old_json_with_offline_mode_true_migrates_to_both_legs_on() {
        let c = parse(&format!(r#"{{{LEGACY_BASE},"offline_mode":true}}"#));
        assert!(c.stt_local);
        assert_eq!(c.cleanup_engine, "local");
        // The legacy field is preserved verbatim, just never read again.
        assert!(c.offline_mode);
    }

    #[test]
    fn old_json_with_offline_mode_false_keeps_cloud_cleanup() {
        // Regression guard: today's cloud users must not lose AI cleanup on
        // upgrade — the naive "default is off" would silently degrade them.
        let c = parse(&format!(r#"{{{LEGACY_BASE},"offline_mode":false}}"#));
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "cloud");
    }

    #[test]
    fn a_legacy_file_without_the_switch_counts_as_cloud() {
        let c = parse(&format!("{{{LEGACY_BASE}}}"));
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "cloud");
    }

    #[test]
    fn new_json_wins_over_the_retired_switch() {
        let c = parse(&format!(
            r#"{{{LEGACY_BASE},"offline_mode":true,"stt_local":false,"cleanup_engine":"cloud"}}"#
        ));
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "cloud");
        // No migration ran, but the legacy field still round-trips untouched.
        assert!(c.offline_mode);

        // Either new key alone also disables migration entirely.
        let only_stt = parse(&format!(
            r#"{{{LEGACY_BASE},"offline_mode":true,"stt_local":false}}"#
        ));
        assert!(!only_stt.stt_local);
        assert_eq!(only_stt.cleanup_engine, "off");
    }

    #[test]
    fn the_old_fast_path_default_is_raised_but_a_chosen_value_is_kept() {
        // 4 was only ever the shipped default, so a stored 4 is not a choice.
        let raised = parse(&format!(
            r#"{{{LEGACY_BASE},"casual_fast_path_words":4}}"#
        ));
        assert_eq!(raised.casual_fast_path_words, default_casual_fast_path_words());
        assert!(default_casual_fast_path_words() > 4);

        // Anything else was set deliberately and must survive a load.
        let kept = parse(&format!(
            r#"{{{LEGACY_BASE},"casual_fast_path_words":9}}"#
        ));
        assert_eq!(kept.casual_fast_path_words, 9);

        // A file written before the field existed gets the default.
        let missing = parse(&format!("{{{LEGACY_BASE}}}"));
        assert_eq!(
            missing.casual_fast_path_words,
            default_casual_fast_path_words()
        );
    }

    #[test]
    fn an_unknown_cleanup_engine_normalises_to_off() {
        for value in ["", "offf", "Local", "gpu", "openai", "  "] {
            let c = parse(&format!(
                r#"{{{LEGACY_BASE},"cleanup_engine":"{value}"}}"#
            ));
            assert_eq!(c.cleanup_engine, "off", "value {value:?}");
        }
        // The legal values survive untouched.
        for value in CLEANUP_ENGINES {
            let c = parse(&format!(
                r#"{{{LEGACY_BASE},"cleanup_engine":"{value}"}}"#
            ));
            assert_eq!(c.cleanup_engine, value);
        }
    }

    #[test]
    fn normalize_is_idempotent_and_never_guesses_cloud() {
        for value in ["", "cloudy", "OFF", "0"] {
            assert_eq!(normalize_cleanup_engine(value), "off");
        }
        for value in CLEANUP_ENGINES {
            assert_eq!(normalize_cleanup_engine(value), value);
        }
    }

    #[test]
    fn unreadable_json_still_yields_the_defaults() {
        let c = parse("not valid json{");
        assert_eq!(c.api_base_url, Config::default().api_base_url);
        assert!(!c.stt_local);
        assert_eq!(c.cleanup_engine, "off");
        // A JSON document of the wrong shape must not panic either.
        assert_eq!(parse("[1,2,3]").cleanup_engine, "off");
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
            cleanup_fallback_model: "qwen-fallback".into(),
            auto_punctuation: false,
            remove_fillers: true,
            auto_paste: true,
            launch_on_startup: false,
            dictionary: vec!["term".into()],
            hotkey: "Ctrl+Shift+Space".into(),
            hotkey_mode: "hold".into(),
            overlay_position: "bottom".into(),
            offline_mode: true,
            stt_local: false,
            cleanup_engine: "cloud".into(),
            stt_version: "v3".into(),
            llm_model_size: "9b".into(),
            models_dir: "C:/SmoothFlow/models".into(),
            custom_llm_path: "C:/ollama/mistral.gguf".into(),
            custom_stt_dir: "D:/parakeet-custom".into(),
            auto_download_models: true,
            casual_fast_path_words: 5,
            keep_audio_clips: true,
        };
        c.save().expect("save config");
        let loaded = Config::load();
        assert_eq!(loaded.api_base_url, "https://custom.example.com");
        assert!(
            loaded.api_key.is_empty(),
            "api_key must not persist to disk (VULN-003)"
        );
        assert_eq!(loaded.model, "whisper-custom");
        assert_eq!(loaded.cleanup_fallback_model, "qwen-fallback");
        assert!(!loaded.auto_punctuation);
        assert!(loaded.remove_fillers);
        assert!(loaded.auto_paste);
        assert_eq!(loaded.dictionary, vec!["term"]);
        assert_eq!(loaded.hotkey, "Ctrl+Shift+Space");
        assert_eq!(loaded.overlay_position, "bottom");
        assert!(loaded.offline_mode);
        assert!(!loaded.stt_local);
        assert_eq!(loaded.cleanup_engine, "cloud");
        assert_eq!(loaded.stt_version, "v3");
        assert_eq!(loaded.llm_model_size, "9b");
        assert_eq!(loaded.models_dir, "C:/SmoothFlow/models");
        assert_eq!(loaded.custom_llm_path, "C:/ollama/mistral.gguf");
        assert_eq!(loaded.custom_stt_dir, "D:/parakeet-custom");
        assert!(loaded.auto_download_models);
        assert_eq!(loaded.casual_fast_path_words, 5);
        assert!(loaded.keep_audio_clips);
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
