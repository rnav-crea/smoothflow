#!/usr/bin/env python3
"""SmoothFlow speech-recognition benchmark.

Scores the local Parakeet engine the way public ASR leaderboards do, so the
number can sit next to published ones instead of being an anecdote.

Two stages:

    python bench/bench.py fetch --dataset earnings22 --limit 200
    python bench/bench.py run   --dataset earnings22
    python bench/bench.py run   --dataset earnings22 --cloud    # hosted, for comparison

`fetch` streams rows out of the Open ASR Leaderboard parquet files on the Hub
using HTTP range requests, so it downloads only the bytes it actually scores.
Audio is written to bench/data/<dataset>/, one .wav plus a .txt reference.

`run` starts smoothflow-stt.exe once (a persistent session, exactly how the app
drives it), feeds every clip through it, and reports word error rate, exact
match rate, hard-token recall and decode speed. With `--cloud` it sends the
same clips to the configured hosted endpoint instead, using the same multipart
request the app builds, so the two rows differ only in the engine.

Scope: this measures RAW speech recognition only. The LLM cleanup pass is not
in this path, so exact-match here is a lower bound on the user-facing
zero-edit rate.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import random
import re
import struct
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import wave
from array import array
from pathlib import Path

BENCH = Path(__file__).resolve().parent
REPO = BENCH.parent
DATA = BENCH / "data"
RESULTS = BENCH / "results"

# Transcripts are multilingual text and these corpora contain "ł", "é", "ü".
# A Windows console defaults to cp1252 and raises UnicodeEncodeError on those,
# which killed a 300-clip run at clip 37. Never let printing decide the run.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

HELPER = REPO / "src-tauri" / "binaries" / "win-x64" / "smoothflow-stt.exe"

HF_BASE = (
    "https://huggingface.co/datasets/"
    "hf-audio/esb-datasets-test-only-sorted/resolve/main/"
)

# Open ASR Leaderboard test splits.
#
# All of these shards are ordered by clip duration, so any fetcher that just
# reads row groups from the top inherits a duration bias: librispeech-other
# runs 21s->2s across its row groups, and earnings22 shard 5 is its shortest
# fifth (1.3s mean, ~7 words per clip). `fetch` therefore samples evenly by
# duration rank across every shard, and records the duration spread so the
# bias cannot come back unnoticed.
DATASETS = {
    "librispeech-clean": {
        "paths": ["librispeech/test.clean-00000-of-00001.parquet"],
        "note": "clean read audiobook speech (flattering floor)",
    },
    "librispeech-other": {
        "paths": ["librispeech/test.other-00000-of-00001.parquet"],
        "note": "noisy/accented read speech (realistic floor)",
    },
    "earnings22": {
        "paths": [
            f"earnings22/test-0000{i}-of-00005.parquet" for i in range(5)
        ],
        "note": "spontaneous financial calls: names, numbers, jargon",
    },
    "common_voice": {
        "paths": [
            f"common_voice/test-0000{i}-of-00003.parquet" for i in range(3)
        ],
        "note": "spontaneous English, many accents",
    },
    "urgent2024": {
        "paths": ["urgent2024/test-00000-of-00001.parquet"],
        "note": "far-field and noisy conditions",
    },
}

SAMPLE_RATE = 16000

# Cloud engine defaults. These match the app's live config
# (%APPDATA%\SmoothFlow\smoothflow.json) and can be overridden per run.
#
# The API key is read from the environment ONLY. It is never written to disk,
# never put in argv (argv shows up in the process list and shell history), and
# never stored in a results file.
CLOUD_KEY_ENV = "SMOOTHFLOW_API_KEY"
CLOUD_DEFAULT_BASE = "https://api.groq.com/openai/v1"
CLOUD_DEFAULT_MODEL = "whisper-large-v3"


# --------------------------------------------------------------------------
# Remote parquet over HTTP range requests
# --------------------------------------------------------------------------


class HttpRangeFile(io.RawIOBase):
    """Read-only seekable file backed by HTTP range requests.

    pyarrow seeks to the footer, then to individual column chunks, so only the
    row groups we ask for are ever transferred.
    """

    def __init__(self, url: str):
        self.url = url
        self.pos = 0
        self.bytes_fetched = 0
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=60) as resp:
            self.size = int(resp.headers["Content-Length"])

    def readable(self) -> bool:
        return True

    def seekable(self) -> bool:
        return True

    def tell(self) -> int:
        return self.pos

    def seek(self, offset: int, whence: int = io.SEEK_SET) -> int:
        if whence == io.SEEK_SET:
            self.pos = offset
        elif whence == io.SEEK_CUR:
            self.pos += offset
        else:
            self.pos = self.size + offset
        return self.pos

    def read(self, n: int = -1) -> bytes:
        if n is None or n < 0:
            n = self.size - self.pos
        if n <= 0 or self.pos >= self.size:
            return b""
        end = min(self.pos + n, self.size) - 1
        req = urllib.request.Request(
            self.url, headers={"Range": f"bytes={self.pos}-{end}"}
        )
        with urllib.request.urlopen(req, timeout=60) as resp:
            data = resp.read()
        self.bytes_fetched += len(data)
        self.pos += len(data)
        return data


def normalize_wav(path: Path) -> None:
    """Force a clip to 16 kHz mono PCM16 in place.

    The Hub parquet claims 16 kHz in its feature metadata, but individual clips
    are whatever the corpus shipped (22.05 kHz is common), so this runs ffmpeg
    only when a clip actually needs it.
    """
    try:
        with wave.open(str(path), "rb") as w:
            ok = (
                w.getframerate() == SAMPLE_RATE
                and w.getnchannels() == 1
                and w.getsampwidth() == 2
            )
    except wave.Error:
        # Not a RIFF file at all (LibriSpeech ships FLAC inside the parquet),
        # so it always goes through ffmpeg.
        ok = False
    if ok:
        return

    tmp = path.with_suffix(".tmp.wav")
    try:
        subprocess.run(
            [
                "ffmpeg", "-y", "-loglevel", "error",
                "-i", str(path),
                "-ar", str(SAMPLE_RATE), "-ac", "1",
                "-c:a", "pcm_s16le",
                str(tmp),
            ],
            check=True,
        )
    except FileNotFoundError:
        raise SystemExit(
            "[fetch] ffmpeg is required to resample clips but was not found on PATH."
        )
    except subprocess.CalledProcessError as e:
        raise SystemExit(f"[fetch] ffmpeg failed on {path.name}: {e}")
    tmp.replace(path)


def duration_index(url: str) -> list[tuple[int, int, float]]:
    """(row_group, row_in_group, seconds) for a whole shard, without the audio.

    The duration column costs ~0.1 MB for an entire shard, which is what makes
    unbiased sampling affordable: we learn the real length distribution first,
    then fetch audio only for the rows we actually keep.
    """
    import pyarrow.parquet as pq

    pf = pq.ParquetFile(HttpRangeFile(url))
    out: list[tuple[int, int, float]] = []
    for rg in range(pf.metadata.num_row_groups):
        table = pf.read_row_group(rg, columns=["audio_length_s"])
        for i, seconds in enumerate(table.column("audio_length_s").to_pylist()):
            if seconds is not None:
                out.append((rg, i, float(seconds)))
    return out


def fetch(
    dataset: str,
    limit: int,
    min_seconds: float | None = None,
    max_seconds: float | None = None,
) -> int:
    """Stream a duration-stratified sample of `dataset` into bench/data/."""
    import pyarrow.parquet as pq

    spec = DATASETS[dataset]
    out = DATA / dataset
    out.mkdir(parents=True, exist_ok=True)

    print(f"[fetch] {dataset}: {spec['note']}")

    # 1. Index every row's duration across all shards (cheap, no audio).
    eligible: list[tuple[str, int, int, float]] = []
    for rel in spec["paths"]:
        for rg, row, seconds in duration_index(HF_BASE + rel):
            if min_seconds is not None and seconds < min_seconds:
                continue
            if max_seconds is not None and seconds > max_seconds:
                continue
            eligible.append((rel, rg, row, seconds))
    if not eligible:
        raise SystemExit(
            f"[fetch] no rows in {dataset} within "
            f"{min_seconds or 0}-{max_seconds or 'inf'}s"
        )
    print(
        f"[fetch] {len(eligible)} eligible rows across "
        f"{len(spec['paths'])} shard(s), "
        f"{sum(e[3] for e in eligible) / 60:.1f} min of audio total"
    )

    # 2. Sample at row-group granularity. Row groups are the unit of transfer,
    #    so picking evenly spaced *groups* and then a fair share of rows inside
    #    each gives an unbiased duration spread without downloading everything.
    groups: dict[tuple[str, int], list[tuple[int, float]]] = {}
    for rel, rg, row, seconds in eligible:
        groups.setdefault((rel, rg), []).append((row, seconds))

    # Row groups are duration-ordered, so ordering them by mean duration keeps
    # the even spacing meaningful.
    order = sorted(
        groups,
        key=lambda key: sum(d for _, d in groups[key]) / len(groups[key]),
    )
    per_group = 40
    n_groups = len(order)
    want_groups = max(1, min(n_groups, -(-limit // per_group)))
    step = n_groups / want_groups
    picked = [order[min(n_groups - 1, int(i * step))] for i in range(want_groups)]

    chosen: list[tuple[str, int, int, float]] = []
    share = max(1, -(-limit // want_groups))
    for key in picked:
        rows = sorted(groups[key])
        if len(rows) > share:
            stride = len(rows) / share
            rows = [rows[min(len(rows) - 1, int(j * stride))] for j in range(share)]
        for row, seconds in rows:
            chosen.append((key[0], key[1], row, seconds))
    chosen = chosen[:limit]
    print(f"[fetch] reading {want_groups} row group(s), {len(chosen)} rows")

    # 3. Read each needed row group once, with retries for flaky reads.
    needed: dict[tuple[str, int], list[int]] = {}
    for rel, rg, row, _ in chosen:
        needed.setdefault((rel, rg), []).append(row)

    remotes: dict[str, tuple[HttpRangeFile, "pq.ParquetFile"]] = {}
    taken = 0
    durations: list[float] = []

    for (rel, rg), row_indices in sorted(needed.items()):
        table = None
        for attempt in range(4):
            try:
                if rel not in remotes:
                    remote = HttpRangeFile(HF_BASE + rel)
                    remotes[rel] = (remote, pq.ParquetFile(remote))
                table = remotes[rel][1].read_row_group(rg, columns=["audio", "text"])
                break
            except Exception as e:
                if attempt == 3:
                    raise
                print(f"[fetch]   retry {rel} group {rg} ({type(e).__name__})")
                remotes.pop(rel, None)

        for row in sorted(row_indices):
            record = table.slice(row, 1).to_pylist()[0]
            audio = record.get("audio") or {}
            blob = audio.get("bytes")
            text = (record.get("text") or "").strip()
            if not blob or not text:
                continue

            stem = f"{dataset}-{taken:04d}"
            wav_path = out / f"{stem}.wav"
            wav_path.write_bytes(blob)
            normalize_wav(wav_path)
            (out / f"{stem}.txt").write_text(text + "\n", encoding="utf-8")
            durations.append(len(load_wav(wav_path)) / SAMPLE_RATE)
            taken += 1

    fetched = sum(r.bytes_fetched for r, _ in remotes.values())
    if taken == 0:
        raise SystemExit(f"[fetch] no rows found for {dataset}")

    durations.sort()
    mean = sum(durations) / len(durations)
    print(
        f"[fetch] wrote {taken} clips to {out}  "
        f"({fetched / 1e6:.0f} MB fetched)"
    )
    print(
        f"[fetch] clip length: mean {mean:.1f}s  "
        f"p10 {durations[len(durations) // 10]:.1f}s  "
        f"median {durations[len(durations) // 2]:.1f}s  "
        f"p90 {durations[(len(durations) * 9) // 10]:.1f}s  "
        f"max {durations[-1]:.1f}s"
    )
    return taken



# --------------------------------------------------------------------------
# Audio
# --------------------------------------------------------------------------


def load_wav(path: Path) -> list[float]:
    """Decode a PCM wave file to mono float32 samples in [-1, 1]."""
    with wave.open(str(path), "rb") as w:
        channels, width, rate, frames = (
            w.getnchannels(),
            w.getsampwidth(),
            w.getframerate(),
            w.getnframes(),
        )
        raw = w.readframes(frames)
    if rate != SAMPLE_RATE:
        raise SystemExit(
            f"[wav] {path.name} is {rate} Hz, expected {SAMPLE_RATE}. "
            "Convert it first: ffmpeg -i in.wav -ar 16000 -ac 1 out.wav"
        )

    if width == 2:
        pcm = array("h")
        pcm.frombytes(raw)
        floats = [s / 32768.0 for s in pcm]
    elif width == 4:
        pcm = array("f")
        pcm.frombytes(raw)
        floats = list(pcm)
    else:
        raise SystemExit(f"[wav] unsupported sample width {width} in {path.name}")

    if channels == 1:
        return floats
    # Down-mix interleaved channels.
    return [
        sum(floats[i : i + channels]) / channels
        for i in range(0, len(floats) - channels + 1, channels)
    ]


def encode_wav(samples: list[float]) -> bytes:
    """Mono 16 kHz PCM16 RIFF — byte-for-byte what transcription.rs uploads."""
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        pcm = array("h", (int(max(-1.0, min(1.0, s)) * 32767) for s in samples))
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


def rms(samples: list[float]) -> float:
    if not samples:
        return 0.0
    return (sum(s * s for s in samples) / len(samples)) ** 0.5


def normalize_gain(samples: list[float], target: float = 0.05, cap: float = 4.0) -> list[float]:
    """Mirror of transcription.rs `normalize_gain`: lift quiet speech, never clip."""
    level = rms(samples)
    if level <= 0.0:
        return list(samples)
    gain = min(target / level, cap)
    if gain <= 1.0:
        return list(samples)
    return [max(-1.0, min(1.0, s * gain)) for s in samples]


# --------------------------------------------------------------------------
# Engine
# --------------------------------------------------------------------------


def model_dir() -> Path:
    appdata = os.environ.get("APPDATA")
    if appdata:
        candidate = Path(appdata) / "SmoothFlow" / "models" / "stt" / "v2"
        if candidate.is_dir():
            return candidate
    fallback = Path.home() / "SmoothFlow" / "models" / "stt" / "v2"
    return fallback


class Helper:
    """Persistent smoothflow-stt.exe session.

    Wire format (mirrors src-tauri/src/stt_local.rs):
        start:  one JSON line, {"ready":true}
        request:  u32 LE sample count, then that many f32 LE samples
        reply:   one JSON line, {"text":"..."}
    """

    def __init__(self, threads: int | None = None, decoding: str | None = None):
        models = model_dir()
        files = {
            "encoder": models / "encoder.int8.onnx",
            "decoder": models / "decoder.int8.onnx",
            "joiner": models / "joiner.int8.onnx",
            "tokens": models / "tokens.txt",
        }
        missing = [n for n, p in files.items() if not p.is_file()]
        if missing:
            raise SystemExit(
                f"[engine] missing model file(s) {missing} in {models}. "
                "Enable offline mode in the app once to download them."
            )
        if not HELPER.is_file():
            raise SystemExit(f"[engine] helper not found: {HELPER}")

        if threads is None:
            threads = max(2, min(8, (os.cpu_count() or 4) - 1))

        argv = [
            str(HELPER),
            "--encoder", str(files["encoder"]),
            "--decoder", str(files["decoder"]),
            "--joiner", str(files["joiner"]),
            "--tokens", str(files["tokens"]),
            "--threads", str(threads),
        ]
        if decoding:
            argv += ["--decoding-method", decoding]

        self.proc = subprocess.Popen(
            argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.threads = threads
        self.decoding = decoding or "greedy_search"

        hello = self.proc.stdout.readline()
        if b'"ready":true' not in hello:
            raise SystemExit(f"[engine] helper did not load: {hello!r}")

    def transcribe(self, samples: list[float]) -> tuple[str, float]:
        """Return (text, wall seconds) for one utterance."""
        payload = struct.pack("<I", len(samples))
        payload += struct.pack(f"<{len(samples)}f", *samples)

        start = time.perf_counter()
        self.proc.stdin.write(payload)
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        elapsed = time.perf_counter() - start

        if not line:
            raise SystemExit("[engine] helper died mid-run")
        reply = json.loads(line.decode("utf-8", "replace"))
        if "error" in reply:
            raise SystemExit(f"[engine] {reply['error']}")
        return reply.get("text", ""), elapsed

    def close(self) -> None:
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()


def response_format(model: str) -> str:
    """Mirror of transcription.rs `response_format`."""
    verbose = ("whisper-large-v3", "whisper-large-v3-turbo", "whisper-1")
    return "verbose_json" if model.strip().lower() in verbose else "json"


# Whisper hallucinates these on silence/background noise. Same list, same gate,
# as transcription.rs — applied here so the cloud row measures the shipped
# pipeline rather than an idealized API call.
HALLUCINATION_PHRASES = (
    "thank you",
    "thank you for watching",
    "thank you very much",
    "thank you so much",
    "thanks for watching",
    "please subscribe",
    "like and subscribe",
    "subtitles by",
    "subtitles by the amara.org community",
    "you",
    "i think so",
)


def is_hallucination(text: str, no_speech_prob: float | None) -> bool:
    if no_speech_prob is None or no_speech_prob < 0.1:
        return False
    normalized = re.sub(r"^[^a-z0-9]+|[^a-z0-9]+$", "", text.lower())
    for phrase in HALLUCINATION_PHRASES:
        if normalized == phrase:
            return True
        if len(phrase.split()) >= 3 and normalized.startswith(phrase):
            return True
    return False


def default_prompt() -> str:
    """The prompt the app sends with an empty dictionary and no editor focus."""
    terms = sorted(
        {
            "gmail", "outlook", "yahoo", "hotmail", "aol",
            "email", "dot com", "dot org", "dot net",
        }
    )
    return f"Vocabulary hints for this audio: {', '.join(terms)}."


class CloudEngine:
    """Hosted transcription through the exact request the app sends.

    Mirrors src-tauri/src/transcription.rs: multipart POST to
    `{base_url}/audio/transcriptions` carrying file/model/response_format/
    language/prompt, with the app's `normalize_gain` applied first, its quiet-
    audio skip at rms < 0.0005, its 1 s between-calls gate, and its Whisper
    hallucination filter on the way out.

    Timing is wall-clock around the HTTP call, so it includes network latency —
    that is the number a user actually waits for.
    """

    is_cloud = True
    threads = 0

    def __init__(
        self,
        base_url: str,
        model: str,
        api_key: str,
        min_interval: float = 3.0,
        max_attempts: int = 6,
    ):
        if not api_key:
            raise SystemExit(
                f"[cloud] no API key. Set it in the environment, not in argv:\n"
                f"  $env:{CLOUD_KEY_ENV} = '<your key>'"
            )
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.api_key = api_key
        self.min_interval = min_interval
        self.max_attempts = max_attempts
        self.decoding = f"cloud:{model}"
        self.prompt = default_prompt()
        self._last = 0.0
        self.requests_made = 0

    def transcribe(self, samples: list[float]) -> tuple[str, float]:
        if rms(samples) < 0.0005:
            # transcription.rs skips these before ever reaching the network.
            return "", 0.0

        wav_bytes = encode_wav(normalize_gain(samples))
        if len(wav_bytes) > 25 * 1024 * 1024:
            raise SystemExit(
                f"[cloud] {len(wav_bytes) / 1e6:.0f} MB exceeds the 25 MB upload "
                "limit — this clip cannot be scored."
            )

        # The app refuses calls less than 1 s apart, and free tiers throttle
        # hard, so pace the calls. Pacing happens BEFORE timing starts:
        # otherwise the queue wait would be reported as model latency.
        gap = self.min_interval - (time.perf_counter() - self._last)
        if gap > 0:
            time.sleep(gap)
        self._last = time.perf_counter()

        return self._post(wav_bytes)

    def _post(self, wav_bytes: bytes) -> tuple[str, float]:
        """One request, with retries. Returns (text, seconds for the 200 reply).

        Only the successful attempt is timed, so a throttled retry cannot be
        mistaken for model latency.
        """
        import requests

        url = f"{self.base_url}/audio/transcriptions"
        delay = 2.0
        last_error = ""

        for attempt in range(1, self.max_attempts + 1):
            try:
                self.requests_made += 1
                t0 = time.perf_counter()
                resp = requests.post(
                    url,
                    headers={"Authorization": f"Bearer {self.api_key}"},
                    files={"file": ("audio.wav", wav_bytes, "audio/wav")},
                    data={
                        "model": self.model,
                        "response_format": response_format(self.model),
                        "language": "en",
                        "prompt": self.prompt,
                    },
                    timeout=90,
                )
                elapsed = time.perf_counter() - t0
            except Exception as e:                     # network-level failure
                last_error = f"{type(e).__name__}: {e}"
                if attempt == self.max_attempts:
                    raise SystemExit(f"[cloud] network error after {attempt} tries: {last_error}")
                print(f"[cloud]   network error, retrying in {delay:.0f}s ({last_error})")
                time.sleep(delay)
                delay = min(delay * 2, 60)
                continue

            if resp.status_code == 200:
                body = resp.json()
                text = body.get("text", "")
                segments = body.get("segments") or []
                prob = segments[0].get("no_speech_prob") if segments else None
                if is_hallucination(text, prob):
                    return "", elapsed
                return text, elapsed

            if resp.status_code in (401, 403):
                raise SystemExit(
                    f"[cloud] auth failed ({resp.status_code}). Check the key. "
                    f"{resp.text[:200]}"
                )
            if resp.status_code == 404:
                raise SystemExit(
                    f"[cloud] 404 for model {self.model!r} at {url}. "
                    "Pass --model or --base-url. "
                    f"{resp.text[:200]}"
                )
            if resp.status_code == 429 or resp.status_code >= 500:
                last_error = f"{resp.status_code}: {resp.text[:160]}"
                if attempt == self.max_attempts:
                    raise SystemExit(f"[cloud] gave up after {attempt} tries: {last_error}")
                wait = delay
                retry_after = resp.headers.get("Retry-After")
                if retry_after and retry_after.isdigit():
                    wait = max(wait, float(retry_after))
                print(f"[cloud]   {resp.status_code}, retrying in {wait:.0f}s")
                time.sleep(wait)
                delay = min(delay * 2, 60)
                continue

            raise SystemExit(
                f"[cloud] HTTP {resp.status_code} from {url}: {resp.text[:300]}"
            )

        raise SystemExit(f"[cloud] exhausted retries: {last_error}")

    def close(self) -> None:
        pass


# --------------------------------------------------------------------------
# Scoring
# --------------------------------------------------------------------------

_PUNCT = re.compile(r"[^a-z0-9'\s]")


def normalize(text: str) -> list[str]:
    """The single normalizer. Applied to both reference and hypothesis.

    Lowercase, strip punctuation, collapse whitespace, drop apostrophes so
    "don't" and "dont" cannot count as an error. Deliberately does NOT expand
    numbers or digits.
    """
    text = text.lower().replace("\u2019", "'").replace("\u2018", "'")
    text = _PUNCT.sub(" ", text)
    text = text.replace("'", " ")
    return text.split()


# Disfluency tokens. The earnings22 references are verbatim, so they contain
# every "uh" and "um". An engine that drops them is doing what a dictation user
# wants, but verbatim WER scores each drop as a deletion. Scoring a second time
# with fillers removed from BOTH sides separates content accuracy from filler
# handling — see README rule 7.
FILLERS = frozenset(
    {
        "uh", "uhh", "uhhh", "um", "umm", "ummm", "uhm", "er", "err", "erm",
        "ah", "ahh", "eh", "hmm", "hmmm", "mm", "mhm", "hunh", "ha", "huh",
        "oh", "ooh", "hm",
    }
)


def strip_fillers(tokens: list[str]) -> list[str]:
    return [w for w in tokens if w not in FILLERS]


def edit_counts(ref: list[str], hyp: list[str]) -> tuple[int, int, int, int]:
    """Levenshtein alignment on word sequences -> (subs, dels, ins, correct)."""
    n, m = len(ref), len(hyp)
    # dp[i][j] = (cost, subs, dels, ins)
    prev = [(j, 0, 0, j) for j in range(m + 1)]
    for i in range(1, n + 1):
        cur = [(i, 0, i, 0)] + [None] * m
        for j in range(1, m + 1):
            if ref[i - 1] == hyp[j - 1]:
                cur[j] = prev[j - 1]
            else:
                s = prev[j - 1]
                d = prev[j]
                ins = cur[j - 1]
                best = min((s[0] + 1, d[0] + 1, ins[0] + 1))
                if s[0] + 1 == best:
                    cur[j] = (best, s[1] + 1, s[2], s[3])
                elif d[0] + 1 == best:
                    cur[j] = (best, d[1], d[2] + 1, d[3])
                else:
                    cur[j] = (best, ins[1], ins[2], ins[3] + 1)
        prev = cur
    cost, subs, dels, ins = prev[m]
    return subs, dels, ins, (n - subs - dels)


def hard_tokens(raw_reference: str) -> list[str]:
    """Proper nouns and anything containing a digit.

    Captured before normalization, because capitalization is the only cheap
    signal for "this is a name" in these transcripts.
    """
    out = []
    for word in raw_reference.split():
        stripped = word.strip("\"'()[].,;:!?")
        if not stripped:
            continue
        if any(ch.isdigit() for ch in stripped) or (
            stripped[:1].isupper() and not stripped.isupper()
        ):
            out.append(stripped.lower().strip("\"'"))
    return out


def bootstrap_ci(subs: list[int], tot: list[int], rounds: int = 2000) -> tuple[float, float]:
    """95% CI for corpus WER, resampling utterances (clustered, not naive)."""
    if not subs:
        return (0.0, 0.0)
    rng = random.Random(1234)
    n = len(subs)
    values = []
    for _ in range(rounds):
        s = t = 0
        for _ in range(n):
            i = rng.randrange(n)
            s += subs[i]
            t += tot[i]
        if t:
            values.append(s / t)
    values.sort()
    lo = values[int(0.025 * len(values))]
    hi = values[int(0.975 * len(values)) - 1]
    return (lo, hi)


def score_pairs(pairs: list[tuple[str, str, str, float, float]]) -> dict:
    """Score (clip, reference, hypothesis, seconds, duration) tuples.

    Every stage goes through this one function — local engine, cloud, cleanup
    pass — so the rows are guaranteed comparable. If each stage had its own
    scoring loop they would drift and the comparison would be meaningless.
    """
    subs_l, dels_l, ins_l, tot_l = [], [], [], []
    f_subs = f_dels = f_ins = f_tot = 0
    audio_s = proc_s = 0.0
    exact = 0
    hard_hit = hard_all = 0
    rows = []

    for clip, ref_raw, hyp_text, elapsed, duration in pairs:
        ref = normalize(ref_raw)
        if not ref:
            continue
        hyp = normalize(hyp_text)

        s, d, i, _ = edit_counts(ref, hyp)
        subs_l.append(s)
        dels_l.append(d)
        ins_l.append(i)
        tot_l.append(len(ref))

        ref_clean = strip_fillers(ref)
        if ref_clean:
            fs, fd, fi, _ = edit_counts(ref_clean, strip_fillers(hyp))
            f_subs += fs
            f_dels += fd
            f_ins += fi
            f_tot += len(ref_clean)

        audio_s += duration
        proc_s += elapsed
        if s + d + i == 0:
            exact += 1

        hyp_bag: dict[str, int] = {}
        for w in hyp:
            hyp_bag[w] = hyp_bag.get(w, 0) + 1
        for w in hard_tokens(ref_raw):
            hard_all += 1
            if hyp_bag.get(w, 0) > 0:
                hard_hit += 1
                hyp_bag[w] -= 1

        errors = s + d + i
        rows.append(
            {
                "clip": clip,
                "seconds": round(duration, 2),
                "wer": round(errors / len(ref), 4),
                "subs": s, "dels": d, "ins": i,
                "reference": ref_raw,
                "hypothesis": hyp_text,
            }
        )

    total_words = sum(tot_l)
    lo, hi = bootstrap_ci(
        [s + d + i for s, d, i in zip(subs_l, dels_l, ins_l)], tot_l
    )
    return {
        "clips": len(tot_l),
        "words": total_words,
        "wer": (sum(subs_l) + sum(dels_l) + sum(ins_l)) / total_words if total_words else 0.0,
        "wer_ci95": [lo, hi],
        "errors": {"subs": sum(subs_l), "dels": sum(dels_l), "ins": sum(ins_l)},
        "wer_no_fillers": ((f_subs + f_dels + f_ins) / f_tot) if f_tot else None,
        "filler_words": total_words - f_tot,
        "exact_match": exact / len(tot_l) if tot_l else 0.0,
        "hard_token_recall": (hard_hit / hard_all) if hard_all else None,
        "hard_tokens": hard_all,
        "audio_seconds": round(audio_s, 1),
        "mean_clip_seconds": round(audio_s / len(tot_l), 2) if tot_l else 0.0,
        "words_per_clip": round(total_words / len(tot_l), 1) if tot_l else 0.0,
        "process_seconds": round(proc_s, 1),
        "speed_multiple": (audio_s / proc_s) if proc_s else 0.0,
        "rtf": (proc_s / audio_s) if audio_s else 0.0,
        "clips_detail": rows,
    }


def run(
    dataset: str,
    threads: int | None,
    verbose: bool,
    decoding: str | None = None,
    cloud: bool = False,
    base_url: str | None = None,
    model: str | None = None,
    max_clips: int = 0,
    min_interval: float = 3.0,
) -> dict:
    folder = DATA / dataset
    wavs = sorted(folder.glob("*.wav"))
    if not wavs:
        raise SystemExit(
            f"[run] no clips in {folder}. Run: python bench/bench.py fetch --dataset {dataset}"
        )
    if max_clips:
        wavs = wavs[:max_clips]

    if cloud:
        engine = CloudEngine(
            base_url or CLOUD_DEFAULT_BASE,
            model or CLOUD_DEFAULT_MODEL,
            os.environ.get(CLOUD_KEY_ENV, "").strip(),
            min_interval,
        )
        print(f"[run] cloud engine: {engine.model} at {engine.base_url}")
        print(f"[run] {len(wavs)} clips, key from ${CLOUD_KEY_ENV}\n")
    else:
        engine = Helper(threads, decoding)
        print(f"[run] helper up: {len(wavs)} clips, {engine.threads} threads, "
              f"{engine.decoding}\n")

    collected: list[tuple[str, str, str, float, float]] = []
    try:
        for idx, wav in enumerate(wavs, 1):
            ref_raw = wav.with_suffix(".txt").read_text(encoding="utf-8").strip()
            if not normalize(ref_raw):
                continue

            samples = load_wav(wav)
            duration = len(samples) / SAMPLE_RATE
            hyp_text, elapsed = engine.transcribe(samples)
            collected.append((wav.name, ref_raw, hyp_text, elapsed, duration))

            if verbose:
                ref, hyp = normalize(ref_raw), normalize(hyp_text)
                s, d, i, _ = edit_counts(ref, hyp)
                if s + d + i:
                    print(f"  {wav.name}  WER {(s + d + i) / len(ref):.0%}")
                    print(f"    ref: {ref_raw[:150]}")
                    print(f"    hyp: {hyp_text[:150]}")
            elif sys.stdout.isatty():
                print(f"\r[run] {idx}/{len(wavs)}", end="", flush=True)
    finally:
        engine.close()

    if not verbose and sys.stdout.isatty():
        print()

    is_cloud = getattr(engine, "is_cloud", False)
    result = score_pairs(collected)
    result.update(
        {
            "dataset": dataset,
            "engine": "cloud" if is_cloud else "local",
            "model": ((model or CLOUD_DEFAULT_MODEL) if is_cloud
                      else "parakeet-tdt-0.6b-v2-int8"),
            "decoding": engine.decoding,
            "threads": engine.threads,
        }
    )
    return result


# --------------------------------------------------------------------------
# Cleanup pass (the shipped Rust pipeline)
# --------------------------------------------------------------------------

# The cleanup pass is text_rules -> casual fast path -> local LLM ->
# faithfulness guard -> text_rules post. Re-implementing that here would
# measure this file instead of the app, so the harness shells out to a
# bench-only binary that calls the real `postprocess` function.
#
# Contract (see src-tauri/src/bin/cleanup_eval.rs):
#   in   {"clips": [{"clip": "<name>", "text": "<raw transcript>"}]}
#   out  {"engine": "local", "model_size": "4b",
#         "clips": [{"clip": "<name>", "text": "<cleaned>", "seconds": 0.0}]}
CLEANER_CANDIDATES = (
    REPO / "src-tauri" / "target" / "debug" / "cleanup_eval.exe",
    Path("D:/Temp/opencode/sf-target/debug/cleanup_eval.exe"),
    Path("/tmp/sf-target/debug/cleanup_eval.exe"),
    REPO / "src-tauri" / "target" / "release" / "cleanup_eval.exe",
)


def find_cleaner(explicit: str | None = None) -> Path:
    if explicit:
        path = Path(explicit)
        if not path.is_file():
            raise SystemExit(f"[clean] no binary at {path}")
        return path
    for candidate in CLEANER_CANDIDATES:
        if candidate.is_file():
            return candidate
    raise SystemExit(
        "[clean] cleanup_eval not found. Build it once:\n"
        "  cd src-tauri && cargo build --bin cleanup_eval\n"
        "or point at it with --cleaner <path>"
    )


def latest_local_result(dataset: str) -> dict:
    """Newest local-engine greedy run for `dataset` — the raw transcripts to clean.

    Filters on the parsed record rather than the filename, so a beam-search or
    cloud run can never be mistaken for the baseline.
    """
    best = None
    for path in RESULTS.glob(f"{dataset}-*.json"):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            continue
        if data.get("engine", "local") != "local":
            continue
        if data.get("decoding", "greedy_search") != "greedy_search":
            continue
        if best is None or path.stat().st_mtime > best[0]:
            best = (path.stat().st_mtime, data)
    if best is None:
        raise SystemExit(
            f"[clean] no local results for {dataset}. Run: "
            f"python bench/bench.py run --dataset {dataset}"
        )
    return best[1]


def clean(
    dataset: str,
    cleaner: str | None,
    cleanup_engine: str,
    model_size: str,
    source: str | None = None,
) -> dict:
    binary = find_cleaner(cleaner)
    base = (
        json.loads(Path(source).read_text(encoding="utf-8"))
        if source
        else latest_local_result(dataset)
    )

    clips = [
        {"clip": r["clip"], "text": r["hypothesis"]}
        for r in base["clips_detail"]
    ]
    durations = {r["clip"]: r["seconds"] for r in base["clips_detail"]}
    references = {r["clip"]: r["reference"] for r in base["clips_detail"]}

    payload = RESULTS / f".{dataset}-cleanup-in.json"
    output = RESULTS / f".{dataset}-cleanup-out.json"
    RESULTS.mkdir(parents=True, exist_ok=True)
    payload.write_text(json.dumps({"clips": clips}), encoding="utf-8")

    print(f"[clean] {binary.name}: {len(clips)} transcripts, "
          f"engine={cleanup_engine}, model_size={model_size}")
    argv = [
        str(binary),
        "--in", str(payload),
        "--out", str(output),
        "--engine", cleanup_engine,
        "--model-size", model_size,
    ]
    proc = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8")
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr or "")
        raise SystemExit(f"[clean] cleanup_eval exited {proc.returncode}")

    cleaned = json.loads(output.read_text(encoding="utf-8"))
    by_clip = {c["clip"]: c for c in cleaned.get("clips", [])}
    missing = [c["clip"] for c in clips if c["clip"] not in by_clip]
    if missing:
        raise SystemExit(
            f"[clean] {len(missing)} clips came back unprocessed, e.g. {missing[:3]}"
        )

    pairs = [
        (
            c["clip"],
            references[c["clip"]],
            by_clip[c["clip"]].get("text", ""),
            float(by_clip[c["clip"]].get("seconds", 0.0)),
            durations[c["clip"]],
        )
        for c in clips
    ]
    result = score_pairs(pairs)
    result.update(
        {
            "dataset": dataset,
            "engine": "cleanup",
            "cleanup_engine": cleaned.get("engine", cleanup_engine),
            "model": cleaned.get("model_size", model_size),
            "decoding": f"cleanup:{cleaned.get('engine', cleanup_engine)}",
            "threads": 0,
        }
    )

    for suffix in (payload, output):
        try:
            suffix.unlink()
        except OSError:
            pass
    return result


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------

# Published reference points, for context only. Measured by Superwhisper on the
# same Open ASR Leaderboard audio, with the competitor's own cleanup pass off so
# the transcript can be scored word for word. Source: superwhisper.com/benchmarks,
# updated 2026-09-03.
#
# WER and recall only: their speeds come from an Apple M4 (local rows) or a
# hosted API, so they are not comparable to a CPU number and are left out.
# "Parakeet V2 in Superwhisper" is the same model SmoothFlow ships, which makes
# librispeech-other a direct control on this harness.
REFERENCE = {
    "librispeech-other": [
        ("Parakeet V2 in Superwhisper", 4.1, None),
        ("OpenAI GPT Transcribe", 3.3, None),
    ],
    "librispeech-clean": [
        ("Wispr Flow (raw)", 3.8, None),
    ],
    "earnings22": [
        ("Parakeet V2 in Superwhisper", 11.1, None),
        ("OpenAI GPT Transcribe", 8.4, None),
    ],
}


def latest_results() -> dict[str, dict]:
    """Newest saved result per dataset."""
    found: dict[str, dict] = {}
    if not RESULTS.is_dir():
        return found
    for path in sorted(RESULTS.glob("*.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            continue
        # Keep greedy, beam-search and cloud runs side by side instead of
        # letting the newest one overwrite the others.
        if data.get("engine") == "cloud":
            label = f"{data['dataset']} [cloud]"
        elif data.get("engine") == "cleanup":
            label = f"{data['dataset']} [clean:{data.get('cleanup_engine', '?')}]"
        else:
            decoding = data.get("decoding", "greedy_search")
            label = (
                data["dataset"]
                if decoding == "greedy_search"
                else f"{data['dataset']} [{decoding.split('_')[0]}]"
            )
        found[label] = data
    return found


def compare() -> None:
    results = latest_results()
    if not results:
        raise SystemExit("[compare] no results yet. Run: python bench/bench.py run")

    header = (
        f"{'dataset':<22}{'clips':>6}{'sec/clip':>10}{'words':>8}{'WER':>8}"
        f"{'95% CI':>14}{'no-filler':>11}{'exact':>8}{'recall':>9}{'speed':>8}"
    )
    print()
    print("SmoothFlow — raw STT, no cleanup pass")
    print(header)
    print("-" * len(header))
    for name in sorted(results):
        r = results[name]
        lo, hi = r["wer_ci95"]
        recall = (
            f"{r['hard_token_recall'] * 100:.1f}%"
            if r.get("hard_token_recall") is not None
            else "-"
        )
        nofill = (
            f"{r['wer_no_fillers'] * 100:.1f}%"
            if r.get("wer_no_fillers") is not None
            else "-"
        )
        # Speed is audio-per-second for the engines; for the cleanup stage it
        # would mean "how fast can text be rewritten", which is not a thing.
        speed = ("-" if r.get("engine") == "cleanup"
                 else f"{r['speed_multiple']:.1f}x")
        print(
            f"{name:<22}{r['clips']:>6}{r.get('mean_clip_seconds', 0):>10.1f}"
            f"{r['words']:>8}"
            f"{r['wer'] * 100:>7.1f}%{f'{lo * 100:.1f}-{hi * 100:.1f}%':>14}"
            f"{nofill:>11}"
            f"{r['exact_match'] * 100:>7.1f}%{recall:>9}"
            f"{speed:>8}"
        )
    print("-" * len(header))

    # Head-to-head on identical audio: same clips, same normalizer, same
    # scorer. The only difference is the engine.
    for dataset in sorted({r["dataset"] for r in results.values()}):
        local = next(
            (
                r for k, r in results.items()
                if r["dataset"] == dataset and r.get("engine", "local") == "local"
                and r.get("decoding", "greedy_search") == "greedy_search"
            ),
            None,
        )
        cloud = next(
            (r for k, r in results.items() if r["dataset"] == dataset and r.get("engine") == "cloud"),
            None,
        )
        if not (local and cloud):
            continue
        print(f"\noffline vs cloud on {dataset}, identical clips:")
        for label, r in (("offline (local)", local), ("cloud", cloud)):
            lo, hi = r["wer_ci95"]
            nofill = (
                f"{r['wer_no_fillers'] * 100:>5.1f}%"
                if r.get("wer_no_fillers") is not None
                else "    -"
            )
            print(
                f"  {label:<18} WER {r['wer'] * 100:>5.1f}%  "
                f"[{lo * 100:.1f}-{hi * 100:.1f}]  no-filler {nofill}  "
                f"exact {r['exact_match'] * 100:>5.1f}%  "
                f"speed {r['speed_multiple']:>5.1f}x"
            )
        delta = (local["wer"] - cloud["wer"]) * 100
        overlap = local["wer_ci95"][0] <= cloud["wer_ci95"][1] and cloud["wer_ci95"][0] <= local["wer_ci95"][1]
        verdict = (
            "indistinguishable" if overlap
            else ("offline is better" if delta < 0 else "cloud is better")
        )
        print(f"  {'difference':<18} {delta:+.1f} points offline-minus-cloud — {verdict}")

        # Corpus WER is micro-averaged: one bad long clip can decide it. The
        # paired view answers "which one would you rather have on THIS clip".
        local_by = {r["clip"]: r for r in local["clips_detail"]}
        shared = [r for r in cloud["clips_detail"] if r["clip"] in local_by]
        if shared:
            wins = sum(
                1 for r in shared if local_by[r["clip"]]["wer"] < r["wer"] - 1e-9
            )
            ties = sum(
                1 for r in shared if abs(local_by[r["clip"]]["wer"] - r["wer"]) <= 1e-9
            )
            losses = len(shared) - wins - ties
            macro_local = sum(local_by[r["clip"]]["wer"] for r in shared) / len(shared)
            macro_cloud = sum(r["wer"] for r in shared) / len(shared)
            print(
                f"  {'per clip':<18} offline better on {wins}/{len(shared)}, "
                f"cloud better on {losses}, tied {ties}"
            )
            print(
                f"  {'mean clip WER':<18} offline {macro_local * 100:.1f}% vs "
                f"cloud {macro_cloud * 100:.1f}%   (corpus numbers above are "
                f"word-weighted, not clip-weighted)"
            )

    # What the cleanup pass is actually worth: raw engine vs rules-only vs
    # rules + the local model, all on the same raw transcripts.
    for dataset in sorted({r["dataset"] for r in results.values()}):
        def pick(kind, cleanup_engine=None):
            return next(
                (
                    r for r in results.values()
                    if r["dataset"] == dataset
                    and r.get("engine") == kind
                    and (cleanup_engine is None
                         or r.get("cleanup_engine") == cleanup_engine)
                ),
                None,
            )

        raw = pick("local")
        rules = pick("cleanup", "off")
        llm = pick("cleanup", "local")
        if not raw or not (rules or llm):
            continue
        stages = [("raw engine", raw)]
        if rules:
            stages.append(("+ rules only", rules))
        if llm:
            stages.append((f"+ rules + Qwen {llm.get('model', '')}", llm))
        print(f"\nwhat cleanup buys on {dataset} (same raw transcripts):")
        for label, r in stages:
            lo, hi = r["wer_ci95"]
            recall = (
                f"{r['hard_token_recall'] * 100:>5.1f}%"
                if r.get("hard_token_recall") is not None
                else "    -"
            )
            print(
                f"  {label:<26} WER {r['wer'] * 100:>5.1f}%  "
                f"[{lo * 100:.1f}-{hi * 100:.1f}]  "
                f"exact {r['exact_match'] * 100:>5.1f}%  recall {recall}"
            )
        print("  exact = clips needing zero edits after the whole pipeline.")

    refs = [(ds, row) for ds, rows in REFERENCE.items() if ds in results for row in rows]
    if refs:
        print("\npublished reference on the same audio (competitor cleanup off):")
        for dataset, (label, wer, recall) in refs:
            recall_txt = f"   recall {recall:.1f}%" if recall is not None else ""
            ours = results[dataset]["wer"] * 100
            delta = wer - ours
            verdict = "we are better" if delta > 0.3 else ("they are better" if delta < -0.3 else "level")
            print(
                f"  {label:<30} on {dataset:<18} "
                f"WER {wer:>5.1f}%  ({verdict} by {abs(delta):.1f}){recall_txt}"
            )
        print("  speed is not shown: their numbers are Apple M4 or hosted, ours is CPU.")
    print()
    print("WER = (substitutions + deletions + insertions) / reference words.")
    print("exact = clips with zero word errors. Lower bound: no LLM cleanup pass.")
    print()


def show(result: dict) -> None:
    lo, hi = result["wer_ci95"]
    if result.get("engine") == "cloud":
        title = f"{result['dataset']}  —  cloud {result.get('model', '')}, raw, no cleanup"
    elif result.get("engine") == "cleanup":
        engine = result.get("cleanup_engine", "?")
        detail = (f"rules + Qwen {result['model']}" if engine == "local"
                  else "deterministic rules only")
        title = f"{result['dataset']}  —  after cleanup: {detail}"
    else:
        title = (f"{result['dataset']}  —  raw Parakeet v2 int8, "
                 f"{result['threads']} threads, {result.get('decoding', 'greedy_search')}")
    print()
    print("=" * 66)
    print(f"  {title}")
    print("=" * 66)
    print(f"  clips / words          {result['clips']} / {result['words']}")
    print(f"  clip length            {result['mean_clip_seconds']:.1f}s mean, "
          f"{result['words_per_clip']:.1f} words/clip")
    print(f"  WER                    {result['wer'] * 100:.1f}%   "
          f"(95% CI {lo * 100:.1f}-{hi * 100:.1f}%)")
    print(f"    substitutions        {result['errors']['subs']}")
    print(f"    deletions            {result['errors']['dels']}")
    print(f"    insertions           {result['errors']['ins']}")
    print(f"  exact match            {result['exact_match'] * 100:.1f}%  "
          f"(raw, no cleanup pass)")
    if result.get("wer_no_fillers") is not None:
        print(f"  WER without fillers    {result['wer_no_fillers'] * 100:.1f}%  "
              f"({result['filler_words']} uh/um removed from both sides)")
    if result["hard_token_recall"] is not None:
        print(f"  name/number recall     {result['hard_token_recall'] * 100:.1f}%  "
              f"({result['hard_tokens']} tokens)")
    print(f"  audio / decode         {result['audio_seconds']}s / "
          f"{result['process_seconds']}s")
    print(f"  speed                  {result['speed_multiple']:.1f}x realtime  "
          f"(RTF {result['rtf']:.3f})")
    if result.get("engine") == "cloud" and result["clips"]:
        print(f"  wait for the text      "
              f"{result['process_seconds'] / result['clips']:.2f}s per clip "
              f"(network included)")
    if result.get("engine") == "cleanup" and result["clips"]:
        print(f"  added by cleanup       "
              f"{result['process_seconds'] / result['clips']:.2f}s per clip "
              f"({result['process_seconds']}s total)")
    print("=" * 66)
    print()


def main() -> None:
    ap = argparse.ArgumentParser(
        description="Benchmark SmoothFlow's local speech recognition.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="datasets:\n"
        + "\n".join(f"  {k:<20} {v['note']}" for k, v in DATASETS.items()),
    )
    ap.add_argument("stage", choices=["fetch", "run", "clean", "compare"])
    ap.add_argument("--dataset", default="earnings22", choices=sorted(DATASETS))
    ap.add_argument("--limit", type=int, default=200, help="clips to fetch")
    ap.add_argument(
        "--min-seconds", type=float, default=None,
        help="drop clips shorter than this (use 8 with --max-seconds 15 to "
             "match dictation length)",
    )
    ap.add_argument(
        "--max-seconds", type=float, default=None,
        help="drop clips longer than this",
    )
    ap.add_argument("--threads", type=int, default=None, help="decode threads")
    ap.add_argument(
        "--decoding-method", default=None,
        choices=["greedy_search", "modified_beam_search"],
        help="decoder to use (default: the helper's greedy_search)",
    )
    ap.add_argument(
        "--cloud", action="store_true",
        help=f"score a hosted endpoint instead of the local engine. The key is "
             f"read from ${CLOUD_KEY_ENV}, never from argv.",
    )
    ap.add_argument(
        "--base-url", default=None,
        help=f"cloud base URL (default {CLOUD_DEFAULT_BASE})",
    )
    ap.add_argument(
        "--model", default=None,
        help=f"cloud model name (default {CLOUD_DEFAULT_MODEL})",
    )
    ap.add_argument("--verbose", action="store_true", help="print every error")
    ap.add_argument(
        "--max-clips", type=int, default=0,
        help="score only the first N clips (0 = all); useful for a smoke test",
    )
    ap.add_argument(
        "--min-interval", type=float, default=3.0,
        help="cloud only: seconds to leave between requests (default 3 = 20/min). "
             "Lower it if your key allows more.",
    )
    ap.add_argument(
        "--cleanup-engine", default="local", choices=["off", "local"],
        help="clean stage: 'off' = deterministic rules only, 'local' = rules + "
             "Qwen. Both call the app's real postprocess function.",
    )
    ap.add_argument(
        "--model-size", default="4b", choices=["2b", "4b", "9b"],
        help="clean stage: which local LLM to use (must be downloaded)",
    )
    ap.add_argument(
        "--source", default=None,
        help="clean stage: score this results JSON instead of the newest local run",
    )
    ap.add_argument(
        "--cleaner", default=None,
        help="path to the cleanup_eval binary (default: search target/debug)",
    )
    args = ap.parse_args()

    if args.stage == "compare":
        compare()
        return

    if args.stage == "fetch":
        fetch(args.dataset, args.limit, args.min_seconds, args.max_seconds)
        return

    if args.stage == "clean":
        result = clean(
            args.dataset,
            args.cleaner,
            args.cleanup_engine,
            args.model_size,
            args.source,
        )
        show(result)
        RESULTS.mkdir(parents=True, exist_ok=True)
        stamp = time.strftime("%Y%m%d-%H%M%S")
        out = RESULTS / f"{args.dataset}-clean-{result['cleanup_engine']}-{stamp}.json"
        out.write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(f"saved {out.relative_to(REPO)}")
        return

    result = run(
        args.dataset,
        args.threads,
        args.verbose,
        args.decoding_method,
        args.cloud,
        args.base_url,
        args.model,
        args.max_clips,
        args.min_interval,
    )
    show(result)

    RESULTS.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    if result.get("engine") == "cloud":
        suffix = "-cloud"
    else:
        decoding = result.get("decoding", "greedy_search")
        suffix = "" if decoding == "greedy_search" else f"-{decoding.split('_')[0]}"
    out = RESULTS / f"{args.dataset}{suffix}-{stamp}.json"
    out.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(f"saved {out.relative_to(REPO)}")


if __name__ == "__main__":
    main()
