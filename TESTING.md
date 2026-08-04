# SmoothFlow — Manual Test Plan

Full end-to-end test plan for the installed SmoothFlow desktop app. Every
scenario has concrete **Steps**, the **Expected** result, and how to **verify**
a pass. The app is a voice-to-text dictation tool: hold the hotkey, speak,
release, and cleaned-up text is typed into the active window.

Release gate: before running this plan, run the automated checks once
(see "Release Gates" at the bottom). Then work through A → G and tick each
row. Nothing blocking may remain open before publishing.

---

## Setup (do this first)

1. Uninstall any previous SmoothFlow build, then install the new one to confirm a clean first run.
2. Open Settings (gear icon, top right of the main window).
3. Paste a valid Groq API key (e.g. `gsk_...`) into **API Key**.
4. Confirm **API Base URL** = `https://api.groq.com/openai/v1`, **Model** = `whisper-large-v3`, **Cleanup Model** = `llama-3.1-8b-instant`.
5. Open Notepad and, separately, VS Code — you will dictate into these.
6. Verify your microphone: Windows Settings → System → Sound → Input → speak and check the level meter moves.
7. Config file lives at `%APPDATA%\SmoothFlow\smoothflow.json`. You'll inspect/corrupt it in section C.

---

## A. Core Dictation Flow

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| A1 | First launch | Double-click the SmoothFlow icon. | Main window opens, status says **Idle**, the two transcript panels show their placeholder text, a tray icon appears in the notification area. | Window visible + tray icon present. |
| A2 | Record via Space | Focus the SmoothFlow window. Hold the **Space** key, speak a full sentence, release. | Recording starts (status → "Recording...", overlay bubble appears at top-center), then raw text appears in **raw voice** panel, cleaned text in **transformed** panel. | Both panels update; status returns to **Idle**. |
| A3 | Record via button | Click **Record**, speak, click **Stop**. | Same as A2. | Button label toggles Record/Stop; both panels update. |
| A4 | Record via global hotkey | Focus Notepad. Hold **Ctrl+Space**, speak a sentence, release. | Overlay bubble appears while held, then the cleaned sentence is typed into Notepad. | Text appears in Notepad; overlay disappears after release. |
| A5 | Live VU meter | Start recording (Space or button) and speak continuously. | Overlay bars rise and fall with your voice. **Note:** bars should also animate when recording via the Ctrl+Space hotkey. | Watch the overlay bubble while speaking in both paths. |
| A6 | End-to-end latency | Set a stopwatch on your phone. Focus Notepad, press **Ctrl+Space**, speak one short sentence, release and start the timer. Stop when text lands in Notepad. | Time is under **3 s** on a normal connection. | Record the number. >5 s repeatedly = flag it. |
| A7 | Rapid start/stop | Tap **Ctrl+Space** 20 times in a row, speaking briefly each time. | No crash, no stuck "Recording..." state, no doubled/partial text. | App still responsive; final status is **Idle**. |

---

## B. Accuracy & Post-Processing

Dictate the exact phrase in **Steps**, compare against **Expected**. Say the phrase naturally, don't spell it.

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| B1 | Filler removal | Dictate: "um hello world" | `hello world.` | Filler "um" gone, trailing period added. |
| B2 | Preserve existing punctuation | Dictate: "hello world!" | `hello world!` | No double period (`hello world!.` is a fail). |
| B3 | Self-correction (time) | Dictate: "meet at 6pm no wait 7pm" | `meet at 7pm` | Only 7pm survives. |
| B4 | Self-correction (day) | Dictate: "today no tomorrow" | `tomorrow` | "today" dropped. |
| B5 | "instead of" | Dictate: "meet at 5pm instead of 4pm" | `meet at 5pm` | First option kept, rest dropped. |
| B6 | Email conversion | Dictate: "email navin at redmail dot com" | Something containing `navin@redmail.com` | "at" converted to `@`. |
| B7 | Fillers toggle OFF | Settings → turn **Remove Filler Words** off. Dictate: "um hello" | `um hello` stays as-is. | Filler retained. Re-enable after. |
| B8 | Punctuation toggle OFF | Settings → turn **Auto Punctuation** off. Dictate: "hello world" | `hello world` with **no** trailing period. | No period. Re-enable after. |
| B9 | Cleanup model empty (offline fallback) | Settings → clear **Cleanup Model**, save. Dictate: "um hello world" | `hello world.` — still works, fillers removed and punctuation added, but no LLM enhancement. | Works without cleanup model. Restore `llama-3.1-8b-instant` after. |
| B10 | Personal dictionary | Settings → **Add a word**: `Hyundai`. Save. Then dictate a sentence containing "Hyundai". | "Hyundai" is transcribed correctly (uncapitalized or misspelled = fail). | Word appears correctly. Remove it after. |
| B11 | Long dictation | Dictate for 30–60 seconds, several sentences, natural pauses. | Full text present, each sentence ends with punctuation, nothing dropped or garbled. | Compare spoken vs. output; no truncation. |

---

## C. Config & Persistence

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| C1 | Settings persist across restart | Change the **Model** to `whisper-1`, save. Fully quit (tray → Quit), relaunch. | Settings modal still shows `whisper-1`. | File check: `%APPDATA%\SmoothFlow\smoothflow.json` contains `"model": "whisper-1"`. Restore `whisper-large-v3` after. |
| C2 | Hotkey change | Settings → **Hotkey** = `Alt+Shift+T`, save. | Old `Ctrl+Space` no longer records; `Alt+Shift+T` does. Restart app → `Alt+Shift+T` still works. | Both checks pass. Restore `Ctrl+Space` after. |
| C3 | Invalid hotkey rejected | Settings → **Hotkey** = `Foo`, save. | Rejected — the value is not saved / an error appears. | Hotkey unchanged after reopen. |
| C4 | Empty API key rejected | Settings → clear **API Key**, save. | Rejected — "API key cannot be empty" (or a save failure). | Old key still saved. Restore key after. |
| C5 | Empty model rejected | Settings → clear **Model**, save. | Rejected — "Model name cannot be empty". | Model unchanged after reopen. |
| C6 | Launch on startup ON | Settings → turn **Launch on Startup** on. | App auto-starts on next login. | Task Manager → Startup apps shows a SmoothFlow entry. |
| C7 | Launch on startup OFF | Turn **Launch on Startup** off, then restart the app once. | Entry removed. | Task Manager → Startup apps no longer lists SmoothFlow. |
| C8 | Overlay position | Settings → turn **Overlay at Top** off (bottom), save. Dictate once. | Overlay bubble appears at the bottom of the screen. | Bubble location. Toggle back on after. |
| C9 | Dictionary add/remove | Settings → add `MyJargon`, save. Click the × on the tag. | Tag appears after add; disappears after remove; persisted after restart. | File check `dictionary` array in `smoothflow.json`. |
| C10 | Corrupt config | Quit the app. Open `%APPDATA%\SmoothFlow\smoothflow.json` and overwrite it with garbage like `not valid json{`. Relaunch. | App opens with default settings, no crash. | App runs; settings are defaults. Fix/restore the file after. |
| C11 | Missing config | Quit the app. Delete `smoothflow.json`. Relaunch. | App opens with defaults, no crash. | App runs; a fresh `smoothflow.json` is recreated on first save. |
| C12 | Key visibility toggle | Settings → click the eye icon next to **API Key**. | Key toggles between `••••` and plain text. | Both states visible. |

---

## D. Error Handling (app must never crash)

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| D1 | No API key | Settings → clear **API Key**, save. Dictate a sentence. | A clear error is shown (status → **Error** / panel shows the message), no silent nothing. | Error text visible, app still usable. |
| D2 | Invalid API key | Set key to `gsk_invalid`. Dictate. | Error shown containing `401` or "API error". | Error visible, app responsive. Restore valid key after. |
| D3 | No internet | Turn on airplane mode (or disconnect Wi-Fi). Dictate. | Error appears after ~15 s (timeout), no hang. | App unresponsive-free; error eventually shows. Reconnect after. |
| D4 | No mic / mic disabled | Windows Settings → disable your microphone input. Dictate via hotkey. | Error like "No input device found" or "Could not start recording", no crash. | Error visible, app still runs. Re-enable mic after. |
| D5 | Pure silence | Record and release without speaking. | Result shown as `(silence)`; nothing typed into the target app. | No text typed, no crash. |
| D6 | Noise only | Record while the room is noisy but nobody speaks. | Empty/silence result, or a short garbage transcript — never a crash. | No crash. |
| D7 | Sub-second dictation | Press and release **Ctrl+Space** instantly. | No crash; result is empty/silence. Acceptable. | App alive. |
| D8 | Double hotkey press | Hold **Ctrl+Space** (start recording), then press it again while still recording. | Second press ignored — no restart, no double text later. | Only one transcript appears on release. |

---

## E. Reliability & App Lifecycle

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| E1 | Tray → Settings | Right-click tray icon → **Settings**. | Main window opens and comes to front. | Window focused. |
| E2 | Tray → Quit while recording | Start dictating, then right-click tray → **Quit** mid-recording. | App exits cleanly, no hang, no stray audio. | Process gone (Task Manager). |
| E3 | Close window (X) | Start dictating, then click the window's X. | **Current behavior:** app exits entirely (by design, see note). | Confirm this is intended; if you expected tray-minimize, flag it as a UX decision. |
| E4 | Single instance | With app running, launch the exe a second time. | Second instance closes; the first window focuses. | Only one SmoothFlow process in Task Manager. |
| E5 | Long session | Dictate every couple of minutes for 30+ minutes. | No crash; memory stays roughly flat. | Watch Task Manager memory trend. |
| E6 | Quit-during-recording relaunch | Tray → Quit while recording, then relaunch. | Fresh start, status **Idle**, no stuck "Recording..." state. | First recording works normally. |

---

## F. Cross-App Injection

Test auto-paste into each target: **Notepad, VS Code, Chrome address bar, Gmail compose, Word, a terminal (cmd/PowerShell).**

For each target:

| # | Test | Steps | Expected | Verify pass |
|---|------|-------|----------|-------------|
| F1 | Inject into target | Focus the target, hold **Ctrl+Space**, dictate one sentence, release. | Cleaned text lands in the focused field at the cursor. | Text present, in the right place. |
| F2 | Auto-Paste OFF | Settings → turn **Auto-Paste** off. Dictate. | Text shows in the app panels only — **nothing** is typed into the target. | Target unchanged. Re-enable after. |
| F3 | Terminal target | Focus a terminal and dictate. | Fails **gracefully** (an error event shows, no crash) — acceptable, terminals don't support paste injection. | No crash; app still usable. |
| F4 | Clipboard preserved | Copy "my-secret-123". Dictate a sentence into Notepad. Then paste (Ctrl+V) in Notepad. | Your copied text is still there (`my-secret-123`), not overwritten by the transcript. | Clipboard content survives. |

---

## G. Release Gates (automated — run once, must pass)

```bash
cd src-tauri
cargo check --target-dir /tmp/sf-target
cargo test --target-dir /tmp/sf-target
cd ..
npm run tauri build
```

| # | Check | Pass condition |
|---|-------|----------------|
| G1 | `cargo check` | No errors. |
| G2 | `cargo test` | All unit tests green (config, audio, transcription, postprocess, text_injection). |
| G3 | `npm run tauri build` | Installer produced; Windows icon valid (no `.ico` resource error). |

---

## Result

- Every A–F row ticked as PASS, G1–G3 green → ready to publish.
- Any row FAIL → fix it before publishing. Known watch-items from code review:
  - A5 (hotkey VU meter animation) — fixed so bars animate in both paths.
  - D4 (hotkey start failure now emits `recording-error` so the UI shows it instead of a silent console log).
