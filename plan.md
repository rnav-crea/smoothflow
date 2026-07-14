# SmoothFlow — Plan

## Vision
Cross-platform voice-to-text dictation app. Alternative to Wispr Flow/Wisperflow with both **local** (privacy-first) and **cloud** (max accuracy) transcription. Free & open source.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Core engine | **Rust** (audio capture, transcription, system APIs) |
| Desktop shell | **Tauri 2.0** (minimal webview window + system tray) |
| UI | **HTML + CSS + vanilla JS** (lightweight, no framework) |
| Local STT | **whisper.cpp** via `whisper-rs` Rust bindings |
| Cloud STT | **OpenAI Whisper API** (optional, user's own key) |
| Audio capture | `cpal` crate (mic input) |
| Hotkey | `tauri-plugin-global-shortcut` |
| Text injection | `enigo` crate (simulate keystrokes into any app) |

**One language (Rust) for all core logic.** Simple `cargo tauri build` per platform.

## Requirements Check

| Requirement | Solution | Status |
|-------------|----------|--------|
| System-wide hotkey | `tauri-plugin-global-shortcut` | ✅ |
| Mic audio capture | `cpal` crate | ✅ |
| Local transcription (offline) | `whisper-rs` → whisper.cpp | ✅ |
| Cloud transcription | HTTP call to Whisper API via `reqwest` | ✅ |
| Type text into any app | `enigo` crate (keyboard simulation) | ✅ |
| Recording indicator UI | Tauri webview overlay / system tray icon | ✅ |
| Auto-punctuation / filler removal | Post-processing in Rust | ✅ |
| Custom dictionary | JSON config file, loaded by Rust | ✅ |
| Cross-platform build | `cargo tauri build` → .exe / .dmg / .AppImage | ✅ |

## Features (by priority)

### P0 — MVP (learn Rust basics + core loop)
- [ ] System-wide hotkey (double-tap to start/stop recording)
- [ ] Audio recording from mic → save to temp WAV
- [ ] Local transcription via whisper.cpp
- [ ] Inject transcribed text into active text field
- [ ] System tray icon (running/recording indicator)

### P1 — Core
- [ ] Cloud transcription option (user can switch in config)
- [ ] Auto-punctuation & filler word removal
- [ ] Settings window (webview UI)
- [ ] Custom dictionary (user adds words)
- [ ] Personal snippets (voice shortcuts)

### P2 — Polished
- [ ] Multi-language support
- [ ] Recording waveform overlay (minimal UI)
- [ ] Voice commands ("new line", "delete that")
- [ ] Background noise suppression (basic gate)

### P3 — Stretch
- [ ] Mobile (via Rust core compiled for Android/iOS)
- [ ] Cross-device sync
- [ ] Linux support

## Architecture
```
┌──────────────────────────────────────────────────┐
│                  Tauri Shell                      │
│  ┌────────────┐  ┌──────────────┐  ┌──────────┐ │
│  │ System Tray │  │  Webview UI  │  │ Hotkey   │ │
│  │ (always on) │  │  (settings)  │  │ Listener │ │
│  └────────────┘  └──────────────┘  └──────────┘ │
└──────────────────────┬───────────────────────────┘
                       │
┌──────────────────────▼───────────────────────────┐
│                 Rust Core                         │
│  ┌──────────┐  ┌────────────┐  ┌──────────────┐  │
│  │ Audio    │  │Transcription│  │ Text         │  │
│  │ Capture  │─▶│ Engine     │─▶│ Injection    │  │
│  │ (cpal)   │  │(local/cloud)│  │ (enigo)      │  │
│  └──────────┘  └────────────┘  └──────────────┘  │
│                      │                            │
│               ┌──────┴──────┐                    │
│               │ Local       │ Cloud              │
│               │ (whisper.rs)│ (reqwest API)      │
│               └─────────────┘                    │
└──────────────────────────────────────────────────┘
```

## Milestones

| Phase | What you'll learn | Scope |
|-------|------------------|-------|
| **Phase 1** | Rust basics (vars, functions, structs, crates) | Audio capture + basic Tauri app |
| **Phase 2** | File I/O, FFI bindings, async | whisper.cpp integration + text injection |
| **Phase 3** | HTTP requests, JSON config, webview UI | Cloud API option + settings window |
| **Phase 4** | Polish, error handling, packaging | Release builds for Windows/Mac/Linux |

## Accuracy Target
- **Local**: ~90% WER using whisper small/medium models
- **Cloud**: ~95%+ WER via OpenAI Whisper API
- Custom dictionary improves domain-specific terms
