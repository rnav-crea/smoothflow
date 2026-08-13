# SmoothFlow

Done-for-you voice dictation for Windows, macOS, and Linux. Hold a hotkey, speak,
release — and your cleaned, punctuated text is typed straight into whatever app you're in.

Built with [Tauri 2](https://tauri.app) (Rust core + WebView2) and a framework-free
vanilla HTML/CSS/JS frontend.

---

## Downloads

Installers are published to the [GitHub Releases page](https://github.com/rnav-crea/smoothflow/releases/latest)
when a new version is tagged. The buttons below download the latest installer directly:

<a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-Windows-x64-setup.exe">
  <img src="https://img.shields.io/badge/Download_for-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Download for Windows">
</a>
<a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-macOS-arm64.dmg">
  <img src="https://img.shields.io/badge/Download_for-macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="Download for macOS">
</a>
<a href="https://github.com/rnav-crea/smoothflow/releases/latest/download/SmoothFlow-Linux-x86_64.AppImage">
  <img src="https://img.shields.io/badge/Download_for-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Download for Linux">
</a>

| Platform | Installer | Notes |
|----------|-----------|-------|
| **Windows** | `SmoothFlow-Windows-x64-setup.exe` (or `.msi`) | x86_64. WebView2 runtime is auto-installed if missing. |
| **macOS** | `SmoothFlow-macOS-arm64.dmg` (Apple Silicon) | Unsigned: first launch needs **right-click → Open**. You'll be asked for **Accessibility permission** on first dictation. |
| **Linux** | `SmoothFlow-Linux-x86_64.AppImage` (or `.deb`) | Requires `libwebkit2gtk-4.1`; `sudo apt install` it if launch complains. |

> The buttons link to the **latest** release, so they always fetch the current version — no need to update them per release.
> Intel Macs (x86_64) aren't shipped yet; build from source for now (see Development below).

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

### Windows

1. Run `SmoothFlow_<version>_x64-setup.exe` and follow the installer.
   If Windows SmartScreen warns *"isn't commonly downloaded"*, click **More info → Run anyway**
   — SmoothFlow is unsigned, so this is expected on first install. Open-source alone doesn't
   remove this; it goes away once the build is signed (paid code-signing certificate or Azure
   Trusted Signing) or after enough people install and it earns SmartScreen's trust.
2. Launch SmoothFlow from the Start menu. It sits in the system tray.

### macOS

1. Open the `.dmg` and drag **SmoothFlow** into Applications.
2. First launch: **right-click** the app in Applications → **Open** → Open (required because the app is unsigned).
3. On your first dictation, grant the **Accessibility** permission when prompted
   (System Settings → Privacy & Security → Accessibility). This is what lets SmoothFlow
   type into other apps. It's also needed for the global hotkey to work.

### Linux

1. Install the `.deb` with `sudo apt install ./SmoothFlow_<version>_amd64.deb`, or make the
   `.AppImage` executable (`chmod +x`) and run it.
2. If it fails to launch, install WebKitGTK: `sudo apt install libwebkit2gtk-4.1-0`.

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
| **Hold `Meta+Space`, speak, release** | Your words are cleaned and typed into the focused app (`Cmd+Space` on macOS, `Win+Space` on Windows) |
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
| **API Key** | Your Groq key. Stored in your OS credential manager (Windows Credential Manager, macOS Keychain, Linux Secret Service) — never in a file. |
| **Auto Punctuation** | Adds ending punctuation (periods) to sentences. |
| **Remove Filler Words** | Strips "um", "uh", "you know", etc. |
| **Auto-Paste** | Types the final text into the active window. Off = text only appears in the main window. |
| **Launch on Startup** | Starts SmoothFlow when you log in. |
| **Hotkey** | The global dictation key, e.g. `Ctrl+Space`. Format: modifier + key (`Ctrl`, `Alt`, `Shift`, `Win`/`Cmd`). |
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
| *No microphone found* | No mic detected | Plug one in / enable it in your OS |
| *Could not start the microphone* | Mic is in use by another app | Close the other app, retry |
| *Auto-paste failed* | Target app blocked paste (e.g. some terminals) | Click into the target app first, retry |
| *Accessibility permission required* (macOS) | SmoothFlow can't type into other apps | System Settings → Privacy & Security → Accessibility → enable SmoothFlow |

---

## Privacy

Full policy in [PRIVACY.md](PRIVACY.md).

- Your **voice audio is sent to Groq's cloud** for transcription (this is how it works).
- Your **API key is stored in your OS credential manager**, not in any settings file.
- SmoothFlow has no local/offline speech recognition.

## Limits (free Groq key)

- Up to **2,000 dictations per day**, max **20 per minute**.
- A dictation can be up to ~100 MB of audio (many minutes of speech).
- Far beyond normal daily use; the per-minute cap is the only one you might feel
  (rapid-fire short dictations in the same minute).

---

## Development

```bash
npm install            # frontend deps (once)
npm run tauri dev      # dev loop: vite on :1420 + Tauri window
npm run tauri build    # production installer for the current OS
```

See `AGENTS.md` for the build prerequisites, architecture, and project conventions.

---

## License

[MIT](LICENSE) — free to use, modify, and distribute, including in commercial
projects. Third-party components (Tauri, cpal, enigo, etc.) have their own licenses.
