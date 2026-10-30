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
use std::path::{Path, PathBuf};
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

/// Load the helper in the background so the ~4 s model load overlaps the user
/// speaking instead of landing after the first utterance. Best-effort: a warm
/// that fails is not a user error, and `transcribe` respawns and reports the
/// real one. Safe to call concurrently with a dictation and with itself.
pub fn prewarm(config: &Config) {
    // ponytail: holds the session mutex for the whole model load, so a
    // dictation arriving mid-warm waits it out. Same total wait as spawning on
    // that dictation, so nothing is lost.
    let mut guard = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_some() {
        return;
    }
    *guard = spawn(config).ok();
}

/// Cap on cores handed to the speech engine.
///
/// The cap is not arbitrary: ONNX encoder sessions stop scaling past a handful
/// of threads and start competing with the audio callback, so a 32-thread
/// workstation must not ask for 31. Six matches the point where the measured
/// gain flattened out on the development machine (6 cores / 12 threads).
const STT_MAX_THREADS: usize = 6;

/// Cores the speech engine may use, given how many the machine has.
///
/// One core is always left over for Windows, the capture stream and the UI:
/// taking every core makes the whole app stutter while the model decodes.
fn threads_for(logical: usize) -> usize {
    logical.saturating_sub(1).clamp(2, STT_MAX_THREADS)
}

/// `threads_for` applied to the real machine.
fn stt_threads() -> usize {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    threads_for(logical)
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

    let files = model_files(config)?;
    let encoder = files.encoder.as_path();
    let decoder = files.decoder.as_path();
    let joiner = files.joiner.as_path();
    let tokens = files.tokens.as_path();

    let threads = stt_threads();
    sf_log!("STT: local helper decoding with {threads} threads");
    let mut child = Command::new(&helper)
        .arg("--encoder")
        .arg(&encoder)
        .arg("--decoder")
        .arg(&decoder)
        .arg("--joiner")
        .arg(&joiner)
        .arg("--tokens")
        .arg(&tokens)
        .arg("--threads")
        .arg(threads.to_string())
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

/// The four files handed to the helper. Paths are resolved but not yet checked.
#[derive(Debug)]
struct ModelFiles {
    encoder: PathBuf,
    decoder: PathBuf,
    joiner: PathBuf,
    tokens: PathBuf,
}

/// The four parts found in `dir`, `None` where a part is missing.
///
/// Prefix matching, not the exact catalog filenames: a bundle ships as
/// `encoder.onnx`, `encoder.int8.onnx` or with a language suffix. This must
/// agree with what `inspect_stt_dir` accepts in Settings — otherwise a folder
/// the UI reports as complete would fail at spawn instead.
fn resolve_in(dir: &Path) -> [Option<PathBuf>; 4] {
    let files = match std::fs::read_dir(dir) {
        Ok(entries) => crate::local_models::collect_files(entries),
        Err(_) => Vec::new(),
    };
    let mut found: [Option<PathBuf>; 4] = [None, None, None, None];
    for (slot, (prefix, extension)) in found.iter_mut().zip(crate::local_models::STT_PARTS) {
        *slot = crate::local_models::pick(&files, prefix, extension).map(PathBuf::from);
    }
    found
}

/// Resolve and verify the four Parakeet files for `config`.
///
/// A non-empty `custom_stt_dir` wins, so a user's own bundle is read where they
/// put it; otherwise the catalog layout for `stt_version` applies. Every path
/// is checked here — a missing file is a user-fixable error with the fix in the
/// message, never a spawn the helper would fail to load.
fn model_files(config: &Config) -> Result<ModelFiles, String> {
    let custom = config.custom_stt_dir.trim();
    let dir = if custom.is_empty() {
        model_dir(config)?
    } else {
        PathBuf::from(custom)
    };
    let found = resolve_in(&dir);
    let expected = [ENCODER, DECODER, JOINER, TOKENS];
    for (index, path) in found.iter().enumerate() {
        if path.is_some() {
            continue;
        }
        let name = crate::local_models::STT_PARTS[index].0;
        return Err(if custom.is_empty() {
            // Catalog download: name the file it expects, so the fix is obvious.
            format!(
                "[STT-002] Speech model file not found at {}. Download the Parakeet models in Settings, or enable cloud mode in Settings.",
                dir.join(expected[index]).display()
            )
        } else {
            format!(
                "[STT-002] Speech model file '{name}' not found in {}. Pick a folder that holds encoder, decoder and joiner (.onnx) plus tokens.txt, or enable cloud mode in Settings.",
                dir.display()
            )
        });
    }
    let [Some(encoder), Some(decoder), Some(joiner), Some(tokens)] = found else {
        // Unreachable: every slot was checked above.
        return Err("[STT-002] The speech model folder is incomplete.".to_string());
    };
    Ok(ModelFiles {
        encoder,
        decoder,
        joiner,
        tokens,
    })
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
    fn threads_never_take_every_core_and_never_go_past_the_cap() {
        // A 12-thread laptop (the dev machine) may use 6, not 11.
        assert_eq!(threads_for(12), 6);
        assert_eq!(threads_for(32), STT_MAX_THREADS);
        assert_eq!(threads_for(8), 6);
        // Small machines still take one core off the top.
        assert_eq!(threads_for(4), 3);
        assert_eq!(threads_for(3), 2);
        // A 1-2 core machine cannot donate a core; the floor is 2, which is
        // what the engine used before this change (so it is never slower).
        assert_eq!(threads_for(2), 2);
        assert_eq!(threads_for(1), 2);
    }

    #[test]
    fn stt_threads_is_always_a_legal_helper_argument() {
        // The helper rejects <1 and >64, so a wrong answer here would make
        // every dictation fail at spawn instead of degrading.
        let threads = stt_threads();
        assert!((2..=STT_MAX_THREADS).contains(&threads), "got {threads}");
    }

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

    fn temp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("smoothflow-stt-{tag}-{:?}", std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_custom_stt_dir_wins_over_the_catalog_layout() {
        let dir = temp_dir("custom");
        for name in [ENCODER, DECODER, JOINER, TOKENS] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let config = Config {
            custom_stt_dir: dir.to_string_lossy().into_owned(),
            // A catalog dir that has nothing in it proves the custom folder won.
            models_dir: temp_dir("custom-catalog").to_string_lossy().into_owned(),
            ..Config::default()
        };
        let files = model_files(&config).unwrap();
        assert_eq!(files.encoder, dir.join(ENCODER));
        assert_eq!(files.decoder, dir.join(DECODER));
        assert_eq!(files.joiner, dir.join(JOINER));
        assert_eq!(files.tokens, dir.join(TOKENS));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_in_a_custom_dir_names_the_folder_and_the_file() {
        let dir = temp_dir("custom-missing");
        let config = Config {
            custom_stt_dir: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };
        let err = model_files(&config).unwrap_err();
        assert!(err.starts_with("[STT-002]"), "got: {err}");
        assert!(err.contains(ENCODER), "got: {err}");
        let dir_text = dir.to_string_lossy().into_owned();
        assert!(err.contains(&dir_text), "got: {err}");
        assert!(err.contains("cloud mode"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_custom_stt_dir_keeps_the_catalog_layout_and_its_wording() {
        // Nothing installed under this root, so the catalog branch must speak.
        let config = Config {
            models_dir: temp_dir("catalog").to_string_lossy().into_owned(),
            stt_version: "v2".into(),
            ..Config::default()
        };
        let err = model_files(&config).unwrap_err();
        assert!(err.starts_with("[STT-002]"), "got: {err}");
        assert!(
            err.contains("Download the Parakeet models in Settings"),
            "got: {err}"
        );
        assert!(err.contains("v2"), "the version folder must be named: {err}");
    }
}
