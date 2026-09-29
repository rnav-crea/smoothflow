//! Local (offline) speech-to-text via the `smoothflow-stt` native helper.
//!
//! The helper is a long-lived child process: loading Parakeet costs ~4 s while
//! decoding is ~0.4 s, so spawning one process per dictation would add 4 s to
//! every dictation. We spawn lazily on the first utterance and keep the
//! session process-wide behind a mutex.
//!
//! Protocol (see `native/smoothflow-stt/`): the helper writes `{"ready":true}`
//! once the model is loaded, then answers every request with exactly one JSON
//! line. A request is a little-endian `u32` sample count followed by that many
//! little-endian `f32` samples. Closing stdin makes it exit 0 — that is the
//! shutdown path, there is no quit command.

use crate::config::Config;
use crate::offline;
use crate::sf_log;
use crate::transcription::resample_to_16k;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

const HELPER_NAME: &str = "smoothflow-stt";
const ENCODER: &str = "encoder.int8.onnx";
const DECODER: &str = "decoder.int8.onnx";
const JOINER: &str = "joiner.int8.onnx";
const TOKENS: &str = "tokens.txt";

/// A live helper process plus the two pipes we speak to it over.
struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    /// One request/response round trip. `Err` always means the session is no
    /// longer trustworthy and the caller must drop it.
    fn exchange(&mut self, samples: &[f32]) -> Result<String, String> {
        self.stdin
            .write_all(&encode_request(samples))
            .map_err(|e| format!("[STT-004] Could not send audio to the local speech engine. ({e})"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("[STT-004] Could not send audio to the local speech engine. ({e})"))?;
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("[STT-004] Could not read from the local speech engine. ({e})"))?;
        if read == 0 {
            return Err("[STT-004] The local speech engine stopped responding. Try again, or enable cloud mode in Settings.".into());
        }
        parse_response(&line)
    }

    /// Dropping stdin makes the helper exit 0; kill first so a wedged process
    /// cannot outlive the session.
    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Transcribe 16 kHz-ready mono `f32` samples on-device.
/// Returns a user-readable `[STT-00n] ...` error when the helper or a model
/// file is missing, or when the engine fails. Never falls back to the network.
pub fn transcribe(samples: &[f32], sample_rate: u32, config: &Config) -> Result<String, String> {
    let audio = resample_to_16k(samples, sample_rate);
    if audio.is_empty() {
        return Ok(String::new());
    }

    let mut guard = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        sf_log!("STT: starting local helper");
        *guard = Some(spawn(config)?);
    }
    let outcome = match guard.as_mut() {
        Some(session) => session.exchange(&audio),
        // Unreachable: the session was just spawned, but never unwrap I/O state.
        None => Err("[STT-004] The local speech engine session is unavailable. Try again.".into()),
    };
    if outcome.is_err() {
        // ponytail: kill on *any* engine error, not just transport failures —
        // respawning costs 4 s but can never reuse a desynchronised pipe.
        if let Some(mut session) = guard.take() {
            session.shutdown();
        }
    }
    outcome
}

fn spawn(config: &Config) -> Result<Session, String> {
    if !platform_supported() {
        return Err(unsupported_platform_message());
    }
    let helper = helper_path()?;
    if !helper.is_file() {
        return Err(format!(
            "[STT-001] Local speech helper not found at {}. Reinstall SmoothFlow or enable cloud mode in Settings.",
            helper.display()
        ));
    }

    let models = model_dir(config)?;
    let encoder = models.join(ENCODER);
    let decoder = models.join(DECODER);
    let joiner = models.join(JOINER);
    let tokens = models.join(TOKENS);
    for path in [&encoder, &decoder, &joiner, &tokens] {
        if !path.is_file() {
            return Err(format!(
                "[STT-002] Speech model file not found at {}. Download the Parakeet models in Settings, or enable cloud mode in Settings.",
                path.display()
            ));
        }
    }

    let mut child = Command::new(&helper)
        .arg("--encoder")
        .arg(&encoder)
        .arg("--decoder")
        .arg(&decoder)
        .arg("--joiner")
        .arg(&joiner)
        .arg("--tokens")
        .arg(&tokens)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("[STT-001] Could not start the local speech helper at {}: {e}", helper.display()))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "[STT-004] The local speech helper gave us no input pipe.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "[STT-004] The local speech helper gave us no output pipe.".to_string())?;
    let mut session = Session {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    };

    // Wait for the model to finish loading (~4 s) before the first request.
    let mut line = String::new();
    match session.stdout.read_line(&mut line) {
        Ok(0) => {
            session.shutdown();
            return Err("[STT-004] The local speech helper exited while loading the model. Check that the model files are valid.".into());
        }
        Ok(_) => {}
        Err(e) => {
            session.shutdown();
            return Err(format!("[STT-004] Could not read the local speech helper's startup message. ({e})"));
        }
    }
    if parse_ready(&line) != Ok(()) {
        session.shutdown();
        return Err(format!("[STT-004] The local speech helper did not report a loaded model: {}", line.trim()));
    }
    Ok(session)
}

/// The helper is a Windows build; there is nothing to look for elsewhere.
fn platform_supported() -> bool {
    cfg!(target_os = "windows")
}

fn unsupported_platform_message() -> String {
    // Fixed string, not `consts::OS`: the user should read "macOS", not "macos".
    "[STT-001] Offline mode is not yet available on macOS or Linux. Please use cloud mode in Settings.".to_string()
}

/// `<app dir>/smoothflow-stt[.exe]`, next to the running binary.
fn helper_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("[STT-001] Could not locate the SmoothFlow executable: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "[STT-001] Could not locate the SmoothFlow installation directory.".to_string())?;
    Ok(dir.join(format!("{HELPER_NAME}{}", std::env::consts::EXE_SUFFIX)))
}

/// `<models root>/stt/<stt_version>` — the same layout `offline::download`
/// writes to.
fn model_dir(config: &Config) -> Result<PathBuf, String> {
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| "Could not determine the SmoothFlow config directory.".to_string())?
        .to_path_buf();
    Ok(offline::models_dir(&config_dir, &config.models_dir)
        .join("stt")
        .join(&config.stt_version))
}

/// LE `u32` sample count followed by LE `f32` samples. `to_le_bytes` keeps this
/// portable — no transmute, no host-endianness assumption.
fn encode_request(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + samples.len() * 4);
    bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    bytes
}

fn parse_ready(line: &str) -> Result<(), ()> {
    #[derive(serde::Deserialize)]
    struct Ready {
        ready: bool,
    }
    match serde_json::from_str::<Ready>(line.trim()) {
        Ok(ready) if ready.ready => Ok(()),
        _ => Err(()),
    }
}

/// One response line: `{"text":"..."}` or `{"error":"..."}`.
fn parse_response(line: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        error: Option<String>,
    }
    let response: Response = serde_json::from_str(line.trim()).map_err(|e| {
        format!("[STT-004] Could not parse the local speech engine response. ({e})")
    })?;
    if let Some(error) = response.error {
        return Err(format!("[STT-003] Local speech engine error: {error}"));
    }
    response
        .text
        .ok_or_else(|| "[STT-004] The local speech engine returned no text.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_request_writes_le_count_then_le_f32_samples() {
        let payload = encode_request(&[1.0, -2.0]);
        assert_eq!(payload.len(), 4 + 2 * 4);
        assert_eq!(&payload[..4], &2u32.to_le_bytes());
        assert_eq!(&payload[4..8], &1.0f32.to_le_bytes());
        assert_eq!(&payload[8..12], &(-2.0f32).to_le_bytes());
        assert_eq!(f32::from_le_bytes(payload[4..8].try_into().unwrap()), 1.0);
        // An empty utterance is legal: count only, no samples.
        assert_eq!(encode_request(&[]), 0u32.to_le_bytes().to_vec());
    }

    #[test]
    fn parse_response_reads_text_including_non_ascii() {
        assert_eq!(parse_response("{\"text\":\"hello there\"}\n").unwrap(), "hello there");
        assert_eq!(parse_response("{\"text\":\"\"}\n").unwrap(), "");
        assert_eq!(parse_response("{\"text\":\"caf\u{e9} \u{4f60}\u{597d}\"}\n").unwrap(), "caf\u{e9} \u{4f60}\u{597d}");
    }

    #[test]
    fn parse_response_surfaces_engine_errors_and_garbage() {
        let err = parse_response("{\"error\":\"model load failed\"}\n").unwrap_err();
        assert!(err.contains("model load failed"), "got: {err}");
        assert!(err.starts_with("[STT-003]"), "got: {err}");
        assert!(parse_response("not json\n").is_err());
        assert!(parse_response("{}\n").is_err());
    }

    #[test]
    fn empty_audio_returns_empty_without_touching_the_helper() {
        // No helper, no models, no config dir needed: the empty case must
        // short-circuit before any of that is resolved.
        let config = Config::default();
        assert_eq!(transcribe(&[], 16000, &config).unwrap(), "");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn an_unsupported_platform_says_so_instead_of_missing_the_helper() {
        assert!(!platform_supported());
        assert!(unsupported_platform_message().starts_with("[STT-001]"));
        assert!(unsupported_platform_message().contains("macOS or Linux"));
    }

    #[test]
    fn parse_ready_accepts_only_the_hello_line() {
        assert!(parse_ready("{\"ready\":true}\n").is_ok());
        assert!(parse_ready("{\"ready\":false}\n").is_err());
        assert!(parse_ready("{\"text\":\"x\"}\n").is_err());
    }
}
