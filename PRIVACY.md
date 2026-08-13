# SmoothFlow Privacy Policy

Effective date: 2026-08-13

This policy explains what SmoothFlow collects, where it goes, and what is stored
on your machine. SmoothFlow is an open-source application; this policy describes
the app as it is distributed.

## What we collect and where it goes

- **Voice audio and transcribed text** are sent to the Groq cloud API (or the
  API endpoint you configure in Settings) for speech-to-text transcription and
  optional cleanup. This is how dictation works — there is **no local/offline
  speech recognition**.
- The audio and transcript are processed by the provider you configured and are
  not stored by us. We do not maintain servers of our own.

## What is stored on your machine

- **API key**: stored in your operating system's credential manager (Windows
  Credential Manager, macOS Keychain, Linux Secret Service). It is never written
  to a settings file.
- **Configuration**: your settings are stored locally in a JSON file
  (`%APPDATA%\SmoothFlow\smoothflow.json` on Windows, `~/SmoothFlow/` on
  macOS/Linux).
- **Dictation history**: kept locally in the same folder. It never leaves your
  machine.

## What we do not collect

- No accounts, no telemetry, no analytics, no crash reporting, no advertising
  identifiers.
- We do not track your usage, your active windows, or your keystrokes beyond the
  text you dictate. The active-window title is read only to build vocabulary
  hints for transcription and is not transmitted anywhere.

## Third parties

The only external service is the transcription/cleanup API endpoint you
configure. When using the default, your audio is handled under
[Groq's privacy policy](https://www.groq.com/legal/privacy-policy). Review the
policy of any other provider you configure.

## Open source

SmoothFlow is [MIT licensed](LICENSE). You can inspect the source to verify this
policy matches the code.

## Contact

For questions about this policy, open an issue on the
[GitHub repository](https://github.com/rnav-crea/smoothflow).
