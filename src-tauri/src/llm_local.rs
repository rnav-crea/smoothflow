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
use crate::postprocess::{
    categorize_window, strip_reasoning_block, system_prompt_offline, user_message, WindowCategory,
};
use crate::sf_log;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The window category whose prompt the live server already holds in its KV
/// cache. Cleared whenever the server is replaced or torn down: a fresh process
/// starts with an empty cache and has to re-prime. `None` means "nothing primed".
static PRIMED_CATEGORY: Mutex<Option<&'static str>> = Mutex::new(None);

/// Clean up a raw transcript on-device. Returns a user-readable `[LLM-00n]`
/// error when the server binary, the model, or the server itself is unhappy.
/// Never falls back to the network.
pub fn cleanup(text: &str, config: &Config, context: &str) -> Result<String, String> {
    with_session(config, |session| {
        let result = request(session, text, config, context);
        // Only a failed request leaves the session untrustworthy. This used to
        // tear the server down unconditionally, so every non-streaming cleanup
        // (every re-dictation, plus live dictation with auto-paste off) killed
        // the engine and made the *next* call load the model from scratch on a
        // new port — a ~9 s stall with no error in the log explaining it.
        let tear_down = result.is_err();
        (result, tear_down)
    })
}

/// Same cleanup, but each generated chunk is handed to `on_partial` as it
/// arrives and the caller pastes it immediately instead of waiting ~7 s for the
/// whole sentence. Returns the full accumulated text.
///
/// `on_partial` receives each *delta*, never the accumulated string: LLM output
/// at `temperature: 0` is append-only, so the caller only ever appends. No
/// retraction, no in-place replacement.
pub fn cleanup_streaming(
    text: &str,
    config: &Config,
    context: &str,
    on_partial: &mut dyn FnMut(&str),
) -> Result<String, String> {
    with_session(config, |session| {
        request_streaming(session, text, config, context, on_partial)
    })
}

/// Resolve the model, reuse or spawn the server, run `request`, and tear the
/// session down when `request` asks for it. The closure returns its result plus
/// a kill flag, because a stream that dies mid-sentence still has to drop the
/// server while reporting success to the caller.
fn with_session<T>(
    config: &Config,
    run: impl FnOnce(&Session) -> (Result<T, String>, bool),
) -> Result<T, String> {
    // Checked here too: model_path would otherwise report a missing model on a
    // platform that has no engines at all.
    if !platform_supported() {
        return Err(unsupported_platform_message());
    }
    let model = model_path(config)?;
    let server = server_path()?;

    let mut guard = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A different model means a different process: the old one is holding
    // several GB of RAM on a file we no longer need.
    if guard.as_ref().is_some_and(|s| s.model != model) {
        if let Some(mut old) = guard.take() {
            sf_log!("LLM: dropping the local cleanup engine — the model path changed");
            old.shutdown();
            // New process, empty KV cache: the next dictation must re-prime.
            set_primed_category(None);
        }
    }
    if guard.is_none() {
        sf_log!("LLM: starting a local cleanup engine for {}", model.display());
        *guard = Some(spawn(&server, &model)?);
    }
    let (outcome, tear_down) = match guard.as_ref() {
        Some(session) => run(session),
        // Unreachable: the session was just spawned, but never unwrap state.
        None => (
            Err("[LLM-002] The local cleanup session is unavailable. Try again.".into()),
            true,
        ),
    };
    if tear_down {
        // Never cache a half-dead session: the next call must respawn cleanly.
        // Logged, because the respawn otherwise showed up as nothing but a new
        // port number on the next "engine ready" line.
        if let Some(mut dead) = guard.take() {
            sf_log!("LLM: dropping the local cleanup engine after a failed request");
            dead.shutdown();
            set_primed_category(None);
        }
    }
    outcome
}

/// Kill and reap the server. Safe with no session and safe to call twice —
/// an orphaned `llama-server` would hold the model in RAM after we exit.
pub fn shutdown() {
    // Set before the lock so a prewarm already blocked on the mutex cannot
    // resurrect a server after this returns.
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut guard = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(mut session) = guard.take() {
        session.shutdown();
    }
    // A torn-down server has no cache left to hit.
    set_primed_category(None);
}

/// Start the server in the background so the multi-second model load overlaps
/// the user speaking instead of landing after the first dictation.
/// Best-effort: a warm that fails is not a user error, and `cleanup` respawns
/// and reports the real one. Call on a background thread — `spawn` polls
/// `/health` for up to 180 s.
pub fn prewarm(config: &Config) {
    let (Ok(model), Ok(server)) = (model_path(config), server_path()) else {
        return;
    };
    // ponytail: holds the session mutex across the whole model load and the
    // prompt prime, so a dictation arriving mid-warm waits it out and then
    // starts on a warm server. Same total wait as spawning on that dictation,
    // minus the prefill it would otherwise pay twice.
    let mut guard = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_some() {
        return;
    }
    *guard = spawn(&server, &model).ok();
    if let Some(session) = guard.as_ref() {
        prime_prompt_cache(session, config);
    }
}

/// Prefill the prompt for `category` while the user is still speaking.
///
/// The prefill, not the generation, is what the user waits for: a ~1500-token
/// prompt costs ~18 ms per uncached token here, so the tail of the instruction
/// sheet lands at 14-27 s on the first dictation into each new window category.
/// The hotkey already knows the target window (the title has to be read before
/// the overlay steals focus) and the user then speaks for 6-28 s, so starting
/// the prefill at press time makes it free. Best-effort: a failed prime is not a
/// user error, the real request reports its own.
///
/// Call on a background thread — the prefill holds `SESSION` and can take tens
/// of seconds. It never spawns a server: `prewarm` owns creation.
pub fn prime_for_category(config: &Config, context: &str) {
    if SHUTTING_DOWN.load(std::sync::atomic::Ordering::Relaxed) || !platform_supported() {
        return;
    }
    // The same call the real request makes, so the prefilled prompt is
    // byte-identical to the one `request_streaming` will send for this window.
    let category = categorize_window(context);
    let label = category_label(category);
    // Lock released before any work: the check is a cheap fast path, not a claim.
    if current_primed_category() == Some(label) {
        sf_log!("LLM: prompt cache already primed for category {label}");
        return;
    }
    // ponytail: held across the prefill on purpose — that is the prefill the
    // next dictation would pay anyway, and it must not run twice at once.
    let guard = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(session) = guard.as_ref() else {
        sf_log!("LLM: no local engine yet, skipping prime for category {label}");
        return;
    };
    sf_log!("LLM: priming prompt cache for category {label} while recording");
    // Status codes and counts only — never the prompt or the answer.
    match send_prime(session, config, category) {
        PrimeOutcome::Primed(usage) => {
            set_primed_category(Some(label));
            sf_log!(
                "LLM: prompt cache primed for category {label} | prompt {} | cached {} | completion {}",
                count_text(usage.prompt),
                count_text(usage.cached),
                count_text(usage.completion)
            );
        }
        PrimeOutcome::Rejected(status) => {
            sf_log!("LLM: prime for category {label} rejected by the engine ({status})")
        }
        PrimeOutcome::Failed(err) => sf_log!("LLM: prime for category {label} failed ({err})"),
    }
}

/// One throwaway completion that leaves the system prompt resident in the
/// server's KV cache. Without it the first dictation in each window category
/// pays the full prefill of the ~1100-token prompt. Best-effort by design:
/// every outcome is logged and dropped, because a warm server that skipped the
/// prime is still correct — just slower.
fn prime_prompt_cache(session: &Session, config: &Config) {
    if SHUTTING_DOWN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    match send_prime(session, config, WindowCategory::Default) {
        PrimeOutcome::Primed(usage) => {
            sf_log!("LLM: prompt cache primed");
            set_primed_category(Some(category_label(WindowCategory::Default)));
            sf_log!(
                "LLM: prime usage | prompt {} | cached {} | completion {}",
                count_text(usage.prompt),
                count_text(usage.cached),
                count_text(usage.completion)
            );
        }
        PrimeOutcome::Rejected(status) => {
            sf_log!("LLM: prompt prime rejected by the engine ({status})")
        }
        PrimeOutcome::Failed(err) => sf_log!("LLM: prompt prime failed ({err})"),
    }
}

/// What one prime attempt did. Status and counts only: the prompt that was sent
/// and the single token that came back are never kept or logged.
enum PrimeOutcome {
    Primed(TokenUsage),
    /// The engine answered, but not with a 2xx.
    Rejected(reqwest::StatusCode),
    /// No answer at all: the client could not be built or the request failed.
    Failed(String),
}

/// Send the prime body for one category. Shared by the startup prime and the
/// while-recording prime so the two can never drift in timeout or error policy.
fn send_prime(session: &Session, config: &Config, category: WindowCategory) -> PrimeOutcome {
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", session.port);
    let outcome = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
        .and_then(|client| {
            client
                .post(&url)
                .json(&prime_body(config, category))
                .send()
                .map_err(|e| e.to_string())
        });
    // Status codes and transport errors only — never the prompt or the answer.
    match outcome {
        Ok(resp) if resp.status().is_success() => {
            // Read on success only, so a failure keeps its bare status message.
            // The body is parsed for counts and dropped: the prompt is the real
            // one, and the answer is one token nobody reads.
            let mut usage = TokenUsage::default();
            if let Ok(body) = resp.json::<serde_json::Value>() {
                usage.absorb(&body);
            }
            PrimeOutcome::Primed(usage)
        }
        Ok(resp) => PrimeOutcome::Rejected(resp.status()),
        Err(err) => PrimeOutcome::Failed(err),
    }
}

/// The category whose prompt the live server already holds, if any.
fn current_primed_category() -> Option<&'static str> {
    *PRIMED_CATEGORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_primed_category(label: Option<&'static str>) {
    *PRIMED_CATEGORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = label;
}

/// A stable, log-safe name for a category. Matched by hand rather than derived
/// so the label a log line shows can never silently change with the enum.
fn category_label(category: WindowCategory) -> &'static str {
    match category {
        WindowCategory::Default => "default",
        WindowCategory::Email => "email",
        WindowCategory::Code => "code",
        WindowCategory::Chat => "chat",
        WindowCategory::Browser => "browser",
        WindowCategory::Document => "document",
    }
}

/// The prime body: the real system prompt for `category`, the real user-message
/// wrapper, one token of generation. The point is the prefix, so only the token
/// counts are read back — never the prompt or the answer.
fn prime_body(config: &Config, category: WindowCategory) -> serde_json::Value {
    let mut body = build_body(
        system_prompt_offline(config, category),
        user_message("", ""),
        false,
    );
    body["max_tokens"] = 1.into();
    body
}

fn spawn(server: &PathBuf, model: &PathBuf) -> Result<Session, String> {
    // ponytail: checked here rather than in `cleanup` and `prewarm` separately —
    // this is the only place a process can come into existence, so one gate
    // covers every caller. Nothing else to synchronise with, hence Relaxed.
    if SHUTTING_DOWN.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("[LLM-002] The local cleanup engine is shutting down.".into());
    }
    if !platform_supported() {
        return Err(unsupported_platform_message());
    }
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
        .map_err(|e| {
            format!(
                "[LLM-001] Could not start the local cleanup engine at {}: {e}",
                server.display()
            )
        })?;

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
fn request(
    session: &Session,
    text: &str,
    config: &Config,
    context: &str,
) -> Result<String, String> {
    let body = build_body(
        system_prompt_offline(config, categorize_window(context)),
        user_message(text, context),
        false,
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
    // Same one-line usage metric as the streaming path: without it a
    // re-dictation (non-streaming only) had no measured cleanup cost at all.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) {
        let mut usage = TokenUsage::default();
        usage.absorb(&parsed);
        log_usage(&usage);
    }
    // Qwen3.5 can emit a thinking block; the shared stripper already handles it.
    Ok(strip_reasoning_block(&extract_content(&raw)?))
}

/// The streamed twin of `request`. Each `delta.content` is handed to
/// `on_partial` as it is read, so the caller can paste while the model is still
/// generating. The kill flag is `true` for every failure — including a stream
/// that dies mid-sentence — so the next dictation respawns cleanly.
fn request_streaming(
    session: &Session,
    text: &str,
    config: &Config,
    context: &str,
    on_partial: &mut dyn FnMut(&str),
) -> (Result<String, String>, bool) {
    let body = build_body(
        system_prompt_offline(config, categorize_window(context)),
        user_message(text, context),
        true,
    );
    let client = match reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return (
                Err(format!(
                    "[LLM-002] Could not create the local cleanup client. ({e})"
                )),
                true,
            )
        }
    };
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", session.port);
    let resp = match client.post(&url).json(&body).send() {
        Ok(resp) => resp,
        Err(e) => {
            return (
                Err(format!("[LLM-002] Local cleanup request failed. ({e})")),
                true,
            )
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        return (Err(format!("[LLM-002] Local cleanup engine error {status}.")), true);
    }

    let mut acc = String::new();
    let mut usage = TokenUsage::default();
    let mut reader = BufReader::new(resp);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => match sse_step(&line) {
                SseStep::Chunk(chunk) => {
                    on_partial(&chunk);
                    acc.push_str(&chunk);
                }
                // Counts only: never text, so never pasted.
                SseStep::Usage(tail) => usage.absorb(&tail),
                SseStep::Done => break,
                SseStep::Ignore => {}
            },
            Err(e) => {
                log_usage(&usage);
                return truncated(
                    &acc,
                    &format!("[LLM-002] The local cleanup stream failed. ({e})"),
                );
            }
        }
    }
    // One line per dictation, whether or not the builder above was the one that
    // answered. Missing counts log as `?`, never as a zero.
    log_usage(&usage);
    if acc.trim().is_empty() {
        // EOF with nothing generated: the server died before the first token.
        return (
            Err("[LLM-002] The local cleanup engine returned no text.".into()),
            true,
        );
    }
    // Same reasoning-block stripper as the non-streaming path. It can only bite
    // when the model leaks a thinking preamble, which the prompt disables.
    (Ok(strip_reasoning_block(&acc)), false)
}

/// A stream that dies after generating nothing is a plain error; one that dies
/// after generating text is only a truncated sentence, and reporting `Err`
/// there would make the caller paste the same dictation a second time.
fn truncated(acc: &str, err: &str) -> (Result<String, String>, bool) {
    if acc.trim().is_empty() {
        return (Err(err.to_string()), true);
    }
    sf_log!(
        "LLM: {err} — keeping the {} characters already delivered.",
        acc.chars().count()
    );
    (Ok(acc.to_string()), true)
}

/// One SSE line. `data: [DONE]` ends the stream; every other non-`data:` line
/// (blank separator, `: comment`, `event:`) is ignored. A `data:` payload that
/// is not the JSON we expect is *skipped, not fatal*: llama-server would have to
/// emit something exotic to trigger it, and silently dropping one token beats
/// abandoning a dictation the user is already reading.
fn sse_step(line: &str) -> SseStep {
    let Some(payload) = line.trim_end_matches(['\r', '\n']).strip_prefix("data: ") else {
        return SseStep::Ignore;
    };
    if payload.trim() == "[DONE]" {
        return SseStep::Done;
    }
    match serde_json::from_str::<SseChunk>(payload) {
        Ok(chunk) => {
            let SseChunk {
                choices,
                usage,
                timings,
            } = chunk;
            if let Some(content) = choices.into_iter().next().and_then(|c| c.delta.content) {
                if !content.is_empty() {
                    return SseStep::Chunk(content);
                }
            }
            // null, missing and "" all mean "no new token". A chunk with no text
            // can still be the tail one: llama-server puts the counts there.
            if usage.is_none() && timings.is_none() {
                return SseStep::Ignore;
            }
            let mut tail = serde_json::Map::new();
            if let Some(usage) = usage {
                tail.insert("usage".to_string(), usage);
            }
            if let Some(timings) = timings {
                tail.insert("timings".to_string(), timings);
            }
            SseStep::Usage(serde_json::Value::Object(tail))
        }
        Err(_) => SseStep::Ignore,
    }
}

enum SseStep {
    Chunk(String),
    /// The final chunk's `usage`/`timings` object. Counts only — never text, so
    /// it must not reach `acc`, `on_partial`, or the cleanup output.
    Usage(serde_json::Value),
    Done,
    Ignore,
}

/// Token accounting for one request, filled from whichever of `usage` (the
/// OpenAI-compatible shape) or `timings` (llama.cpp's own) the engine sent.
/// Counts and durations only: no prompt, transcript, or output is ever stored.
#[derive(Default)]
struct TokenUsage {
    prompt: Option<u64>,
    cached: Option<u64>,
    completion: Option<u64>,
    prompt_ms: Option<f64>,
    predicted_ms: Option<f64>,
}

impl TokenUsage {
    /// Fold one tail object in. The two vocabularies are negotiated rather than
    /// guessed, because llama.cpp has moved these fields between releases.
    /// Missing values leave the previous ones alone, so a chunk carrying only
    /// half the counts cannot blank the half already seen.
    fn absorb(&mut self, tail: &serde_json::Value) {
        let usage = tail.get("usage");
        let timings = tail.get("timings");
        let prompt = count(usage.and_then(|u| u.get("prompt_tokens")))
            .or_else(|| count(timings.and_then(|t| t.get("prompt_n"))));
        let cached = count(
            usage
                .and_then(|u| u.get("prompt_tokens_details"))
                .and_then(|d| d.get("cached_tokens")),
        )
        .or_else(|| count(timings.and_then(|t| t.get("cache_n"))));
        let completion = count(usage.and_then(|u| u.get("completion_tokens")))
            .or_else(|| count(timings.and_then(|t| t.get("predicted_n"))));
        if prompt.is_some() {
            self.prompt = prompt;
        }
        if cached.is_some() {
            self.cached = cached;
        }
        if completion.is_some() {
            self.completion = completion;
        }
        if let Some(ms) = timings.and_then(|t| t.get("prompt_ms")).and_then(as_f64) {
            self.prompt_ms = Some(ms);
        }
        if let Some(ms) = timings
            .and_then(|t| t.get("predicted_ms"))
            .and_then(as_f64)
        {
            self.predicted_ms = Some(ms);
        }
    }
}

fn count(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|f| f.round() as u64))
}

fn as_f64(value: &serde_json::Value) -> Option<f64> {
    value.as_f64()
}

/// `?` for anything the engine did not report, so a gap is visible instead of
/// being logged as a misleading zero.
fn count_text(value: Option<u64>) -> String {
    match value {
        Some(n) => n.to_string(),
        None => "?".to_string(),
    }
}

/// A duration a human can read at a glance: the prefill of a warm prompt is
/// tens of milliseconds, the decode of a sentence is a second or two.
fn duration_text(value: Option<f64>) -> String {
    match value {
        Some(ms) if ms < 1000.0 => format!("{ms:.0} ms"),
        Some(ms) => format!("{:.2} s", ms / 1000.0),
        None => "?".to_string(),
    }
}

/// Exactly one line per dictation: how many prompt tokens were new, how many
/// came back out of the KV cache, and what the two phases cost in wall clock.
fn log_usage(usage: &TokenUsage) {
    sf_log!(
        "LLM: usage | prompt {} | cached {} | completion {} | prefill {} | decode {}",
        count_text(usage.prompt),
        count_text(usage.cached),
        count_text(usage.completion),
        duration_text(usage.prompt_ms),
        duration_text(usage.predicted_ms)
    );
}

#[derive(serde::Deserialize)]
struct SseChunk {
    #[serde(default)]
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
    #[serde(default)]
    timings: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: SseDelta,
}

#[derive(Default, serde::Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

/// The whole SSE body as deltas plus whether `[DONE]` was seen. Tests only —
/// the live path folds `sse_step` line by line so it can paste as it reads.
#[cfg(test)]
fn parse_sse(body: &str) -> (Vec<String>, bool) {
    let mut chunks = Vec::new();
    let mut done = false;
    // split_inclusive keeps the line terminator, exactly like BufRead::read_line.
    for line in body.split_inclusive('\n') {
        match sse_step(line) {
            SseStep::Chunk(chunk) => chunks.push(chunk),
            SseStep::Done => {
                done = true;
                break;
            }
            // The tail chunk's counts are diagnostics, not text.
            SseStep::Usage(_) => {}
            SseStep::Ignore => {}
        }
    }
    (chunks, done)
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
/// `offline::download_bundle` writes to. A non-empty `custom_llm_path` wins, so
/// a user who already owns a .gguf (e.g. from Ollama) never re-downloads 2.55 GB.
fn model_path(config: &Config) -> Result<PathBuf, String> {
    let custom = config.custom_llm_path.trim();
    if !custom.is_empty() {
        let path = PathBuf::from(custom);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(missing_model(
                &path,
                "Pick an existing .gguf in Settings, or enable cloud mode in Settings.",
            ))
        };
    }
    let filename = resolve_model_name(&config.llm_model_size)?;
    let config_dir = Config::path()
        .parent()
        .ok_or_else(|| {
            "[LLM-001] Could not determine the SmoothFlow config directory.".to_string()
        })?
        .to_path_buf();
    let root = offline::models_dir(&config_dir, &config.models_dir).join("llm");
    let path = root.join(&config.llm_model_size).join(filename);
    if !path.is_file() {
        let generic = missing_model(
            &path,
            "Download the Qwen models in Settings, or enable cloud mode in Settings.",
        );
        // A size that is installed but not the configured one is almost always a
        // Settings typo — name it instead of telling the user to re-download.
        return match *installed_sizes(&root) {
            [only] => Err(format!(
                "{generic} Cleanup Model is set to {}, and the {} model is installed — change Cleanup Model in Settings to {}.",
                config.llm_model_size, only, only
            )),
            _ => Err(generic),
        };
    }
    Ok(path)
}

/// Catalog LLM sizes that have their model file on disk under `<root>`.
/// Zero or more than one means no single actionable suggestion, so the caller
/// falls back to the generic error.
fn installed_sizes(root: &Path) -> Vec<&'static str> {
    let mut sizes: Vec<&'static str> = offline::catalog()
        .iter()
        .filter(|spec| spec.category == "llm")
        .filter(|spec| root.join(spec.directory).join(spec.filename).is_file())
        .map(|spec| spec.directory)
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn missing_model(path: &Path, hint: &str) -> String {
    format!(
        "[LLM-001] Cleanup model not found at {}. {hint}",
        path.display()
    )
}

/// The llama.cpp build is Windows-only for now; nothing to look for elsewhere.
fn platform_supported() -> bool {
    cfg!(target_os = "windows")
}

fn unsupported_platform_message() -> String {
    // Fixed string, not `consts::OS`: the user should read "macOS", not "macos".
    "[LLM-001] Offline mode is not yet available on macOS or Linux. Please use cloud mode in Settings.".to_string()
}

/// `<app dir>/llama/llama-server[.exe]`. The llama.cpp build ships as a 9 KB
/// stub next to a pile of DLLs, so the whole directory has to be installed.
fn server_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("[LLM-001] Could not locate the SmoothFlow executable: {e}"))?;
    let dir = exe.parent().ok_or_else(|| {
        "[LLM-001] Could not locate the SmoothFlow installation directory.".to_string()
    })?;
    Ok(dir
        .join("llama")
        .join(format!("llama-server{}", std::env::consts::EXE_SUFFIX)))
}

/// OpenAI-compatible body. `cache_prompt` keeps the system prompt resident
/// across dictations, which is most of the cost of a short cleanup.
fn build_body(system: String, user: String, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": "local",
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "temperature": 0,
        "max_tokens": 4096,
        "stream": stream,
        "cache_prompt": true,
        // Qwen3.5 defaults to a reasoning pass. Left on, a 30-word dictation
        // ran past 1200 tokens at ~6 tok/s and never stopped. Cleanup wants the
        // answer, not the deliberation.
        "chat_template_kwargs": { "enable_thinking": false }
    });
    if stream {
        // Diagnostics only: makes llama-server emit a final SSE chunk carrying
        // token counts, so we can see how much of the prompt the KV cache saved.
        // Counts, never content. Meaningless without a stream, hence the guard.
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    body
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
    fn a_custom_path_that_is_not_there_fails_loudly() {
        for custom in [
            "C:/definitely/not/here.gguf",
            "   C:/definitely/not/here.gguf   ",
        ] {
            let config = Config {
                custom_llm_path: custom.to_string(),
                ..Config::default()
            };
            let err = model_path(&config).unwrap_err();
            assert!(
                err.starts_with("[LLM-001]"),
                "custom {custom:?} gave: {err}"
            );
            assert!(
                err.contains("not/here.gguf"),
                "custom {custom:?} gave: {err}"
            );
        }
    }

    #[test]
    fn a_custom_path_that_exists_bypasses_the_catalog() {
        let file = std::env::temp_dir().join("smoothflow-bring-your-own.gguf");
        std::fs::write(&file, b"GGUF").unwrap();
        // A bogus llm_model_size would fail the catalog lookup, so this also
        // proves the custom path short-circuits it.
        let config = Config {
            custom_llm_path: file.to_string_lossy().into_owned(),
            llm_model_size: "12b".to_string(),
            ..Config::default()
        };
        assert_eq!(model_path(&config).unwrap(), file);
        let _ = std::fs::remove_file(&file);
    }

    /// A throwaway `<root>/llm` tree: the given sizes get their catalog file.
    fn fake_llm_root(name: &str, sizes: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("smoothflow-llm-drift-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        for size in sizes {
            let dir = root.join(size);
            std::fs::create_dir_all(&dir).unwrap();
            let filename = resolve_model_name(size).unwrap();
            std::fs::write(dir.join(filename), b"GGUF").unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn one_installed_size_is_the_only_actionable_drift_suggestion() {
        let root = fake_llm_root("one", &["4b"]);
        assert_eq!(installed_sizes(&root), vec!["4b"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_or_several_installed_sizes_leave_the_generic_error() {
        // Empty root, an empty size directory, and a stray file must not count.
        let empty = fake_llm_root("none", &[]);
        assert!(installed_sizes(&empty).is_empty());
        std::fs::create_dir_all(empty.join("2b")).unwrap();
        assert!(installed_sizes(&empty).is_empty());
        let _ = std::fs::remove_dir_all(&empty);

        let many = fake_llm_root("many", &["4b", "9b"]);
        assert_eq!(installed_sizes(&many), vec!["4b", "9b"]);
        let _ = std::fs::remove_dir_all(&many);
    }

    #[test]
    fn request_body_is_deterministic_offline_and_unauthenticated() {
        let body = build_body("sys".to_string(), "user".to_string(), true);
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["stream"], true);
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
    fn the_one_shot_path_is_the_only_body_with_stream_false() {
        // Same builder, so a change to one cannot drift from the other.
        assert_eq!(
            build_body("sys".into(), "user".into(), false)["stream"],
            false
        );
        assert_eq!(build_body("sys".into(), "user".into(), true)["stream"], true);
    }

    #[test]
    fn the_prime_body_shares_the_real_prompt_and_generates_nothing() {
        let config = Config {
            dictionary: vec!["SmoothFlow".into()],
            ..Config::default()
        };
        let body = prime_body(&config, WindowCategory::Default);
        // Byte-identical prompt construction to a real cleanup, or the KV cache
        // never hits. Dictionary included — that changes the prompt suffix.
        assert_eq!(
            body["messages"][0]["content"],
            system_prompt_offline(&config, WindowCategory::Default)
        );
        assert_eq!(body["messages"][1]["content"], user_message("", ""));
        // Prefill only: a token of generation and nothing to parse.
        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["stream"], false);
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[test]
    fn the_prime_body_covers_every_category_and_each_label_is_distinct() {
        // Each category primes its own tail, and the labels are what the
        // one-prime-per-server guard compares — a duplicate would silently skip a
        // category's prefill.
        let mut labels = Vec::new();
        for category in [
            WindowCategory::Default,
            WindowCategory::Email,
            WindowCategory::Code,
            WindowCategory::Chat,
            WindowCategory::Browser,
            WindowCategory::Document,
        ] {
            let label = category_label(category);
            assert!(!labels.contains(&label), "duplicate label {label}");
            labels.push(label);
        }
        assert_eq!(labels.len(), 6);
        // The label the prime is filed under is the category the real request
        // will ask for, so it has to come from `categorize_window` itself.
        assert_eq!(category_label(categorize_window("")), "default");
        assert_eq!(category_label(categorize_window("Inbox - Gmail")), "email");
        assert_eq!(
            category_label(categorize_window("main.rs - Visual Studio Code")),
            "code"
        );
        assert_eq!(category_label(categorize_window("Slack | general")), "chat");
        assert_eq!(category_label(categorize_window("GitHub - Chrome")), "code");
        assert_eq!(
            category_label(categorize_window("notes.md - Obsidian")),
            "document"
        );

        let config = Config::default();
        let default_prompt = prime_body(&config, WindowCategory::Default)["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_string();
        let email_prompt = prime_body(&config, WindowCategory::Email)["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(
            default_prompt, email_prompt,
            "the tail has to differ, or priming per category is pointless"
        );
    }

    fn sse_body(chunks: &[&str], tail: &str) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(chunk);
            body.push_str("\n\n");
        }
        body.push_str(tail);
        body
    }

    #[test]
    fn sse_reads_deltas_in_order_and_stops_at_done() {
        let body = sse_body(
            &[
                r#"{"choices":[{"delta":{"role":"assistant"},"index":0}]}"#,
                r#"{"choices":[{"delta":{"content":"Hello"},"index":0}]}"#,
                r#"{"choices":[{"delta":{"content":", world"},"index":0}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"total_tokens":9}}"#,
            ],
            "data: [DONE]\n\n",
        );
        let (chunks, done) = parse_sse(&body);
        assert_eq!(chunks, vec!["Hello", ", world"]);
        assert!(done);
    }

    #[test]
    fn sse_treats_null_missing_and_empty_content_as_no_new_token() {
        let body = sse_body(
            &[
                r#"{"choices":[{"delta":{"content":null},"index":0}]}"#,
                r#"{"choices":[{"delta":{},"index":0}]}"#,
                r#"{"choices":[{"delta":{"content":""},"index":0}]}"#,
                r#"{"choices":[]}"#,
                r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            ],
            "data: [DONE]\n\n",
        );
        let (chunks, done) = parse_sse(&body);
        assert_eq!(chunks, vec!["ok"]);
        assert!(done);
    }

    #[test]
    fn sse_skips_blank_comment_and_non_data_lines() {
        let body = "event: message\n: ping\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n\n\
                    id: 42\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\ndata: [DONE]\n\n";
        let (chunks, done) = parse_sse(body);
        assert_eq!(chunks, vec!["a", "b"]);
        assert!(done);
    }

    #[test]
    fn sse_survives_content_containing_newlines_and_braces() {
        // The wire form of a newline is the two bytes \ and n, so a JSON payload
        // can never contain a raw 0x0A: BufReader::read_line splits exactly at
        // frame boundaries and the payload arrives in one piece. This asserts
        // both halves of that claim — no raw newline inside the data line, and
        // the escapes decoded into real newlines after parsing.
        let payload = r#"{"choices":[{"delta":{"content":"Hi Dana,\n\nThanks"}}]}"#;
        let line = format!("data: {payload}");
        assert_eq!(line.bytes().filter(|b| *b == b'\n').count(), 0);

        let braces = r#"{"choices":[{"delta":{"content":" {\"quoted\"} [a] 50% <tag>"}}]}"#;
        let body = sse_body(&[payload, braces], "data: [DONE]\n\n");
        let (chunks, done) = parse_sse(&body);
        assert_eq!(chunks, vec!["Hi Dana,\n\nThanks", " {\"quoted\"} [a] 50% <tag>"]);
        // One frame per source line: nothing was split or glued together.
        let frames = body.lines().filter(|l| l.starts_with("data: ")).count();
        assert_eq!(chunks.len(), frames - 1);
        assert!(done);
    }

    #[test]
    fn sse_skips_non_json_data_lines_instead_of_failing() {
        // llama-server would have to emit something exotic to trigger this, and
        // losing one token beats throwing away a dictation already on screen.
        let body = sse_body(
            &[
                "not json at all",
                "",
                "{unbalanced",
                r#"{"choices":[{"delta":{"content":"kept"}}]}"#,
            ],
            "data: [DONE]\n\n",
        );
        let (chunks, done) = parse_sse(&body);
        assert_eq!(chunks, vec!["kept"]);
        assert!(done);
    }

    #[test]
    fn sse_without_a_done_marker_still_yields_what_arrived() {
        // Connection dropped right after the last token: no [DONE], no error
        // from the parser. `request_streaming` decides that this is enough.
        let (chunks, done) = parse_sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"half a se\"}}]}\n\n",
        );
        assert_eq!(chunks, vec!["half a se"]);
        assert!(!done);
        assert_eq!(parse_sse("").0, Vec::<String>::new());
    }

    #[test]
    fn truncated_stream_errors_before_any_text_and_keeps_it_afterwards() {
        let (err, kill) = truncated("", "[LLM-002] The local cleanup stream failed.");
        assert!(err.unwrap_err().starts_with("[LLM-002]"));
        assert!(kill);

        let (kept, kill) = truncated("half a sen", "[LLM-002] The local cleanup stream failed.");
        assert_eq!(kept.unwrap(), "half a sen");
        assert!(kill);
        // Whitespace-only output is not a sentence the user can use.
        let (err, _) = truncated("  \n", "[LLM-002] The local cleanup stream failed.");
        assert!(err.is_err());
    }

    #[test]
    fn sse_reads_the_final_usage_chunk_as_token_counts_only() {
        // The tail chunk llama-server sends for `stream_options.include_usage`:
        // no content, both vocabularies present. It must yield counts and never
        // text, or the counts would end up pasted into the document.
        let mut usage = TokenUsage::default();
        let tail = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1103,\"completion_tokens\":12,\"prompt_tokens_details\":{\"cached_tokens\":1024}},\"timings\":{\"prompt_n\":1103,\"cache_n\":1024,\"predicted_n\":12,\"prompt_ms\":200.0,\"predicted_ms\":1500.0}}\n";
        match sse_step(tail) {
            SseStep::Usage(value) => usage.absorb(&value),
            _ => panic!("a content-free tail chunk must be recognised as usage"),
        }
        assert_eq!(count_text(usage.prompt), "1103");
        assert_eq!(count_text(usage.cached), "1024");
        assert_eq!(count_text(usage.completion), "12");
        assert_eq!(duration_text(usage.prompt_ms), "200 ms");
        assert_eq!(duration_text(usage.predicted_ms), "1.50 s");

        // Older/other builds only ship `timings`. Same three counts, no panic.
        let timings_only = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"timings\":{\"prompt_n\":900,\"cache_n\":0,\"predicted_n\":7}}\n";
        match sse_step(timings_only) {
            SseStep::Usage(value) => usage.absorb(&value),
            _ => panic!("a timings-only tail chunk must be recognised as usage"),
        }
        assert_eq!(count_text(usage.prompt), "900");
        assert_eq!(count_text(usage.cached), "0");
        assert_eq!(count_text(usage.completion), "7");

        // Nothing reported at all stays a gap, not a misleading zero.
        let empty = TokenUsage::default();
        assert_eq!(count_text(empty.prompt), "?");
        assert_eq!(duration_text(empty.prompt_ms), "?");
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
    fn no_server_is_started_after_shutdown_was_requested() {
        use std::sync::atomic::Ordering;
        // Pre-existing state, because another test in this binary calls
        // shutdown() and the flag is process-wide.
        let was = SHUTTING_DOWN.load(Ordering::Relaxed);
        SHUTTING_DOWN.store(true, Ordering::Relaxed);
        // Paths that could never spawn: a live gate must not even reach the
        // filesystem checks behind it.
        let err = match spawn(&PathBuf::from("no-server"), &PathBuf::from("no-model")) {
            Err(err) => err,
            Ok(_) => panic!("a server was started after shutdown"),
        };
        SHUTTING_DOWN.store(was, Ordering::Relaxed);
        assert!(err.starts_with("[LLM-002]"), "got: {err}");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn an_unsupported_platform_says_so_instead_of_missing_the_engine() {
        assert!(!platform_supported());
        assert!(unsupported_platform_message().starts_with("[LLM-001]"));
        assert!(unsupported_platform_message().contains("macOS or Linux"));
    }

    #[test]
    fn free_port_returns_a_usable_loopback_port() {
        let port = free_port().unwrap();
        assert!(port > 0);
    }
}
