# SmoothFlow — agents guide

## Project
Cross-platform voice-to-text dictation app. Tauri 2.0 shell, Rust core, vanilla HTML/CSS/JS frontend.

**Positioning:** SmoothFlow is being built as a head-to-head alternative to Wispr Flow. Feature and UX decisions should be judged against that bar — if a change makes SmoothFlow feel slower, clunkier, or less capable than Wispr Flow for the same action (start/stop dictation, accuracy, injection into any app, low friction config), flag it rather than shipping it quietly. Built with [OpenCode](https://opencode.ai).

## Workflow rules (read this before doing anything)

**One feature/fix at a time.** Don't start the next item until the current one compiles, has been checked against the constraints below, and has been reported (see "Report after every change"). If a request bundles multiple features, say so and propose an order instead of doing them all at once.

**Ask before assuming.** Stop and ask when:
- A requirement is ambiguous (which config field, what the UI copy should say, error-handling behavior).
- A change would touch the constraints in `Key constraints` below (Send-safety on `AudioRecorder`, the whisper-rs removal, MinGW/env-var setup, icon generation).
- There's more than one reasonable way to implement something and the choice affects the user (e.g. blocking vs. async, silent fallback vs. visible error).

Don't guess and move on — a wrong guess costs more than the question.

**Delegate implementation to a subagent.** Keep the primary conversation for planning, review, and talking to me. Use OpenCode's built-in `plan` agent (or just ask, in `build`) to scope the feature and confirm anything ambiguous *before* touching code — `plan` can't edit files, which forces the confirmation step instead of skipping it. Once scoped, hand the actual implementation to a project subagent (defined below in `.opencode/agent/`):
1. Restate the plan for the feature in 2–3 lines and confirm/ask before dispatching.
2. Dispatch the subagent scoped to *only* that feature, either by @-mentioning it (`@rust-feature-builder ...`) or letting `build` auto-select it from its description. Its prompt must include: the exact files it's allowed to touch, the relevant section of `Key constraints`, and the "definition of done" below — subagents start with a clean context and only know what's in the prompt you give them.
3. The subagent implements, runs `cargo check --target-dir /tmp/sf-target` (or the frontend equivalent) itself, and fixes failures before returning.
4. Review its diff against the constraints before reporting back to me.

**Definition of done** for any feature/fix, checked before it's reported complete:
- `cargo check --target-dir /tmp/sf-target` is clean (or `npm run tauri dev` / `tauri build` if the change is frontend-only).
- `AudioRecorder`'s `unsafe impl Send` invariant still holds (stream stopped before drop).
- Required env vars (`CC`, `CXX`, `LIBCLANG_PATH`) aren't hardcoded anywhere new — assume they're already set in the shell.
- Tauri command signatures in the table below are updated if changed.

**Report after every change**, in full, not just a diff dump:
- **What changed** — file by file, one line of purpose each.
- **Why** — the decision made and any trade-off, especially if you picked one option after asking (or because asking wasn't warranted).
- **How to verify** — the exact command to run and what a pass looks like.
- **Follow-ups / known gaps** — anything deferred, stubbed, or worth revisiting.

## Subagents (`.opencode/agent/`)

OpenCode subagents are markdown files where the **filename becomes the agent name**. Create these two (ask me to generate them if they don't exist, or write directly):

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
Respect: AudioRecorder's unsafe Send impl (stream must stop before drop), no whisper-rs,
MinGW/libclang env vars are already set in the shell (never hardcode them in code).
Return: files touched + one-line purpose each, and the exact verify command to run.
```

`.opencode/agent/frontend-builder.md`
```markdown
---
description: Implements one SmoothFlow feature or fix in index.html / public/app.js. Use for any frontend-only change (no framework, vanilla JS).
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
You implement exactly one scoped feature/fix in the SmoothFlow frontend (index.html, public/app.js).
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

# Rust check-only (faster)
cargo check --target-dir /tmp/sf-target
```

**Prerequisites (Windows):** `x86_64-pc-windows-gnu` Rust toolchain + MinGW-w64 GCC (at `D:\Downloads\mingw64\bin`) + LLVM (for libclang, bindgen needs `LIBCLANG_PATH`).

**Env vars required for Rust compilation:**
```
CC=D:\Downloads\mingw64\bin\gcc.exe
CXX=D:\Downloads\mingw64\bin\g++.exe
LIBCLANG_PATH=C:\Program Files\LLVM\bin
```

## Architecture

```
index.html + public/app.js  ← Vite dev server (:1420)
         ↕ @tauri-apps/api invoke()
src-tauri/src/
  lib.rs           — Tauri commands: start_recording, stop_recording, get/set config
  main.rs          — entry, calls smoothflow_lib::run()
  audio.rs         — cpal mic capture → Vec<f32>
  transcription.rs — configurable endpoint (Groq, OpenAI, xAI, etc.)
  text_injection.rs — enigo keystroke simulation into active window
  config.rs        — serde Config struct, JSON file I/O
  postprocess.rs   — filler removal + auto-punctuation
```

## Key constraints

- `cpal::Stream` is `!Send` on Windows → `AudioRecorder` has `unsafe impl Send` (stream stopped before drop)
- Cloud-only transcription. No local/offline STT.
- Icons: Windows resource build requires valid `.ico`. Generate via `IcoMaker` C# script in `icons/`.
- Frontend is static, no framework. Vite dev server runs on port 1420.

## Tauri commands (invoke from JS)

| Command | Args | Returns |
|---------|------|---------|
| `start_recording` | none | `Result<(), String>` |
| `stop_recording` | none | `Result<String, String>` (transcribed text) |
| `get_config` | none | `Config` |
| `update_config` | `new_config: Config` | `Result<(), String>` |

## Config (`smoothflow.json`)

```rust
struct Config {
    api_base_url: String,       // e.g. https://api.groq.com/openai/v1
    api_key: String,
    auto_punctuation: bool,
    remove_fillers: bool,
    launch_on_startup: bool,
}
```

## File locking on Windows

`cargo build` frequently hits "file in use (os error 32)". Use `--target-dir` with a fresh temp path per build.
