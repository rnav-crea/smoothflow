//! Local (offline) LLM cleanup via a pinned `llama-server` child process.
//!
//! Same shape as `stt_local.rs`: one long-lived process behind a process-wide
//! mutex, spawned lazily on first use. A 2.55 GB model takes many seconds to
//! load, so spawning per request is not an option.
//!
//! Privacy: the server is bound to `127.0.0.1` only and started with
//! `--offline`, so nothing about the dictation can leave the machine. There is
//! no API key and no `Authorization` header — there is nothing to authenticate
//! against locally.
//!
//! Failure policy: every error returns `Err` and the session is torn down. This
//! module NEVER falls back to the cloud cleanup — the caller
//! (`postprocess_with_context`) already has the deterministic-rules fallback.

use crate::config::Config;
use crate::offline;
use crate::postprocess::{categorize_window, strip_reasoning_block, system_prompt, user_message};
use crate::sf_log;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Generous: a 2.55 GB model on a slow CPU is slow to load.
const READY_DEADLINE: Duration = Duration::from_secs(180);
const HEALTH_INTERVAL: Duration = Duration::from_millis(250);
/// Short, so a wedged poll loop cannot eat the whole deadline on one request.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
/// Long, for the actual completion — CPU inference is not fast.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// A live server plus everything needed to talk to and reap it.
struct Session {
    child: Child,
    port: u16,
    model: PathBuf,
}

impl Session {
    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Clean up a raw transcript on-device. Returns a user-readable `[LLM-00n]`
/// error when the server binary, the model, or the server itself is unhappy.
/// Never falls back to the network.
pub fn cleanup(text: &str, config: &Config, context: &str) -> Result<String, String> {
    let model = model_path(config)?;
    let server = server_path()?;

    let mut guard = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A different model means a different process: the old one is holding
    // several GB of RAM on a file we no longer need.
    if guard.as_ref().is_some_and(|s| s.model != model) {
        if let Some(mut old) = guard.take() {
            old.shutdown();
        }
    }
    if guard.is_none() {
        *guard = Some(spawn(&server, &model)?);
    }
    let outcome = match guard.as_ref() {
        Some(session) => request(session, text, config, context),
        // Unreachable: the session was just spawned, but never unwrap state.
        None => Err("[LLM-002] The local cleanup session is unavailable. Try again.".into()),
    };
    if outcome.is_err() {
        // Never cache a half-dead session: the next call must respawn cleanly.
        if let Some(mut dead) = guard.take() {
            dead.shutdown();
        }
    }
    outcome
}

/// Kill and reap the server. Safe with no session and safe to call twice —
/// an orphaned `llama-server` would hold the model in RAM after we exit.
pub fn shutdown() {
    let mut guard = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(mut session) = guard.take() {
        session.shutdown();
    }
}

fn spawn(server: &PathBuf, model: &PathBuf) -> Result<Session, String> {
    if !server.is_file() {
        return Err(format!(
            "[LLM-001] Local cleanup engine not found at {}. Reinstall SmoothFlow or enable cloud mode in Settings.",
            server.display()
        ));
    }
    if !model.is_file() {
        return Err(format!(
            "[LLM-001] Cleanup model not found at {}. Download the Qwen models in Settings, or enable cloud mode in Settings.",
            model.display()
        ));
    }

    let port = free_port()?;
    // ponytail: threads follow the machine instead of a config knob nobody
    // would get right; llama.cpp defaults to the same thing anyway.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut child = Command::new(server)
        .arg("-m")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-np")
        .arg("1")
        .arg("-c")
        .arg("4096")
        .arg("-t")
        .arg(threads.to_string())
        .arg("--offline")
        .arg("--no-webui")
        .arg("--jinja")
        .arg("--cache-prompt")
        .arg("--spec-type")
        .arg("ngram-mod")
        // The server logs heavily; a piped-but-unread pipe would deadlock.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("[LLM-001] Could not start the local cleanup engine at {}: {e}", server.display()))?;

    if let Err(err) = wait_ready(&mut child, port) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }
    sf_log!("LLM: local cleanup engine ready on port {port}");
    Ok(Session {
        child,
        port,
        model: model.clone(),
    })
}

/// One completion round trip. `Err` always means the caller must drop the
/// session. No transcript, prompt, or output is logged.
fn request(session: &Session, text: &str, config: &Config, context: &str) -> Result<String, String> {
    let body = build_body(
        system_prompt(config, categorize_window(context)),
        user_message(text, context),
    );
    // Separate from the health client: a 5 s timeout would cap every cleanup.
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("[LLM-002] Could not create the local cleanup client. ({e})"))?;
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", session.port);
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("[LLM-002] Local cleanup request failed. ({e})"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("[LLM-002] Local cleanup engine error {status}."));
    }
    let raw = resp
        .text()
        .map_err(|e| format!("[LLM-002] Could not read the local cleanup response. ({e})"))?;
    // Qwen3.5 can emit a thinking block; the shared stripper already handles it.
    Ok(strip_reasoning_block(&extract_content(&raw)?))
}

/// Poll `/health` until the model is loaded, the child dies, or we run out of
/// patience.
fn wait_ready(child: &mut Child, port: u16) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(HEALTH_TIMEOUT)
        .build()
        .map_err(|e| format!("[LLM-002] Could not create the local cleanup client. ({e})"))?;
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + READY_DEADLINE;
    loop {
        // Fail fast on a crash instead of waiting out the whole 180 s.
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("[LLM-002] Could not check the local cleanup engine. ({e})"))?
        {
            return Err(format!(
                "[LLM-002] The local cleanup engine exited while loading the model ({status}). Check that the model file is valid, or enable cloud mode in Settings."
            ));
        }
        if client
            .get(&url)
            .send()
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "[LLM-002] The local cleanup engine did not finish loading within {} s. Close other apps to free memory, or enable cloud mode in Settings.",
                READY_DEADLINE.as_secs()
            ));
        }
        std::thread::sleep(HEALTH_INTERVAL);
    }
}

/// Ask the OS for a free port, then hand it over. Never a hardcoded port —
/// two SmoothFlow instances, or a stale server, must not collide.
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("[LLM-002] Could not reserve a local port. ({e})"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("[LLM-002] Could not reserve a local port. ({e})"))
}

/// The catalog filename for the configured size. `Err` on an unknown size so a
/// typo in the config can never silently pick a different model.
fn resolve_model_name(size: &str) -> Result<&'static str, String> {
    offline::catalog()
        .iter()
        .find(|spec| spec.category == "llm" && spec.directory == size)
        .map(|spec| spec.filename)
        .ok_or_else(|| {
            format!(
                "[LLM-001] Unknown cleanup model size \"{size}\" in Settings. Pick 2b, 4b, or 9b, or enable cloud mode."
            )
        })
}

/// `<models root>/llm/<llm_model_size>/<filename>` — the layout
/// `offline::download_bundle` writes to.
fn model_path(config: &Config) -> Result<PathBuf, String> {
    let filename = resolve_model_name(&config.llm_model_size)?;
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "[LLM-001] Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();
    let path = offline::models_dir(&config_dir, &config.models_dir)
        .join("llm")
        .join(&config.llm_model_size)
        .join(filename);
    if !path.is_file() {
        return Err(format!(
            "[LLM-001] Cleanup model not found at {}. Download the Qwen models in Settings, or enable cloud mode in Settings.",
            path.display()
        ));
    }
    Ok(path)
}

/// `<app dir>/llama/llama-server[.exe]`. The llama.cpp build ships as a 9 KB
/// stub next to a pile of DLLs, so the whole directory has to be installed.
fn server_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("[LLM-001] Could not locate the SmoothFlow executable: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "[LLM-001] Could not locate the SmoothFlow installation directory.".to_string())?;
    Ok(dir.join("llama").join(format!("llama-server{}", std::env::consts::EXE_SUFFIX)))
}

/// OpenAI-compatible body. `cache_prompt` keeps the system prompt resident
/// across dictations, which is most of the cost of a short cleanup.
fn build_body(system: String, user: String) -> serde_json::Value {
    serde_json::json!({
        "model": "local",
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "temperature": 0,
        "max_tokens": 4096,
        "stream": false,
        "cache_prompt": true,
        // Qwen3.5 defaults to a reasoning pass. Left on, a 30-word dictation
        // ran past 1200 tokens at ~6 tok/s and never stopped. Cleanup wants the
        // answer, not the deliberation.
        "chat_template_kwargs": { "enable_thinking": false }
    })
}

/// `choices[0].message.content`, or an error. An empty `choices` and a null
/// content are both failures, never an empty string.
fn extract_content(body: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Message {
        #[serde(default)]
        content: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Choice {
        message: Message,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        choices: Vec<Choice>,
    }
    let parsed: Response = serde_json::from_str(body)
        .map_err(|e| format!("[LLM-002] Could not parse the local cleanup response. ({e})"))?;
    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .ok_or_else(|| "[LLM-002] The local cleanup engine returned no text.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_name_comes_from_the_catalog_for_every_known_size() {
        assert_eq!(resolve_model_name("2b").unwrap(), "Qwen3.5-2B-Q4_K_M.gguf");
        assert_eq!(resolve_model_name("4b").unwrap(), "Qwen3.5-4B-Q4_K_M.gguf");
        assert_eq!(resolve_model_name("9b").unwrap(), "Qwen3.5-9B-Q4_K_M.gguf");
    }

    #[test]
    fn unknown_model_size_errors_instead_of_picking_something() {
        // Empty, mis-cased, STT size, and a plausible typo all fail loudly.
        for size in ["", "4B", "v2", "12b", "4 b"] {
            let err = resolve_model_name(size).unwrap_err();
            assert!(err.starts_with("[LLM-001]"), "size {size:?} gave: {err}");
        }
    }

    #[test]
    fn model_path_uses_the_catalog_filename_under_llm_and_size() {
        for (size, filename) in [
            ("2b", "Qwen3.5-2B-Q4_K_M.gguf"),
            ("4b", "Qwen3.5-4B-Q4_K_M.gguf"),
            ("9b", "Qwen3.5-9B-Q4_K_M.gguf"),
        ] {
            let config = Config {
                llm_model_size: size.to_string(),
                ..Config::default()
            };
            // Installed or not, the catalog filename has to be the one named.
            match model_path(&config) {
                Ok(path) => assert_eq!(path.file_name().unwrap().to_str(), Some(filename)),
                Err(err) => {
                    assert!(err.contains(filename), "size {size} gave: {err}");
                    assert!(err.contains("llm"), "size {size} gave: {err}");
                }
            }
        }
        let config = Config {
            llm_model_size: "12b".to_string(),
            ..Config::default()
        };
        assert!(model_path(&config).is_err());
    }

    #[test]
    fn custom_models_dir_is_honoured() {
        let config = Config {
            models_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            llm_model_size: "4b".to_string(),
            ..Config::default()
        };
        match model_path(&config) {
            Ok(path) => assert!(path.starts_with(std::env::temp_dir())),
            Err(err) => assert!(err.contains(&std::env::temp_dir().to_string_lossy().into_owned())),
        }
    }

    #[test]
    fn request_body_is_deterministic_offline_and_unauthenticated() {
        let body = build_body("sys".to_string(), "user".to_string());
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["stream"], false);
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["model"], "local");
        // Without this the model runs a reasoning pass and never stops.
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "user");

        let json = body.to_string();
        for forbidden in ["Authorization", "api_key", "bearer", "http://", "https://"] {
            assert!(
                !json.to_lowercase().contains(&forbidden.to_lowercase()),
                "body must not mention {forbidden}: {json}"
            );
        }
    }

    #[test]
    fn extract_content_reads_the_first_choice() {
        assert_eq!(
            extract_content(r#"{"choices":[{"message":{"content":"hello there"}}]}"#).unwrap(),
            "hello there"
        );
        // Extra fields the server adds are ignored.
        assert_eq!(
            extract_content(
                r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi \u4f60\u597d"},"finish_reason":"stop"}],"usage":{"total_tokens":9}}"#
            )
            .unwrap(),
            "hi \u{4f60}\u{597d}"
        );
    }

    #[test]
    fn extract_content_rejects_empty_choices_null_content_and_garbage() {
        for body in [
            r#"{"choices":[]}"#,
            r#"{"choices":[{"message":{"content":null}}]}"#,
            r#"{"choices":[{"message":{}}]}"#,
            r#"{"choices":"nope"}"#,
            "not json",
        ] {
            let err = extract_content(body).unwrap_err();
            assert!(err.starts_with("[LLM-002]"), "body {body:?} gave: {err}");
        }
    }

    #[test]
    fn shutdown_on_an_empty_session_is_a_no_op() {
        shutdown();
        shutdown();
        // Still empty — a failed request must not have cached a dead session.
        assert!(SESSION.lock().unwrap().is_none());
    }

    #[test]
    fn free_port_returns_a_usable_loopback_port() {
        let port = free_port().unwrap();
        assert!(port > 0);
    }
}
