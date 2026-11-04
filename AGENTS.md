# SmoothFlow — agents guide

## Project
Cross-platform voice-to-text dictation app. Tauri 2.0 shell, Rust core, vanilla HTML/CSS/JS frontend.

**Positioning:** SmoothFlow is being built as a head-to-head alternative to Wispr Flow. Feature and UX decisions should be judged against that bar — if a change makes SmoothFlow feel slower, clunkier, or less capable than Wispr Flow for the same action (start/stop dictation, accuracy, injection into any app, low friction config), flag it rather than shipping it quietly. Built with [OpenCode](https://opencode.ai).

## Workflow rules (read this before doing anything)

**One feature/fix at a time.** Don't start the next item until the current one compiles, has been checked against the constraints below, and has been reported (see "Report after every change"). If a request bundles multiple features, say so and propose an order instead of doing them all at once.

**Ask before assuming.** Stop and ask when:
- A requirement is ambiguous (which config field, what the UI copy should say, error-handling behavior).
- A change would touch the constraints in `Key constraints` below (Send-safety on `AudioRecorder`, the offline engine/dependency choice, MinGW/env-var setup, icon generation).
- There's more than one reasonable way to implement something and the choice affects the user (e.g. blocking vs. async, silent fallback vs. visible error).

Don't guess and move on — a wrong guess costs more than the question.

**Delegate implementation to a subagent.** Keep the primary conversation for planning, review, and talking to me. Use OpenCode's built-in `plan` agent (or just ask, in `build`) to scope the feature and confirm anything ambiguous *before* touching code — `plan` can't edit files, which forces the confirmation step instead of skipping it. Once scoped, hand the actual implementation to a project subagent (defined below in `.opencode/agent/`):
1. Restate the plan for the feature in 2–3 lines and confirm/ask before dispatching.
2. Dispatch the subagent scoped to *only* that feature, either by @-mentioning it (`@rust-feature-builder ...`) or letting `build` auto-select it from its description. Its prompt must include: the exact files it's allowed to touch, the relevant section of `Key constraints`, and the "definition of done" below — subagents start with a clean context and only know what's in the prompt you give them.
3. The subagent implements, runs `cargo check --target-dir /tmp/sf-target` from `src-tauri/` (or the frontend equivalent) itself, and fixes failures before returning.
4. Review its diff against the constraints before reporting back to me.

**Definition of done** for any feature/fix, checked before it's reported complete:
- `cargo check --target-dir /tmp/sf-target` (from `src-tauri/`) is clean, or `npm run tauri dev` / `tauri build` if the change is frontend-only.
- `AudioRecorder`'s `unsafe impl Send` invariant still holds (stream stopped before drop).
- Required env vars (`CC`, `CXX`, `LIBCLANG_PATH`) aren't hardcoded anywhere new — assume they're already set in the shell.
- Tauri command signatures in the table below are updated if changed.

**Report after every change**, in full, not just a diff dump:
- **What changed** — file by file, one line of purpose each.
- **Why** — the decision made and any trade-off, especially if you picked one option after asking (or because asking wasn't warranted).
- **How to verify** — the exact command to run and what a pass looks like.
- **Follow-ups / known gaps** — anything deferred, stubbed, or worth revisiting.

## Subagents (`.opencode/agent/`)

OpenCode subagents are markdown files where the **filename becomes the agent name**. Both already exist in `.opencode/agent/`:

`.opencode/agent/rust-feature-builder.md`
```markdown
---
description: Implements one SmoothFlow feature or fix in src-tauri/src (Rust side). Use for any Rust-only change. Runs cargo check and fixes failures before returning.
mode: subagent
tools:
  write: true
  edit: true
  bash: true
permission:
  edit: allow
  bash:
    "cargo check *": allow
    "*": ask
---
You implement exactly one scoped feature/fix in the SmoothFlow Rust codebase (src-tauri/src).
Do not touch files outside what the prompt lists.
Before finishing: run `cargo check --target-dir /tmp/sf-target` and fix any errors.
Respect: AudioRecorder's unsafe Send impl (stream must stop before drop).
Engine/dependency choices (e.g. a local STT engine or native crate) must be confirmed with me
based on hardware, accuracy, latency, and build compatibility — don't pick one unilaterally.
MinGW/libclang env vars are already set in the shell (never hardcode them in code).
Return: files touched + one-line purpose each, and the exact verify command to run.
```

`.opencode/agent/frontend-builder.md`
```markdown
---
description: Implements one SmoothFlow feature or fix in index.html / app.js. Use for any frontend-only change (no framework, vanilla JS).
mode: subagent
tools:
  write: true
  edit: true
  bash: true
permission:
  edit: allow
  bash:
    "npm run tauri dev*": allow
    "*": ask
---
You implement exactly one scoped feature/fix in the SmoothFlow frontend (index.html, app.js).
No framework — vanilla JS/HTML/CSS only. Calls into Rust go through @tauri-apps/api invoke()
using the commands in the Tauri commands table in AGENTS.md — don't invent new command names
without flagging it back to the main conversation.
Return: files touched + one-line purpose each, and how to verify (npm run tauri dev + what to click/check).
```

Invoke manually with `@rust-feature-builder` / `@frontend-builder` in a message, or let the primary agent (`build`) auto-select one based on its description. Use the built-in `plan` primary agent first for anything that needs scoping or a decision from me — it's read-only by default, so it can't jump ahead and start editing before you've confirmed the approach.

## Build & dev

```bash
# dev (vite on :1420 + tauri window)
npm run tauri dev

# production build
npm run tauri build

# Rust check-only (faster) — run from src-tauri/ (cargo needs the manifest)
cargo check --target-dir /tmp/sf-target
```

`npm run tauri dev` is the frontend+Rust combined dev loop. For Rust-only work, `cargo check` in `src-tauri/` is much faster. The `--target-dir /tmp/sf-target` avoids Windows file-lock issues (see below).

**Prerequisites (Windows):** `x86_64-pc-windows-gnu` Rust toolchain + MinGW-w64 GCC (at `D:\Downloads\mingw64\bin`) + LLVM (for libclang, bindgen needs `LIBCLANG_PATH`).

**Env vars required for Rust compilation:**
```
CC=D:\Downloads\mingw64\bin\gcc.exe
CXX=D:\Downloads\mingw64\bin\g++.exe
LIBCLANG_PATH=C:\Program Files\LLVM\bin
```

## Architecture

```
index.html + app.js          ← Vite dev server (:1420), main window UI (root of repo)
public/overlay.html          ← floating recording pill window
         ↕ @tauri-apps/api invoke()
src-tauri/src/
  lib.rs           — Tauri commands + global hotkey, tray, overlay, VU meter
  main.rs          — entry, calls smoothflow_lib::run()
  audio.rs         — cpal mic capture → Vec<f32>
  transcription.rs — configurable endpoint (Groq, OpenAI, xAI, etc.)
  text_injection.rs — enigo keystroke simulation into active window
  config.rs        — serde Config struct, JSON file I/O, OS vault for API key
  history.rs       — dictation history persistence
  postprocess.rs   — filler removal, auto-punctuation, self-correction resolution, spoken-email conversion
  accessibility.rs — macOS-only Accessibility permission prompt (no-op elsewhere)
  hotkey.rs       — macOS-only CGEventTap backend for the bare-Fn hotkey (no-op elsewhere)
  offline.rs       — NEW: model manager, download, verification, safe paths (offline mode)
  stt_local.rs     — sherpa-onnx child-process wrapper for local STT (Parakeet TDT 0.6B) + prewarm
  llm_local.rs     — llama.cpp server manager + local cleanup client (streaming + prompt-cache prime)
  text_rules.rs    — deterministic pre/post-processing rules (ported from JezFlow)
  wav.rs           — std-only 16-bit mono WAV writer for optional audio clips
src-tauri/binaries/win-x64/  — COMMITTED, ~44 MB: smoothflow-stt.exe + llama/ (12 files).
                      Bundled via bundle.resources in tauri.conf.json, flattened into the
                      exe dir. Both stt_local::helper_path and llm_local::server_path
                      resolve there. scripts/sync-engines.mjs mirrors it into
                      target/debug so dev and release cannot drift. Never ship only
                      llama-server.exe — it is a 9 KB stub.
                      llama/ is trimmed to the llama-server runtime only (server exe +
                      its import closure + 3 ggml-cpu tiers). Re-trim with
                      scripts/package-llama-engine.mjs rather than copying a raw
                      upstream archive — the untrimmed tree is 47 MB of CLIs and
                      arch variants that are never executed.
native/smoothflow-stt/      — C++ source for the helper (CMake + main.cpp). Builds on
                      Windows (MSVC) and, since the CMakeLists now globs lib/*.a and
                      guards the /MT runtime flag, on Linux/macOS via CI. The spawn
                      paths still return a clear "not available on macOS or Linux"
                      error until the download path lands.
.github/workflows/engines.yml — builds/packages both engines per platform and
                      publishes them to the fixed prerelease tag `engines`
                      (smoothflow-stt-<target>.zip, smoothflow-llama-<target>.zip,
                      engines.json). This is the supply side for the planned
                      ship-nothing installer; the app does not download from it yet.
```

Frontend is in the repo root (`index.html`, `app.js`, `public/`), not under `src-tauri/`.
Vite serves root files; the overlay window loads `public/overlay.html`.

## Key constraints

- `cpal::Stream` is `!Send` on Windows → `AudioRecorder` has `unsafe impl Send` (stream stopped before drop)
- Existing cloud transcription stays supported (Groq/OpenAI/xAI-style endpoints via `transcription.rs`). The cloud phase is complete for roadmap purposes, not a claim that it is bug-free; further cloud work is deferred. Next is planning, then approved implementation of an **explicit offline privacy mode**, NOT implemented yet — do not claim it exists or ship half of it silently.
  Offline contract (binding once built): audio/transcripts/context are processed on-device; no cloud STT, no cloud LLM cleanup, and no automatic cloud fallback; a missing model or engine failure surfaces a visible local error; model downloads happen only as an explicit user action, separate from offline operation. Offline planning must also cover the updater, API-key validation (`test_api_connection`), background network requests, and history/log retention privacy.
- Icons: Windows resource build requires valid `.ico`. Generate via `IcoMaker` C# script in `icons/`.
- Frontend is static, no framework. Vite dev server runs on port 1420.
- macOS: keyboard injection needs Accessibility permission (see `accessibility.rs`). Default hotkey is `Ctrl+Space` on ALL platforms (macOS included — Fn/Globe is reserved by macOS for the emoji picker). A user who explicitly picks bare `Fn` gets the CGEventTap backend in `hotkey.rs` (needs Accessibility permission, same prompt); the tap masks FlagsChanged because hardware Fn emits those, not KeyDown/KeyUp. Enabled `macos-private-api` in `Cargo.toml` + `tauri.conf.json` for the transparent overlay.
- Config/history live in `%APPDATA%\SmoothFlow` on Windows, `~/SmoothFlow` (home dir) on macOS/Linux — see `config.rs` `config_dir()`.
- CI (`src-tauri/../.github/workflows/build.yml`) runs `cargo check`, `cargo test`, and `tauri build --no-bundle` on `ubuntu-latest` + `macos-latest`; a `release` job on `windows-latest` builds the NSIS installer + updater manifest and publishes a GitHub Release when a `v*` tag is pushed. Local Windows dev covers day-to-day builds.

## Offline-First Architecture Plan (Approved)

### Technology Stack (Locked)

| Component | Choice | Version | Why |
|-----------|--------|---------|-----|
| **STT Engine** | Custom native `smoothflow-stt` helper using `sherpa-onnx` C++ API; child process | 1.13.8+ | Parakeet TDT 0.6B ONNX int8, CPU real-time, Apache 2.0, cross-platform process boundary; no new Rust FFI dependency |
| **LLM Cleanup** | `llama.cpp` server (bundled binary) | b4400+ | Qwen3.5 GGUF Q4_K_M, OpenAI-compatible, `--spec-type ngram-mod`, CPU/Metal/CUDA |
| **Audio Capture** | `cpal` (current) | — | Already works, keep |
| **Model Download** | `reqwest` + `sha256` | — | User-initiated only, resume support, explicit consent |

**No new Rust FFI dependencies for inference** — both engines run as child processes. `llama-server` exposes localhost HTTP; the custom Sherpa helper is a separately built native executable. This keeps the `AudioRecorder` `unsafe impl Send` invariant untouched.

### Models & Sizes (Privacy-First Defaults)

| Model | Format | Size | RAM (Runtime) | Accuracy Tier |
|-------|--------|------|---------------|---------------|
| Parakeet TDT 0.6B v2 (EN) | ONNX int8 | 661.19 MB / 630.56 MiB | Benchmark required | Best English |
| Parakeet TDT 0.6B v3 (25 lang) | ONNX int8 | 670.48 MB / 639.42 MiB | Benchmark required | Multilingual |
| Qwen3.5-4B-Instruct | GGUF Q4_K_M | 2,740,937,888 bytes | Benchmark required | Fast cleanup (default) |
| Qwen3.5-2B-Instruct | GGUF Q4_K_M | 1,280,835,840 bytes | Benchmark required | Weak devices |
| Qwen3.5-9B-Instruct | GGUF Q4_K_M | 5,680,522,464 bytes | Benchmark required | Optional high-quality tier |

**Total default bundle (v2 EN + 4B):** ~3.4 GB download.  
**Minimal bundle (v2 EN + 2B):** ~1.94 GB download.  
The 120 MB Parakeet estimates in earlier drafts were incorrect; the verified INT8 encoder plus decoder and vocabulary is approximately 630–640 MiB.

**No model ships in installer.** User clicks "Enable Offline Mode" → downloads to config dir/models/. Explicit consent, visible progress, resume support.

### Hardware Targets

| Tier | CPU | RAM | STT (15s) | LLM Cleanup (60w) | Notes |
|------|-----|-----|-----------|-------------------|-------|
| Minimum | 4-core x64/ARM64 2.5 GHz+ | 8 GB | Benchmark required | Qwen 2B target | 2B is the constrained-device default |
| Recommended | 6+ core, AVX2/NEON | 8–16 GB | Benchmark required | Qwen 4B target | Modern laptop/desktop; proposed default |
| Cloud Parity | 8+ core, AVX-512/AMX | 16 GB | Benchmark required | Qwen 4B/9B | Qwen 9B remains opt-in |

**No GPU required.** Metal/CUDA optional (auto-detected by llama.cpp).

### Offline Contract (Binding, Enforced in Code)

1. `offline_mode: true` → **zero automatic network requests**. STT/LLM never use cloud services; no silent cloud fallback.
2. Model download only via `download_models` (explicit user action). No background fetch or remote catalog refresh.
3. `test_api_connection` and updater checks are never automatic in offline mode; they require an explicit user action and are clearly identified as network exceptions.
4. Remote fonts must be bundled locally or removed before claiming strict offline behavior.
5. Missing model / engine crash → visible error in overlay + toast.
6. Audio/transcripts/context never leave device in offline mode. History is local; raw transcripts and audio clips remain disabled by default.

### Implementation Phases

| Phase | Scope | Deliverable | Files |
|-------|-------|-------------|-------|
| **1** | TextRules port (deterministic layer) | 15 tests pass (14 conformance cases + faithfulness guard) | `text_rules.rs`, `postprocess.rs` |
| **2** | Local STT integration | Offline transcription works | `stt_local.rs`, `offline.rs`, `transcription.rs`, `config.rs` |
| **3** | Local LLM cleanup | Offline cleanup works | `llm_local.rs`, `offline.rs`, `postprocess.rs` |
| **4** | Polish & Learning | Fast path, corrections, eval | `text_rules.rs`, `postprocess.rs`, `history.rs`, `config.rs` |
| **5** | Live text & Field context | Streaming partials, cursor context | `stt_local.rs`, `lib.rs`, `overlay.html`, `app.js` |
| **6** | Meeting notes (optional) | Dual-channel, structured notes | New files |

### New Tauri Commands

| Command | Args | Returns |
|---------|------|---------|
| `get_offline_status` | none | `{ models_installed, stt_ready, llm_ready, versions }` |
| `download_models` | `{ stt_version: "v2"\|"v3", llm_size: "2b"\|"4b"\|"9b" }` | Streaming progress / `Result<(), String>` |
| `delete_models` | none | `Result<(), String>` |
| `test_local_pipeline` | `{ text: String }` | `Result<String, String>` |

### Config Additions (`smoothflow.json`)

```rust
struct Config {
    // ... existing fields ...
    offline_mode: bool,              // false = cloud, true = local only
    stt_version: String,             // "v2" (EN) | "v3" (25 lang)
    llm_model_size: String,          // "2b" | "4b" | "9b"
    models_dir: String,              // custom path, empty = default
    auto_download_models: bool,      // false: explicit user action only
    casual_fast_path_words: usize,   // default 40, skip LLM for short input (offline only)
    keep_audio_clips: bool,          // for --eval regression testing
    custom_llm_path: String,         // user's own .gguf (e.g. from Ollama); empty = catalog
}
```

**Implementation note (2026-10-03):** offline is functional on Windows and verified end-to-end. `get_offline_status` / `download_models` / `delete_models` / `test_local_pipeline` / `get_models_dir` are registered in `lib.rs`. The Windows engines ship inside the installer (`src-tauri/binaries/win-x64/`, verified against a real NSIS install). Local STT runs at ~0.19 s per second of audio; local cleanup is generation-bound and streams, so the first word lands ~1.2 s after release. macOS and Linux have no engines yet and both spawn paths return a clear "not available, use cloud mode" error. `cargo test` still cannot run on this Windows box (the test binary fails to load, `0xc0000139`; CI on ubuntu/macos is the only working test path), so new unit tests are compile-verified only. **Read `PORT-PLAN.md` for measured latencies, dead ends and the ranked open problems.** Read **`BENCHMARK.md`** for public-corpus accuracy/speed numbers and the comparison against published leaderboard figures (produced by `bench/`, reproducible, no private audio). `plan.md` is stale — its only actionable section shipped in `5c42a54`.

**Two Tauri 2 traps, both learned the hard way:**
- A **synchronous** `#[tauri::command]` runs **on the main thread**. `tauri-macros/src/command/wrapper.rs` `body_blocking` generates a direct call with no `spawn`. A multi-GB model download froze the entire app. Six heavy commands are now `async fn` + `tauri::async_runtime::spawn_blocking`: `stop_recording`, `inject_text`, `test_api_connection`, `download_models`, `delete_models`, `test_local_pipeline`. Do not "fix" this with `#[tauri::command(async)]` on a sync fn — that only moves blocking onto a tokio worker.
- Anything that blocks for more than a few hundred ms belongs in `spawn_blocking`. Microsecond commands (`get_config`, `get_history`, `get_offline_status`) are fine as sync.

## Tauri commands (invoke from JS)

| Command | Args | Returns |
|---------|------|---------|
| `start_recording` | none | `Result<(), String>` |
| `stop_recording` | none | `Result<String, String>` (transcribed text) |
| `cancel_transcription` | none | `bool` — true when a run was actually in flight. Aborts the current transcription (no history row, no paste, no error); the hotkey does the same when pressed during a run |
| `transcribe_audio_file` | `path: String` | `Result<HistoryEntry, String>` — "Transcribe file": decodes a user-picked **WAV** (8/16/24/32-bit int or 32-bit float, any channel count, ≤30 min) with `wav::read_wav_mono`, transcribes it with the configured engine, cleans it up with no field context, and writes a History row whose `source_name` is the file's name and whose `audio_file` is `None`. Never pastes; never copies the file (no playback, no ↻ retry) |
| `get_config` | none | `Config` |
| `update_config` | `new_config: Config` | `Result<(), String>` |
| `get_history` | `cursor: Option<i64>, limit: Option<usize>` | `Result<HistoryPage, String>` — `{ entries: Vec<HistoryEntry>, has_more: bool }`, newest first (`id DESC`), `limit` capped at 100, `None` = all |
| `get_history_stats` | none | `Result<HistoryStats, String>` — `{ total_dictations, total_words, today_dictations, today_words }`, derived from the table |
| `delete_history_entry` | `id: i64` | `Result<(), String>` — emits `history-updated` `{action:"deleted",id}` |
| `toggle_history_saved` | `id: i64` | `Result<(), String>` — emits `history-updated` `{action:"toggled",id,saved}` |
| `clear_history` | none | `Result<(), String>` — deletes all rows, emits `history-updated` `{action:"cleared"}` |
| `reset_stats` | none | `Result<(), String>` — stats are derived, so this deletes all rows too |
| `get_audio_file_path` | `file_name: String` | `Result<String, String>` — absolute path of a stored clip (`config_dir()/recordings/<file_name>`), for the History player via `convertFileSrc`; rejects `/`, `\`, `..`, `:` and empty names |
| `inject_text` | `text: String` | `Result<(), String>` |
| `test_api_connection` | `base_url: String, api_key: String` | `Result<(), String>` |
| `get_offline_status` | none | `ModelStatus` |
| `get_models_dir` | none | `ModelsLocation` — `{ resolved: String (absolute path models are read from), is_custom: bool (true when `models_dir` overrides the default) }` |
| `download_models` | `{ stt_version: String, llm_size: String }` | `Result<DownloadProgress, String>` |
| `delete_models` | none | `Result<(), String>` |
| `test_local_pipeline` | `{ text: String }` | `Result<String, String>` |

## Config (`smoothflow.json`)

```rust
struct Config {
    api_base_url: String,
    api_key: String,
    model: String,
    cleanup_model: String,
    cleanup_fallback_model: String,
    auto_punctuation: bool,
    remove_fillers: bool,
    auto_paste: bool,
    launch_on_startup: bool,
    dictionary: Vec<String>,
    hotkey: String,
    overlay_position: String,
    // Offline fields:
    offline_mode: bool,
    stt_version: String,
    llm_model_size: String,
    models_dir: String,
    auto_download_models: bool,
    casual_fast_path_words: usize,
    keep_audio_clips: bool,
}
```

## File locking on Windows

`cargo build` frequently hits "file in use (os error 32)". Use `--target-dir` with a fresh temp path per build.

## Current status (2026-08-12)

All cross-platform fixes are committed and pushed to `origin/main`:

- **accessibility.rs** (new) — macOS Accessibility permission: `is_trusted()`, one-shot `request_once()` (fires `CGRequestPostEventAccess`), `open_settings()` deep link. No-ops on Windows/Linux. Hooks: `start_recording` command, hotkey `Pressed`, and a guard at the top of `type_text`.
- **config.rs / history.rs** — `APPDATA` fallback now uses `home_dir().join("SmoothFlow")` instead of `current_dir()` (was writing to cwd on macOS/Linux).
- **config.rs** — default hotkey is `Ctrl+Space` on Windows/Linux (macOS keeps bare `Fn`); `Alt+Space`/`Win+Space` are reserved by Windows and unregisterable, so they're not defaults.
- **text_injection.rs** — paste combo is platform-aware: `Cmd+V` on macOS, `Ctrl+V` elsewhere. Also has the macOS Accessibility guard.
- **Cargo.toml / tauri.conf.json** — `macos-private-api` feature + `macOSPrivateApi: true` (transparent overlay on macOS).

Known gap: `bundle.resources: ["WebView2Loader.dll"]` was reverted in `716e140` — the `.dll` still ships in `src-tauri/` and is gitignored, but it's no longer declared in `tauri.conf.json` bundle resources. If the NSIS/MSI "WebView2Loader.dll not found" crash reappears, re-add it.

## Anchor Commit

**Rollback point if anything breaks:** `e81f003` — offline verified working on a real Windows install (engines bundled, `get_models_dir`, bring-your-own `.gguf`). The ~1,100 lines of pre-warm, streaming cleanup, casual fast path and async-command work on top of it are still **uncommitted**; see `PORT-PLAN.md` "State".