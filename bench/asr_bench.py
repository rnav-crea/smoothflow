#!/usr/bin/env python3
"""Side-by-side ASR bench: local Parakeet (sherpa-onnx Python) vs Phonon (fermion).

Reuses bench.py scoring (normalize / edit_counts via score_pairs) so numbers
are comparable to bench/bench.py runs. Reports WER, latency and speed per
engine on the same clips.

Usage:
  python bench/asr_bench.py --dataset librispeech-clean --limit 50
  python bench/asr_bench.py --dataset earnings22 --limit 50

Phonon is loaded via the `fermion` Python package (pip install fermion-research).
First run downloads the model (~156 MB). Set FERMION_MODEL to pick a variant
(default: phonon-2.5-en).
"""
from __future__ import annotations

import argparse
import importlib.util
import os
import sys
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent
REPO = BENCH.parent
DATA = BENCH / "data"

# Reuse bench.py normalizer/scorer verbatim.
spec = importlib.util.spec_from_file_location("bench", BENCH / "bench.py")
bench = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
sys.modules["bench"] = bench
spec.loader.exec_module(bench)  # type: ignore[union-attr]

SAMPLE_RATE = 16000


def load_wave(path: Path):
    import soundfile as sf

    audio, sr = sf.read(str(path), dtype="float32", always_2d=True)
    if sr != SAMPLE_RATE:
        raise SystemExit(f"[wav] {path.name} is {sr} Hz, expected {SAMPLE_RATE}")
    return audio[:, 0].tolist(), len(audio) / sr


# --------------------------------------------------------------- Parakeet

class ParakeetEngine:
    name = "parakeet-v2-sherpa"

    def __init__(self, threads: int = 6):
        import sherpa_onnx

        appdata = os.environ.get("APPDATA")
        models = Path(appdata) / "SmoothFlow" / "models" / "stt" / "v2"
        if not models.is_dir():
            raise SystemExit(f"[parakeet] model dir not found: {models}")
        self.rec = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=str(models / "encoder.int8.onnx"),
            decoder=str(models / "decoder.int8.onnx"),
            joiner=str(models / "joiner.int8.onnx"),
            tokens=str(models / "tokens.txt"),
            num_threads=threads,
            model_type="nemo_transducer",
            feature_dim=80,
            decoding_method="greedy_search",
            provider="cpu",
        )

    def transcribe(self, samples):
        import numpy as np

        arr = np.asarray(samples, dtype=np.float32)
        stream = self.rec.create_stream()
        t0 = time.perf_counter()
        stream.accept_waveform(SAMPLE_RATE, arr)
        self.rec.decode_stream(stream)
        return stream.result.text, time.perf_counter() - t0


# ------------------------------------------------------------------ Phonon

class PhononEngine:
    """Phonon via the `fermion transcribe` CLI (the only public API).

    The `fermion` package has no Python Transcriber class — transcription is
    the `fermion transcribe MODEL FILE` command. We write the 16 kHz mono
    samples to a temp WAV and call it, exactly like a user would.

    Known cost (see bench/phonon-notes.md): the CLI reloads the model on every
    call (~2.4s load), so per-clip latency is dominated by startup, not decode.
    That is the honest number for CLI-driven use, and the reason a resident
    `fermion serve` would be needed for production — flagged, not hidden.
    """

    def __init__(self, model: str, threads: int = 6):
        import shutil
        import tempfile

        self.model_name = model
        self.name = f"phonon:{model}"
        self.threads = threads
        self._tmpdir = Path(tempfile.mkdtemp(prefix="phonon-bench-"))
        self._counter = 0
        self._binary = shutil.which("fermion")
        if self._binary is None:
            raise SystemExit("[phonon] `fermion` not on PATH — pip install fermion-research")
        # Download + verify the model once, so the first timed clip isn't the fetch.
        print("[phonon] downloading/verifying model...")
        import subprocess

        pre = subprocess.run(
            [self._binary, "transcribe", model, "--download-only"],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        if pre.returncode != 0:
            raise SystemExit(f"[phonon] model fetch failed: {(pre.stderr or '')[:500]}")

    def transcribe(self, samples):
        import subprocess

        self._counter += 1
        wav_path = self._tmpdir / f"clip-{self._counter}.wav"
        import wave
        from array import array

        with wave.open(str(wav_path), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(SAMPLE_RATE)
            pcm = array("h", (int(max(-1.0, min(1.0, s)) * 32767) for s in samples))
            w.writeframes(pcm.tobytes())

        t0 = time.perf_counter()
        proc = subprocess.run(
            [self._binary, "transcribe", self.model_name, str(wav_path),
             "--threads", str(self.threads)],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        elapsed = time.perf_counter() - t0
        try:
            wav_path.unlink()
        except OSError:
            pass
        if proc.returncode != 0:
            raise SystemExit(
                f"[phonon] fermion exited {proc.returncode}: "
                f"{(proc.stderr or '')[:500]}"
            )
        text = (proc.stdout or "").strip()
        return text, elapsed


# -------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dataset", default="librispeech-clean")
    ap.add_argument("--limit", type=int, default=50)
    ap.add_argument("--limit-seconds", type=float, default=0.0,
                    help="skip clips longer than this (0 = no cap)")
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--skip-phonon", action="store_true")
    args = ap.parse_args()

    folder = DATA / args.dataset
    wavs = sorted(folder.glob("*.wav"))
    if not wavs:
        raise SystemExit(f"[run] no clips in {folder}")
    if args.limit_seconds:
        lens = []
        for w in wavs:
            import soundfile as sf

            info = sf.info(str(w))
            lens.append((w, info.duration))
        wavs = [w for w, d in lens if d <= args.limit_seconds]
    wavs = wavs[: args.limit]

    print(f"[bench] {args.dataset}: {len(wavs)} clips")
    clips = []
    for wav in wavs:
        ref = wav.with_suffix(".txt").read_text(encoding="utf-8").strip()
        samples, duration = load_wave(wav)
        clips.append((wav.name, ref, samples, duration))

    engines = [("parakeet-v2", ParakeetEngine(args.threads))]
    if not args.skip_phonon:
        model = os.environ.get("FERMION_MODEL", "phonon-2.5-en")
        try:
            engines.append((f"phonon:{model}", PhononEngine(model)))
        except Exception as e:
            print(f"[phonon] unavailable ({type(e).__name__}: {e}); running parakeet only")

    for label, engine in engines:
        pairs = []
        for name, ref, samples, duration in clips:
            hyp, elapsed = engine.transcribe(samples)
            pairs.append((name, ref, hyp, elapsed, duration))
        result = bench.score_pairs(pairs)
        print(f"\n=== {label} ===")
        print(f"  WER: {result['wer']:.2%}  (CI {result['wer_ci95'][0]:.2%}–{result['wer_ci95'][1]:.2%})")
        print(f"  exact match: {result['exact_match']:.1%}  hard-token recall: {result['hard_token_recall']}")
        print(f"  speed: {result['speed_multiple']:.1f}x realtime  (RTF {result['rtf']:.3f})")
        print(f"  process: {result['process_seconds']}s for {result['audio_seconds']}s audio")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
