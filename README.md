<p align="center">
  <img src="src-tauri/icons/icon.png" alt="SmoothFlow logo" width="120">
</p>

<h1 align="center">SMOOTHFLOW</h1>

<p align="center">
  <strong>Voice dictation that types clean text into any app.</strong><br>
  Hold a hotkey, speak, release — your words land punctuated and filler-free wherever your cursor is.
</p>

<p align="center">
  <a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-Windows-x64-setup.exe">
    <img src="https://img.shields.io/badge/Download_for-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Download for Windows">
  </a>
  <a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-macOS-arm64.dmg">
    <img src="https://img.shields.io/badge/Download_for-macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="Download for macOS">
  </a>
  <a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-Linux-x86_64.AppImage">
    <img src="https://img.shields.io/badge/Download_for-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Download for Linux">
  </a>
</p>

<p align="center">
  Built with <a href="https://tauri.app">Tauri 2</a> (Rust core + webview) and a framework-free vanilla JS frontend.<br>
  Free &amp; open source · MIT licensed · Works on Windows, macOS (Apple Silicon), and Linux
</p>

---

## Features

- **Dictate into any app** — email, chat, Word, browser, terminal. Works anywhere you can type, system-wide.
- **Clean text, automatically** — ending punctuation added, "um/uh" fillers stripped, self-corrections resolved
  ("I'm going I'm going to" → "I'm going to"), spoken emails converted ("user at gmail dot com" → `user@gmail.com`).
- **Global hotkey** — hold-to-talk from any application; SmoothFlow lives in the system tray.
- **Floating overlay** — a small pill at the top of the screen with a live level meter while you speak;
  turns red with a short message if something goes wrong.
- **Personal dictionary** — teach it names, jargon, and project terms so they transcribe correctly.
- **Hallucination guard** — filters Whisper's classic "thank you for watching"-style noise on silence,
  gated by per-segment speech probability so real speech is never dropped.
- **Bring your own provider** — works with any OpenAI-compatible transcription endpoint (Groq, OpenAI, Together, …).
  Your key, your provider, no lock-in — see the [Provider Guide](#provider-guide).
- **Watch it think** — the main window shows both the raw transcript and the cleaned final text side by side.

---

## Quick Start

### Step 1 — Install

| Platform | Installer | First launch |
|----------|-----------|--------------|
| **Windows** | `SmoothFlow-Windows-x64-setup.exe` (or `.msi`) from the buttons above | Run it. If SmartScreen warns *"isn't commonly downloaded"*: **More info → Run anyway** (expected — the build is unsigned; this goes away once builds are code-signed or enough installs earn trust). Launch from the Start menu; it sits in the tray. |
| **macOS** (Apple Silicon) | `SmoothFlow-macOS-arm64.dmg` | Open the `.dmg`, drag **SmoothFlow** to Applications. First launch: **right-click the app → Open → Open** (required because it's unsigned). On first dictation, grant the **Accessibility** permission when prompted (System Settings → Privacy & Security → Accessibility) — this is what lets SmoothFlow type into other apps. |
| **Linux** | `SmoothFlow-Linux-x86_64.AppImage` (or `.deb`) | `chmod +x` the AppImage and run it, or `sudo apt install ./SmoothFlow_<version>_amd64.deb`. If launch complains, install WebKitGTK: `sudo apt install libwebkit2gtk-4.1-0`. |

Intel Macs aren't shipped yet — build from source ([Development](#development)).

Installers always come from the [latest release](https://github.com/rnav-crea/smoothflow/releases/latest).
Once installed, SmoothFlow auto-checks for updates on launch and shows an in-app banner — no re-downloading.

### Step 2 — Get a free API key (2 minutes)

SmoothFlow sends your voice to a cloud transcription service — you plug in your own key.

1. Go to [console.groq.com](https://console.groq.com) → sign up → create an API key (starts with `gsk_`). No credit card needed.
2. Open **Settings** (tray icon → Settings, or the gear button in the window).
3. Paste the key into **API Key**.
4. Leave the defaults — **API Base URL** `https://api.groq.com/openai/v1`, **Model** `whisper-large-v3-turbo`.
5. Close Settings. Changes save automatically.

Done. Hold **Ctrl+Space**, speak, release. (Prefer a different provider? See the [Provider Guide](#provider-guide).)

---

## How to Use

| Action | Result |
|--------|--------|
| **Hold `Ctrl+Space`, speak, release** | Your words are cleaned and typed into the focused app (default on all platforms — changeable in Settings) |
| Click **Record** in the main window | Same thing, hands-free start/stop |
| Hold **Space** (main window focused) | Also starts dictation |

While you speak, a small pill appears at the top of the screen with a live level meter.
When you release, the pill closes. If something goes wrong, the pill reappears in red
with a short message (see [Troubleshooting](#troubleshooting)).

The main window shows two panels so you can watch the pipeline:

- **raw voice** — what the speech model heard
- **transformed** — the cleaned, final text (also what gets typed out)

**Tips**

- Speak naturally — punctuation and filler removal are automatic; no need to say "comma" or "period".
- Self-correct out loud ("I'm going— I'm going to be late") and the cleanup model resolves it.
- Say emails and URLs the way you'd say them: "support at example dot com".
- Rapid-fire dictations are rate-limited to about one per second; the pill will tell you if you're faster.

### Using the Fn key on macOS (optional)

The default hotkey is **Ctrl+Space on every platform**, including macOS. Older
SmoothFlow versions defaulted macOS to the bare `Fn` key — new installs use Ctrl+Space,
and existing installs keep whatever they had saved.

Why the change: macOS reserves Fn/Globe for system actions — a tap opens the
**Emoji & Symbols picker**, so it makes an unreliable dictation key out of the box.

If you prefer hold-to-talk on **Fn** anyway (Settings → Hotkey → `Fn`), here's what happens:

1. SmoothFlow asks for the **Accessibility** permission (System Settings → Privacy & Security →
   Accessibility). Without it, the Fn hotkey can't be captured — grant it and restart the app.
2. While SmoothFlow is running, it captures the Fn key system-wide: dictation works from any app,
   and the emoji picker does **not** open.
3. When SmoothFlow isn't running, Fn behaves normally again (emoji picker etc.). If you want it
   silent even then, set System Settings → Keyboard → **Press 🌐 key to → Do Nothing**.

---

## Settings Reference

Open via tray icon → Settings, or the gear button. Everything saves automatically on close.

| Setting | What it does |
|---------|--------------|
| **API Base URL** | Where transcription requests go. Leave at Groq's URL unless you switch providers ([Provider Guide](#provider-guide)). |
| **Model** | The speech-to-text model. `whisper-large-v3-turbo` is the recommended default. |
| **Cleanup Model** | Optional LLM used to fix self-corrections and polish text. Default `openai/gpt-oss-20b` (a Groq-hosted model). Set to empty to skip LLM cleanup entirely. |
| **API Key** | Your provider key. Stored in your OS credential manager (Windows Credential Manager, macOS Keychain, Linux Secret Service) — never in a plain file. |
| **Auto Punctuation** | Adds ending punctuation (periods) to sentences. |
| **Remove Filler Words** | Strips "um", "uh", "you know", etc. |
| **Auto-Paste** | Types the final text into the active window. Off = text only appears in the main window. |
| **Launch on Startup** | Starts SmoothFlow when you log in. |
| **Hotkey** | The global dictation key, e.g. `Ctrl+Space`. Format: modifier + key (`Ctrl`, `Alt`, `Shift`, `Win`/`Cmd`). |
| **Overlay at Top** | Shows the recording pill at the top of the screen instead of the bottom. |
| **Personal Dictionary** | Names, jargon, or terms the transcriber should recognize — passed as vocabulary hints with every request. |

---

## Provider Guide

SmoothFlow works with **any OpenAI-compatible endpoint**: it posts audio to
`{API Base URL}/audio/transcriptions` and, if a Cleanup Model is set, text to
`{API Base URL}/chat/completions`. Both calls hit the **same base URL** — that's the one constraint.

### The four fields

When switching providers, this is what changes:

| Field | Required? | Notes |
|-------|-----------|-------|
| **API Base URL** | Always | Must point at the provider's OpenAI-compatible root (ending in `/v1`). |
| **API Key** | Always | From that provider's console. Keys are per-provider — an OpenAI key won't work against Groq. |
| **Model** | Always | Model names differ per provider — see table below. |
| **Cleanup Model** | Optional | Must also belong to the *same* provider. Empty = skip LLM cleanup (raw transcript still gets punctuation/filler post-processing locally). |

### Comparison table

| Provider | Base URL | Transcription model | Cleanup model options | Cost |
|----------|----------|--------------------|-----------------------|------|
| **Groq** *(default)* | `https://api.groq.com/openai/v1` | `whisper-large-v3-turbo` or `whisper-large-v3` | `openai/gpt-oss-20b`, `llama-3.1-8b-instant`, `llama-3.3-70b-versatile` | Free tier |
| **OpenAI** | `https://api.openai.com/v1` | `whisper-1` | `gpt-4o-mini`, `gpt-4o` | Paid |
| **Together AI** | `https://api.together.xyz/v1` | `openai/whisper-large-v3` | `meta-llama/Llama-3.3-70B-Instruct-Turbo`, etc. | Paid (~$0.0015/min) |
| **Fireworks AI** | `https://audio-prod.api.fireworks.ai/v1` | `whisper-v3` or `whisper-v3-turbo` | ⚠️ none — see note | Paid |

**Fireworks note:** its chat models live on a different host (`api.fireworks.ai/inference/v1`)
than its audio endpoint, so one base URL can't serve both. Use Fireworks for transcription
and set **Cleanup Model** to empty. Its `/models` listing isn't on the audio host either,
so the **Test Connection** button may fail even though dictation works.

### Copy-paste blocks

<details>
<summary><strong>Groq</strong> — free, recommended default</summary>

```
API Base URL:  https://api.groq.com/openai/v1
Model:         whisper-large-v3-turbo
Cleanup Model: openai/gpt-oss-20b
API Key:       console.groq.com (starts with gsk_)
```
</details>

<details>
<summary><strong>OpenAI</strong></summary>

```
API Base URL:  https://api.openai.com/v1
Model:         whisper-1
Cleanup Model: gpt-4o-mini
API Key:       platform.openai.com (starts with sk-)
```
</details>

<details>
<summary><strong>Together AI</strong></summary>

```
API Base URL:  https://api.together.xyz/v1
Model:         openai/whisper-large-v3
Cleanup Model: meta-llama/Llama-3.3-70B-Instruct-Turbo
API Key:       api.together.ai
```
</details>

<details>
<summary><strong>Fireworks AI</strong> — transcription only</summary>

```
API Base URL:  https://audio-prod.api.fireworks.ai/v1
Model:         whisper-v3   (or whisper-v3-turbo)
Cleanup Model: (empty)
API Key:       fireworks.ai
```
</details>

### Providers that DON'T work

These lack an OpenAI-compatible `/audio/transcriptions` endpoint or use a different
request format — SmoothFlow will get 404s or parse errors:

- **xAI (Grok)** — its speech-to-text endpoint is `/v1/stt`, not `/v1/audio/transcriptions`.
- **Gemini (Google AI Studio)** — different multimodal API format.
- **Anthropic (Claude)** — no audio input.
- **DeepSeek, Mistral, Perplexity** — text/chat only, no transcription endpoint.

### Common misconfigurations and what happens

| You changed… | You kept… | Result |
|--------------|-----------|--------|
| Base URL + Key | Old Model name | `[TXN-004]` 404 — the new provider doesn't know that model name. Update **Model**. |
| Base URL + Key + Model | Old Cleanup Model | Transcription works, then cleanup fails — the new provider doesn't host that LLM. Update **Cleanup Model** to one of theirs, or clear it. |
| Everything except Cleanup Model | Cleanup Model cleared | Works fine — clearing cleanup is always safe. |
| Key only (wrong provider for the URL) | — | `[TXN-003]` 401 Invalid API key. Keys are provider-specific. |

Rule of thumb: **all four fields must belong to the same provider.** When in doubt,
paste one of the copy-paste blocks above.

---

## Troubleshooting

Errors shown in the red overlay pill carry a `[TXN-xxx]` code — match it below.

| Code | You see | What it means | Fix |
|------|---------|---------------|-----|
| TXN-001 | *No API key set* | Key missing | Add your key in Settings |
| TXN-002 | *Network error* | No internet / provider unreachable | Check your connection |
| TXN-003 | *Invalid API key (401)* | Key wrong, or belongs to a different provider | Re-check key + Base URL match ([Provider Guide](#provider-guide)) |
| TXN-004 | *Model or endpoint not found (404)* | Wrong model name or Base URL | Check both in Settings |
| TXN-005 | *Rate limited (429)* | Too many requests in a short window | Wait a few seconds, retry |
| TXN-005 | *Recording too fast* | Two dictations within ~1 second | Pause a moment between dictations |
| TXN-006 | *Server error* | Provider-side problem | Retry in a minute |
| TXN-007 | *Could not parse the response* | Endpoint returned something unexpected | Usually a wrong/misconfigured Base URL |
| TXN-008 | *No audio captured / encoding failed* | Mic produced silence or WAV encode failed | Speak closer to the mic; check mic permissions |
| — | *No microphone found* | No mic detected | Plug one in / enable it in your OS |
| — | *Could not start the microphone* | Mic in use by another app | Close the other app, retry |
| — | *Auto-paste failed* | Target app blocked paste (some terminals) | Click into the target app first, retry |
| — | *Accessibility permission required* (macOS) | Can't type into other apps / hotkey dead | System Settings → Privacy & Security → Accessibility → enable SmoothFlow |

Still stuck? [Open an issue](https://github.com/rnav-crea/smoothflow/issues).

---

## Privacy & Limits

Full policy in [PRIVACY.md](PRIVACY.md).

- Your **voice audio is sent to the cloud provider you configure** (Groq by default) for
  transcription — that's how it works; there is no local/offline recognition.
- Your **API key is stored in your OS credential manager**, not in any settings file.
- Nothing else leaves your machine: history stays local, and text goes only where you paste it.

**Free Groq tier limits**

- Up to **2,000 dictations/day**, max **20 requests/minute**.
- A single dictation can be very long (Whisper accepts large uploads — far more than a sentence).
- In practice the per-minute cap is the only one you'll ever feel, and only during rapid-fire use.

Paid providers (OpenAI, Together, Fireworks) have their own pricing and limits — check their docs.

---

## Development

```bash
npm install            # frontend deps (once)
npm run tauri dev      # dev loop: vite on :1420 + Tauri window
npm run tauri build    # production installer for the current OS
```

Rust-only iteration: `cargo check --target-dir /tmp/sf-target` from `src-tauri/`.

Build prerequisites (Windows toolchain, MinGW, libclang env vars) and the architecture
walkthrough live in [AGENTS.md](AGENTS.md). Testing notes in [TESTING.md](TESTING.md).

---

## License

[MIT](LICENSE) — free to use, modify, and distribute, including commercially.
Third-party components (Tauri, cpal, enigo, etc.) carry their own licenses.
