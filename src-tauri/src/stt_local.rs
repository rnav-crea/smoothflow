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

/// One file a model family needs: how to recognise it in a folder and which
/// helper flag feeds it. `pick` matches the prefix so the same part can ship
/// as `encoder.onnx` or `encoder.int8.onnx`; `int8` wins (see `local_models`).
struct Part {
    prefix: &'static str,
    extension: &'static str,
    /// Exact flag name the helper accepts (`main.cpp` `DestinationFor`).
    flag: &'static str,
}

/// One supported model family: the file set the helper needs to load it.
struct Family {
    id: &'static str,
    parts: &'static [Part],
}

const fn file_part(prefix: &'static str, flag: &'static str) -> Part {
    Part {
        prefix,
        extension: ".onnx",
        flag,
    }
}

// Order is load-bearing: the first family whose file set is complete wins, so
// families are listed longest-and-most-specific first. `transducer` is unique
// (nothing else has a joiner); whisper/canary/firered are distinguished from
// the two-file families only by order, which is fine for a *custom* folder —
// catalog folders resolve by their configured model id in phase 3.
const FAMILIES: &[Family] = &[
    // Parakeet TDT (v2/v3): encoder + decoder + joiner + tokens.
    Family {
        id: "transducer",
        parts: &[
            file_part("encoder", "--encoder"),
            file_part("decoder", "--decoder"),
            file_part("joiner", "--joiner"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // Whisper: encoder + decoder + tokens (no joiner).
    Family {
        id: "whisper",
        parts: &[
            file_part("encoder", "--whisper-encoder"),
            file_part("decoder", "--whisper-decoder"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // Moonshine v1: preprocessor + encoder + uncached + cached decoders.
    Family {
        id: "moonshine",
        parts: &[
            file_part("preprocessor", "--moonshine-preprocessor"),
            file_part("encoder", "--moonshine-encoder"),
            file_part("uncacheddecoder", "--moonshine-uncached-decoder"),
            file_part("cacheddecoder", "--moonshine-cached-decoder"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // Moonshine v2: preprocessor + encoder + one merged decoder.
    Family {
        id: "moonshine-v2",
        parts: &[
            file_part("preprocessor", "--moonshine-preprocessor"),
            file_part("encoder", "--moonshine-encoder"),
            file_part("mergeddecoder", "--moonshine-merged-decoder"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // Canary / FireRed ASR: encoder + decoder + tokens.
    Family {
        id: "canary",
        parts: &[
            file_part("encoder", "--canary-encoder"),
            file_part("decoder", "--canary-decoder"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    Family {
        id: "fire_red_asr",
        parts: &[
            file_part("encoder", "--firered-encoder"),
            file_part("decoder", "--firered-decoder"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // SenseVoice / Paraformer / Dolphin / FireRed CTC: model.onnx + tokens.
    Family {
        id: "sense_voice",
        parts: &[
            file_part("model", "--sensevoice"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    Family {
        id: "paraformer",
        parts: &[
            file_part("model", "--paraformer"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    Family {
        id: "dolphin",
        parts: &[
            file_part("model", "--dolphin"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    Family {
        id: "fire_red_asr_ctc",
        parts: &[
            file_part("model", "--firered-ctc"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    // NeMo CTC (Parakeet CTC) / Zipformer CTC: encoder.onnx + tokens.
    Family {
        id: "nemo_ctc",
        parts: &[
            file_part("encoder", "--nemo-ctc"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
    Family {
        id: "zipformer2_ctc",
        parts: &[
            file_part("encoder", "--zipformer-ctc"),
            Part {
                prefix: "tokens",
                extension: ".txt",
                flag: "--tokens",
            },
        ],
    },
];

/// A resolved model: which family it is and the `(flag, path)` list to spawn
/// the helper with, in family part order.
#[derive(Debug)]
struct ModelFiles {
    family: &'static str,
    args: Vec<(&'static str, PathBuf)>,
}

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
    sf_log!("STT: local helper loading the {} family", files.family);

    let threads = stt_threads();
    sf_log!("STT: local helper decoding with {threads} threads");
    let mut command = Command::new(&helper);
    for (flag, path) in &files.args {
        command.arg(flag).arg(path);
    }
    let mut child = command
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

/// Resolve the helper binary.
///
/// Order: the downloaded engine under the config directory
/// (`<config dir>/engines/<target>/smoothflow-stt[.exe]`, written by
/// `engines::download`) first, then `<app dir>/smoothflow-stt[.exe]` next to the
/// running binary. The second branch is what a dev build and a hand-installed
/// copy use, so neither has to download anything.
fn helper_path() -> Result<PathBuf, String> {
    if let Some(helper) = crate::engines::installed_helper() {
        return Ok(helper);
    }
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

/// Resolve and verify the model files for `config`.
///
/// A non-empty `custom_stt_dir` wins, so a user's own bundle is read where they
/// put it; otherwise the catalog layout for `stt_version` applies. The folder
/// is matched against `FAMILIES` by shape: the first family whose whole file
/// set is present wins. A folder no family recognises is a user-fixable error
/// with the fix in the message, never a spawn the helper would fail to load.
fn model_files(config: &Config) -> Result<ModelFiles, String> {
    let custom = config.custom_stt_dir.trim();
    let dir = if custom.is_empty() {
        model_dir(config)?
    } else {
        PathBuf::from(custom)
    };
    if let Some(files) = resolve_in(&dir) {
        return Ok(files);
    }
    let err = if custom.is_empty() {
        // Catalog download: name what the folder was expected to hold.
        format!(
            "[STT-002] Speech model files not recognised in {}. Download the Parakeet models in Settings, or enable cloud mode in Settings.",
            dir.display()
        )
    } else {
        format!(
            "[STT-002] Speech model files not recognised in {}. Pick a folder that holds one supported model family — for example a Parakeet folder (encoder, decoder and joiner .onnx plus tokens.txt) or a SenseVoice folder (model.onnx plus tokens.txt) — or enable cloud mode in Settings.",
            dir.display()
        )
    };
    Err(err)
}

/// Match `dir` against `FAMILIES` by shape: the first family whose whole file
/// set is found wins.
///
/// Prefix matching, not exact catalog filenames: a bundle ships as
/// `encoder.onnx`, `encoder.int8.onnx` or with a language suffix. This must
/// agree with what `inspect_stt_dir` accepts in Settings — otherwise a folder
/// the UI reports as complete would fail at spawn instead (Settings' inspector
/// is still Parakeet-only; that widens with the catalog in phase 3).
fn resolve_in(dir: &Path) -> Option<ModelFiles> {
    let files = match std::fs::read_dir(dir) {
        Ok(entries) => crate::local_models::collect_files(entries),
        Err(_) => Vec::new(),
    };
    for family in FAMILIES {
        let mut args = Vec::with_capacity(family.parts.len());
        let mut complete = true;
        for part in family.parts {
            match crate::local_models::pick(&files, part.prefix, part.extension) {
                Some(path) => args.push((part.flag, PathBuf::from(path))),
                // A family never lists the same part twice, so `args` holds a
                // strict prefix; a break cannot leave a wrong-looking list.
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            return Some(ModelFiles {
                family: family.id,
                args,
            });
        }
    }
    None
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
        assert_eq!(files.family, "transducer");
        assert_eq!(arg_path(&files, "--encoder"), dir.join(ENCODER));
        assert_eq!(arg_path(&files, "--decoder"), dir.join(DECODER));
        assert_eq!(arg_path(&files, "--joiner"), dir.join(JOINER));
        assert_eq!(arg_path(&files, "--tokens"), dir.join(TOKENS));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolution_picks_the_first_complete_family_by_shape() {
        // encoder + tokens only: not transducer (no joiner), not whisper (no
        // decoder) — but it is exactly the NeMo CTC (Parakeet CTC) shape.
        let dir = temp_dir("ctc-shape");
        for name in ["encoder.int8.onnx", TOKENS] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let files = resolve_in(&dir).unwrap();
        assert_eq!(files.family, "nemo_ctc");
        assert_eq!(arg_path(&files, "--nemo-ctc"), dir.join("encoder.int8.onnx"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_model_tokens_dir_resolves_to_sense_voice_before_its_rivals() {
        // model.onnx + tokens.txt is the SenseVoice, Paraformer, Dolphin and
        // FireRed-CTC shape; the family table order decides, SenseVoice first.
        let dir = temp_dir("sv-shape");
        for name in ["model.int8.onnx", TOKENS] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let files = resolve_in(&dir).unwrap();
        assert_eq!(files.family, "sense_voice");
        assert_eq!(arg_path(&files, "--sensevoice"), dir.join("model.int8.onnx"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn arg_path(files: &ModelFiles, flag: &str) -> PathBuf {
        files
            .args
            .iter()
            .find(|(name, _)| *name == flag)
            .map(|(_, path)| path.clone())
            .unwrap_or_else(|| panic!("no {flag} in {:?}", files.args))
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
        assert!(err.contains("not recognised"), "got: {err}");
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
