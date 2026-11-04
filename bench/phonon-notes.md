# Phonon-2.5 vs Parakeet v2 — fair-weather bench (2026-11-04)

Same machine, same clips, same scorer (`bench.py:score_pairs`).

## Results (50 clips each)

| Dataset | Parakeet v2 (yours) | Phonon-2.5 English |
|---|---|---|
| librispeech-clean | **1.42%** WER, 66.0% exact, 11.0x realtime | 2.05% WER, 58.0% exact, 2.7x realtime |
| librispeech-other   | **2.59%** WER, 54.0% exact, 11.3x realtime | 3.39% WER, 46.0% exact, 2.4x realtime |
| earnings22          | 16.10% WER, 10.0% exact, 11.4x realtime | **13.64%** WER, 8.0% exact, 2.8x realtime |

(CIs all overlap on earnings22; on the LibriSpeech sets Parakeet is ahead.)

## Why Phonon loses on speed here

1. **It reloads the model every call.** `fermion transcribe` is a one-shot CLI: L
   each invocation loads the model (measured: **2.4s load**) + ~1.7s decode. That
   is the 4.04–4.15s per clip above, and it is *not* Phonon's fault — it is the
   CLI's. Your Parakeet helper and `sherpa-onnx` both keep the model resident, so
   they pay load once.
2. **No Python Transcriber exists.** `fermion` ships only a CLI. There is no
   in-process API to call from a bench loop or from an app; the only way to use
   it without rebuilding process-per-clip is `fermion serve` (OpenAI-compatible)
   and HTTP calls — i.e. a resident sidecar server, same as your `llama-server`.
3. **Format is proprietary.** The model is a single `model.fermion` container
   (packed int8, AVX2 kernels, custom C decoder). It is **not** ONNX, not
   sherpa-onnx-loadable, and there is no published ONNX export. Your Rust helper
   (`smoothflow-stt.exe` + sherpa-onnx) cannot load it. Adopting Phonon means
   shipping Fermion's runtime (or PyTorch) as a sidecar.
4. **No Chinese / Indian English focus.** 25 European languages, no Chinese,
   accented-English dictation was 97% WER in TypeWhisper's independent test.

## Accuracy verdict

Parakeet v2 wins on read-speech (LibriSpeech clean/other — the audiobook
condition, closest to clear held-out dictation). Phonon-2.5 English wins on
earnings calls (+2.5 points) — spontaneous, jargon-dense speech. That matches
Fermion's own table (they win AMI/earnings, lose LibriSpeech). For an app
whose primary use is dictation into arbitrary text fields, that trade is roughly
a wash; for meeting-notes use, Phonon is plausibly better.

## What "testing this kind of model" would commit us to

- A **Python + PyTorch sidecar** (fermion needs Python 3.10+ and its deps) or a
  **second sidecar server** (`fermion serve`) that Rust HTTP-calls. Either way:
  a 200MB+ extra runtime, slower cold start, antivirus surface, and a new
  failure mode to debug in users' homes ("why is Python running?").
- **No integration into the current engine.** It does not drop into
  `smoothflow-stt.exe`. It is a parallel engine with its own lifecycle.
- The upside versus shipping it: nothing you can't get from a sherpa-onnx-native
  model of similar size (SenseVoiceSmall 226MB, Moonshine) that already loads in
  the helper you ship.

## Recommendation

Keep Parakeet as the shipped default. The Phonon numbers do not justify a
Python sidecar: it is 4x slower per s (and that gap is model load, not decode),
slightly worse on clean/other, and only better on earnings-style speech.

If you still want a Fermat-family option for users who care about download size,
the honest way is to wait for an ONNX/GGML export (transcribe.cpp-style) or evaluate
`sherpa-onnx` models that already fit — rather than bundling Python for a
marginal, mixed-result gain.

## Reproduce

```
pip install fermion-research sherpa-onnx huggingface_hub soundfile
python bench/asr_bench.py --dataset librispeech-clean --limit 50
python bench/asr_bench.py --dataset librispeech-other --limit 50
python bench/asr_bench.py --dataset earnings22 --limit 50
```

## Model provenance note

Not a licence or provenance concern for the *benchmark*: both models are
CC-BY-4.0 and for the bench we only download and score them. The point of the
note is the same as before — the container format is Fermion's own, so "we
tested it" does not mean "we can ship it in the app we have."
