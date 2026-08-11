# SmoothFlow

Done-for-you voice dictation for Windows. Hold a hotkey, speak, release — and your
cleaned, punctuated text is typed straight into whatever app you're in.

Built with [Tauri 2](https://tauri.app) (Rust core + WebView2) and a framework-free
vanilla HTML/CSS/JS frontend.

---

## What it does

- **Dictate into any app** — email, chat, Word, browser, terminal. Works anywhere you can type.
- **Clean text, automatically** — punctuation, removed "um/uh" fillers, fixed self-corrections
  ("I'm going I'm going to" → "I'm going to"), spoken emails ("user at gmail dot com" → `user@gmail.com`).
- **Global hotkey** — works even when SmoothFlow is in the background (system tray).
- **Floating overlay** — a small pill at the top of your screen shows a live level meter
  while you speak, and shows errors in red if something goes wrong.

---

## Installation

1. Run the installer (`SmoothFlow_<version>_x64-setup.exe`).
2. Launch SmoothFlow from the Start menu. It sits in the system tray.

## First-time setup (2 minutes)

1. Open **Settings** (tray icon → Settings, or the settings button in the window).
2. Get a **free API key**: go to [console.groq.com](https://console.groq.com) → sign up →
   create an API key (starts with `gsk_`). No credit card needed.
3. Paste the key into **API Key** in Settings.
4. Keep the defaults:
   - **API Base URL**: `https://api.groq.com/openai/v1`
   - **Model**: `whisper-large-v3`
5. Close Settings — changes save automatically.

---

## How to use it

| Action | Result |
|--------|--------|
| **Hold `Ctrl+Space`, speak, release** | Your words are cleaned and typed into the focused app |
| Click **Record** in the main window | Same, hands-free start/stop |
| Hold **Space** (when the main window is focused) | Also starts dictation |

While you speak, a small pill appears at the top of the screen with a live level meter.
When you release, the pill closes. If something goes wrong, the pill reappears in red
with a short message.

The main window shows two panels so you can watch the magic:
- **raw voice** — what the speech model heard
- **transformed** — the cleaned, final text (also what gets typed out)

---

## Settings explained

| Setting | What it does |
|---------|--------------|
| **API Base URL** | Where transcription requests go. Leave at Groq's URL unless you use another OpenAI-compatible provider. |
| **Model** | The speech-to-text model. `whisper-large-v3` is the recommended default. |
| **Cleanup Model** | Optional LLM used to fix self-corrections. Leave as `llama-3.1-8b-instant`. |
| **API Key** | Your Groq key. Stored in **Windows Credential Manager**, never in a file. |
| **Auto Punctuation** | Adds ending punctuation (periods) to sentences. |
| **Remove Filler Words** | Strips "um", "uh", "you know", etc. |
| **Auto-Paste** | Types the final text into the active window. Off = text only appears in the main window. |
| **Launch on Startup** | Starts SmoothFlow when you log into Windows. |
| **Hotkey** | The global dictation key, e.g. `Ctrl+Space`. Format: modifier + key (`Ctrl`, `Alt`, `Shift`, `Win`). |
| **Overlay at Top** | Shows the recording pill at the top of the screen. |
| **Personal Dictionary** | Add names, jargon, or terms the transcriber should recognize. |

---

## Common error messages

| You see | What it means | Fix |
|---------|---------------|-----|
| *No API key set* | Key missing | Add your Groq key in Settings |
| *Invalid API key* | Key is wrong/rejected | Re-check the key in Settings |
| *Network error* | No internet / Groq unreachable | Check your connection |
| *Rate limited (429)* | Too many requests in a short window | Wait a few seconds, try again |
| *Model or endpoint not found* | Wrong model name or URL | Check Settings |
| *No microphone found* | No mic detected | Plug one in / enable it in Windows |
| *Could not start the microphone* | Mic is in use by another app | Close the other app, retry |
| *Auto-paste failed* | Target app blocked paste (e.g. some terminals) | Click into the target app first, retry |

---

## Privacy

- Your **voice audio is sent to Groq's cloud** for transcription (this is how it works).
- Your **API key is stored in Windows Credential Manager**, not in any settings file.
- SmoothFlow has no local/offline speech recognition.

## Limits (free Groq key)

- Up to **2,000 dictations per day**, max **20 per minute**.
- A dictation can be up to ~100 MB of audio (many minutes of speech).
- Far beyond normal daily use; the per-minute cap is the only one you might feel
  (rapid-fire short dictations in the same minute).
