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
