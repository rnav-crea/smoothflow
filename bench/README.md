# SmoothFlow speech benchmark

Reproducible accuracy and speed numbers for the local Parakeet engine, on the
same audio the public ASR leaderboards use — so a SmoothFlow number can sit
next to a published one instead of being an anecdote.

**This file documents the harness. For the measured results, tables and the
comparison against published/competitor numbers, read
[`../BENCHMARK.md`](../BENCHMARK.md).**

## Run it

```bash
# 1. PULL CLIPS  (streams only the bytes it needs, ~13-238 MB per 300 clips)
python bench/bench.py fetch --dataset earnings22 --limit 300

# 2. SCORE THEM
python bench/bench.py run --dataset earnings22

# 2b. SAME CLIPS THROUGH THE HOSTED ENDPOINT, for the offline-vs-cloud row.
#     The key comes from the environment, never from argv (argv is visible in
#     the process list and lands in shell history).
$env:SMOOTHFLOW_API_KEY = '<your key>'
python bench/bench.py run --dataset earnings22 --cloud

# 3. PUT EVERY RUN IN ONE TABLE
python bench/bench.py compare
```

`run --verbose` prints each clip where the engine made a mistake, with the
reference and the hypothesis side by side — that is where the fixes come from.

`--cloud` sends the exact multipart request the app builds
(`transcription.rs`): same `audio.wav` PCM16 body, same `model`,
`response_format`, `language=en` and vocabulary `prompt`, same 1 s between-call
gate, and the same gain normalization and Whisper hallucination filter. The only
thing that changes between the two rows is the engine, so the difference is the
engine. Throttled retries are excluded from the timing, so a 429 cannot be
mistaken for model latency.

Requirements: Python 3.10+, `pyarrow`, `ffmpeg` on `PATH`, and `requests` for
`--cloud` only. Models must already be downloaded (enable offline mode in the app
once). The helper binary is read from `src-tauri/binaries/win-x64/`.

## Measuring the cleanup pass

Raw WER is the leaderboard number, not the user-facing one. The app then runs
`text_rules` → casual fast path → local Qwen → faithfulness guard, and *that*
output is what lands in the document. Re-implementing that pipeline in Python
would measure the Python copy, so `clean` shells out to a bench-only binary that
calls the real `postprocess` function:

```bash
# one-time build
cd src-tauri && cargo build --bin cleanup_eval && cd ..

# rules only -- what the app produces when no model is installed
python bench/bench.py clean --dataset earnings22 --cleanup-engine off

# rules + the local model
python bench/bench.py clean --dataset earnings22 --cleanup-engine local --model-size 4b
```

Both stages re-score the **saved raw transcripts** from the newest `run`, through
the same `score_pairs` used everywhere else, so the only variable is the cleanup
stage. `compare` then prints raw → rules → rules+Qwen side by side.

`clean` needs `--model-size` to name a model that is actually on disk under
`%APPDATA%\SmoothFlow\models\llm\<size>\`. If the size in `smoothflow.json` is
not installed, the app's `model_path` returns an error and cleanup silently
falls back to rules — so check that setting before believing a "rules only"
number.

## Datasets

| name | what it stresses | source |
|---|---|---|
| `librispeech-clean` | clean read audiobook speech — the flattering floor | Open ASR Leaderboard |
| `librispeech-other` | noisy/accented read speech — the realistic floor | Open ASR Leaderboard |
| `earnings22` | **spontaneous financial calls: names, tickers, numbers, jargon** | Open ASR Leaderboard |
| `common_voice` | spontaneous English, many accents | Open ASR Leaderboard |
| `urgent2024` | far-field and noisy conditions | Open ASR Leaderboard |

All five are the `hf-audio/esb-datasets-test-only-sorted` splits, which is what
"ESB" in a published dataset ID refers to.

## What the numbers mean

- **WER** — `(substitutions + deletions + insertions) / reference words`.
  Corpus-level: total errors over total words, *not* an average of per-clip
  rates, which would weight a three-word clip the same as a two-minute one.
- **95% CI** — bootstrapped over clips, not a naive binomial interval. Speech
  clips cluster, so the honest interval is wider than the textbook one. **A
  difference under about 2 points is noise, not an improvement.**
- **exact** — clips transcribed with zero word errors. This is a *lower bound*
  on the user-facing zero-edit rate, because the LLM cleanup pass is not in
  this measurement path.
- **recall** — proper nouns and digit-bearing tokens recovered. Capitalization
  in the source transcript is the only cheap "this is a name" signal, so this
  is approximate by construction.
- **speed** — `audio seconds / decode seconds`. Run one dataset at a time.
  Running two `run` jobs concurrently halves both numbers, and that is a
  measurement artifact, not a regression.

## Rules that keep the number honest

1. **One normalizer, both sides.** Lowercase, strip punctuation, collapse
   whitespace, drop apostrophes. If the reference and the hypothesis are not
   normalized identically, WER means nothing.
2. **No number or unit expansion.** `2020` stays `2020`. This deliberately
   under-reports accuracy on text where the reference spells numbers out.
3. **Raw only.** The cleanup pass rewords things, which *raises* WER against a
   verbatim reference. Public leaderboards measure competitors the same way,
   with cleanup off, precisely so the comparison is fair.
4. **Freeze the audio.** `bench/data/` is generated once and reused. Re-fetching
   between runs invalidates every comparison.
5. **Report the N.** 300 clips and ~4,500 words is enough to separate 15.7% from
   8.4%, and not enough to separate 15.7% from 15.2%.
6. **Sample by duration, never by file order.** These shards are duration-sorted,
   so "read the first N rows" measures the longest or shortest clips in the
   corpus and nothing else. This one mistake moved one dataset by 3.5 points.
7. **Verbatim references punish filler removal, and filler removal is correct.**
   `earnings22` refers to every "uh" and "um". An engine that silently drops them
   is doing what a dictation user wants, but WER scores each drop as a deletion.
   Report the verbatim number (comparable to the leaderboard) *and* the
   filler-stripped one (closer to what a user experiences): on 300 clips they
   differ by roughly two points for the local engine.

## Results — 2026-10-30

~300 clips per dataset, duration-stratified, Parakeet TDT 0.6B v2 int8, 8
threads, one job at a time.

| dataset | clips | sec/clip | words | WER | 95% CI | exact | recall | speed |
|---|---|---|---|---|---|---|---|---|
| librispeech-clean | 286 | 6.3 | 4,970 | **2.2%** | 1.7–2.7% | 75.5% | — | 10.4× |
| librispeech-other | 300 | 5.3 | 4,408 | **3.4%** | 2.7–4.2% | 70.0% | — | 10.2× |
| earnings22 | 300 | 5.7 | 4,551 | **15.7%** | 14.0–17.5% | 30.7% | 65.1% | 9.7× |

Published reference points on the same corpora
(`superwhisper.com/benchmarks`, updated 2026-09-03):

| dataset | SmoothFlow | Parakeet V2 in Superwhisper | best cloud |
|---|---|---|---|
| librispeech-other | **3.4%** | 4.1% | 3.3% (OpenAI) |
| librispeech-clean | **2.2%** | — | 3.8% (Wispr Flow raw) |
| earnings22 | 15.7% | **11.1%** | **8.4%** (OpenAI) |

`Parakeet V2 in Superwhisper` is the *same model*, so `librispeech-other` is a
direct control: 3.4% vs 4.1% with overlapping intervals means this harness and
their measurement agree.

### Sampling bias — read this before trusting any number

The corpus repo is sorted by duration. The first version of this fetcher simply
took the first N rows, which produced:

| dataset | biased sample | real distribution | effect |
|---|---|---|---|
| librispeech-other | 17.5s mean (longest clips) | 6.5s mean | long clips are easier, so WER looked better |
| earnings22 | 1.3s mean, ~7 words (shortest shard) | 5.7s mean | fragments, so WER looked much worse |

That run reported 2.6% and 19.2%. Neither was comparable to anything. `fetch`
now indexes `audio_length_s` for the whole shard (0.1 MB), samples evenly by
duration rank, then reads only the row groups it needs. Every result records
`mean_clip_seconds` so the bias cannot return unnoticed.

### Dataset defect: 28 misaligned earnings22 clips

28 of 300 clips carry reference text physically longer than their audio (up to
22 words/second, where human speech is 2–3). Those clips score 40% WER on their
own but hold only 3% of the words; excluding them moves the corpus number from
16.0% to 15.2%. That is a corpus defect, not an engine one, and the remaining
gap is real.

### Hypotheses tested and rejected

- **Beam search would close the gap to the same model.** `--decoding-method
  modified_beam_search` scores 17.1% against greedy's 15.7%, and runs slower
  (8.3× vs 9.7×). Deletions rise 303 → 385. Greedy is right for this engine.
- **A long-lived helper session degrades over time.** All 18 collapse clips
  re-run in a fresh process each reproduced their output byte for byte. No
  session-state bug.
- **Short clips cause the deletions.** WER is flat above 2s: 15.7% at 2–4s,
  14.1% at 4–8s, 15.6% at 8s+. Distributed error, not an edge case.

### What this says

- **Read speech: competitive.** 2.2% clean and 3.4% other match or beat both a
  shipping product running the same engine and the best cloud model.
- **Spontaneous speech: 4–7 points behind.** 15.7% against 11.1% for the same
  model elsewhere, and 8.4% for the best cloud. This is the honest weakness.
- **The problem is the kind of error, not the count.** 6.7% of words are dropped
  outright (303 deletions of 4,551) and 43% of digit-bearing tokens never appear
  (82 in the references, 35 unaccounted). Dropped numbers and names are what
  make a transcript unusable.
- **Speed is a genuine advantage.** 9.7–10.4× realtime on plain x86 CPU against
  1.3–2.3× for Wispr's cloud path. Superwhisper reports 165× on an Apple M4,
  which is different silicon and not comparable to this machine.

## Not measured yet

- **The cleanup pass.** Exact-match here is raw, so the user-facing zero-edit
  rate is higher than the 30.7% measured on earnings22. Adding it means driving
  `llama-server` too, and reusing the real `system_prompt_offline` so the prompt
  cannot drift from what ships.
- **End-to-end latency.** This measures decode time, not the wait a person
  feels (hotkey release → text in the app). That includes injection and belongs
  in a separate measurement.
- **Your own voice.** Public corpora are not your accent, your room, or your
  jargon. Twenty of your own dictations would be worth more than these 750
  clips for predicting your daily experience — `keep_audio_clips` in
  `config.rs` exists for exactly that and is currently unimplemented.
