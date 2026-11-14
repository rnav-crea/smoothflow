use reqwest::blocking::Client;
use reqwest::header::RANGE;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Shared by every transfer (engines and models): set by the user's Cancel,
/// and checked between chunks so a multi-GB download stops within about a
/// second. The `.part` file is deliberately kept on a cancel — resuming costs
/// one HTTP Range request and nothing already fetched is thrown away.
pub static TRANSFER_CANCELLED: AtomicBool = AtomicBool::new(false);

/// The running model transfer's percent, 0..=100, and whether one is running.
///
/// This is the event-free twin of `model-group-download-progress`: the dialog
/// polls it twice a second, so a missed or late event can never leave the user
/// staring at a still bar while gigabytes move. Written by the download loop
/// only (`download_model` owns the active flag).
static TRANSFER_PERCENT: AtomicU32 = AtomicU32::new(0);
static TRANSFER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// What the UI polls: `active` false means "no transfer, ignore percent".
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TransferProgress {
    pub active: bool,
    pub percent: u32,
}

pub fn progress_snapshot() -> TransferProgress {
    TransferProgress {
        active: TRANSFER_ACTIVE.load(Ordering::Relaxed),
        percent: TRANSFER_PERCENT.load(Ordering::Relaxed),
    }
}

/// Mark a transfer as running (or finished) and reset the percent when it ends.
pub fn set_running(active: bool) {
    TRANSFER_ACTIVE.store(active, Ordering::Relaxed);
    if !active {
        TRANSFER_PERCENT.store(0, Ordering::Relaxed);
    }
}

fn set_percent(percent: Option<f32>) {
    let value = percent.unwrap_or(0.0).clamp(0.0, 100.0);
    TRANSFER_PERCENT.store(value as u32, Ordering::Relaxed);
}

pub fn request_cancel() {
    TRANSFER_CANCELLED.store(true, Ordering::Relaxed);
}

pub fn cancel_requested() -> bool {
    TRANSFER_CANCELLED.load(Ordering::Relaxed)
}

/// Clear at the start of a fresh transfer, so a leftover flag from a cancelled
/// transfer cannot kill the next one before its first byte. Never call this
/// from inside a running transfer — the running one owns its flag.
pub fn reset_cancel() {
    TRANSFER_CANCELLED.store(false, Ordering::Relaxed);
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub id: &'static str,
    pub category: &'static str,
    pub directory: &'static str,
    pub repository: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub expected_size: u64,
    pub expected_sha256: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub id: &'static str,
    pub category: &'static str,
    pub directory: &'static str,
    pub filename: &'static str,
    pub expected_size: u64,
    pub installed: bool,
    pub verified: bool,
    pub installed_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub files: Vec<FileStatus>,
    pub installed_files: usize,
    pub missing_files: usize,
    pub total_bytes: u64,
    pub expected_total_bytes: u64,
}

const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "parakeet-v2-encoder",
        category: "stt",
        directory: "v2",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        filename: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/encoder.int8.onnx",
        expected_size: 652_184_296,
        expected_sha256: Some("a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab"),
    },
    ModelSpec {
        id: "parakeet-v2-decoder",
        category: "stt",
        directory: "v2",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        filename: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/decoder.int8.onnx",
        expected_size: 7_257_753,
        expected_sha256: Some("b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e"),
    },
    ModelSpec {
        id: "parakeet-v2-joiner",
        category: "stt",
        directory: "v2",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        filename: "joiner.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/joiner.int8.onnx",
        expected_size: 1_739_080,
        expected_sha256: Some("7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2"),
    },
    ModelSpec {
        id: "parakeet-v2-tokens",
        category: "stt",
        directory: "v2",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/tokens.txt",
        expected_size: 9_384,
        expected_sha256: Some("ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d"),
    },
    ModelSpec {
        id: "parakeet-v3-encoder",
        category: "stt",
        directory: "v3",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        filename: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/encoder.int8.onnx",
        expected_size: 652_184_281,
        expected_sha256: Some("acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247"),
    },
    ModelSpec {
        id: "parakeet-v3-decoder",
        category: "stt",
        directory: "v3",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        filename: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/decoder.int8.onnx",
        expected_size: 11_845_275,
        expected_sha256: Some("179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e"),
    },
    ModelSpec {
        id: "parakeet-v3-joiner",
        category: "stt",
        directory: "v3",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        filename: "joiner.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/joiner.int8.onnx",
        expected_size: 6_355_277,
        expected_sha256: Some("3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3"),
    },
    ModelSpec {
        id: "parakeet-v3-tokens",
        category: "stt",
        directory: "v3",
        repository: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/tokens.txt",
        expected_size: 93_939,
        expected_sha256: Some("d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
    },
    ModelSpec {
        id: "qwen-2b",
        category: "llm",
        directory: "2b",
        repository: "unsloth/Qwen3.5-2B-GGUF",
        filename: "Qwen3.5-2B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf",
        expected_size: 1_280_835_840,
        expected_sha256: Some("aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223"),
    },
    ModelSpec {
        id: "qwen-4b",
        category: "llm",
        directory: "4b",
        repository: "unsloth/Qwen3.5-4B-GGUF",
        filename: "Qwen3.5-4B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
        expected_size: 2_740_937_888,
        expected_sha256: Some("00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"),
    },
    ModelSpec {
        id: "qwen-9b",
        category: "llm",
        directory: "9b",
        repository: "unsloth/Qwen3.5-9B-GGUF",
        filename: "Qwen3.5-9B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
        expected_size: 5_680_522_464,
        expected_sha256: Some("03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8"),
    },
    // ── Additional STT models ────────────────────────────────────────────────
    // Sizes below were read from the HuggingFace API (`?blobs=true`) on
    // 2026-11-14 and must stay exact: `status()` compares
    // `installed_bytes == expected_size`, so an estimated number makes a good
    // download look corrupt. SHA-256 comes from the LFS oid HuggingFace
    // publishes per file and `download()` refuses a spec without one, so every
    // row here must carry a real hash — spot-verified against a downloaded file
    // (unified joiner, 1_735_860 bytes) before being written down.
    ModelSpec {
        id: "parakeet-unified-encoder",
        category: "stt",
        directory: "parakeet-unified-en",
        repository: "csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming",
        filename: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/main/encoder.int8.onnx",
        expected_size: 654_040_552,
        expected_sha256: Some("6716910b7a0833997fec7a410494c995d70124001a0e9b66d6370d6aced577e0"),
    },
    ModelSpec {
        id: "parakeet-unified-decoder",
        category: "stt",
        directory: "parakeet-unified-en",
        repository: "csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming",
        filename: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/main/decoder.int8.onnx",
        expected_size: 7_257_753,
        expected_sha256: Some("a5e223392c90e75f8144cdb5eb95af7625db389e39edef2bd1a9c872b3298fe6"),
    },
    ModelSpec {
        id: "parakeet-unified-joiner",
        category: "stt",
        directory: "parakeet-unified-en",
        repository: "csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming",
        filename: "joiner.int8.onnx",
        url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/main/joiner.int8.onnx",
        expected_size: 1_735_860,
        expected_sha256: Some("869f43f7d24595c55581ad3bf249a935fb8a71389fbdaa7504b9f46f93140f8a"),
    },
    ModelSpec {
        id: "parakeet-unified-tokens",
        category: "stt",
        directory: "parakeet-unified-en",
        repository: "csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming",
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming/resolve/main/tokens.txt",
        expected_size: 8_952,
        expected_sha256: Some("dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff"),
    },
    ModelSpec {
        id: "canary-180m-encoder",
        category: "stt",
        directory: "canary-180m-flash",
        repository: "csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8",
        filename: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8/resolve/main/encoder.int8.onnx",
        expected_size: 132_678_643,
        expected_sha256: Some("7a75b4e2a5857a6dcc0819503bbe3fad66943db4a3ccf21d3f27c633667d303f"),
    },
    ModelSpec {
        id: "canary-180m-decoder",
        category: "stt",
        directory: "canary-180m-flash",
        repository: "csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8",
        filename: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8/resolve/main/decoder.int8.onnx",
        expected_size: 74_437_848,
        expected_sha256: Some("e41a2ab9c0c2fe81a1e8ade5a45fb02a74bc4db7d1f91b89a54a25e2cf79cba2"),
    },
    ModelSpec {
        id: "canary-180m-tokens",
        category: "stt",
        directory: "canary-180m-flash",
        repository: "csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8",
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8/resolve/main/tokens.txt",
        expected_size: 53_555,
        expected_sha256: Some("2dae6fc7815f9640645e0c765522b278ee0cef49b482d91f6913e334628d3e77"),
    },
    ModelSpec {
        id: "whisper-medium-encoder",
        category: "stt",
        directory: "whisper-medium",
        repository: "csukuangfj/sherpa-onnx-whisper-medium",
        // Upstream name differs: `medium-encoder.int8.onnx`. It is saved locally
        // as `encoder.int8.onnx` because `local_models::pick` matches by prefix
        // (`name.starts_with("encoder")`).
        filename: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-medium/resolve/main/medium-encoder.int8.onnx",
        expected_size: 374_196_283,
        expected_sha256: Some("1c54582b4d829de0089f6cb63bbbdb3bf7555398bacaf855fbecf1a84dfd193e"),
    },
    ModelSpec {
        id: "whisper-medium-decoder",
        category: "stt",
        directory: "whisper-medium",
        repository: "csukuangfj/sherpa-onnx-whisper-medium",
        filename: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-medium/resolve/main/medium-decoder.int8.onnx",
        expected_size: 571_059_257,
        expected_sha256: Some("595d00a338a365a7bfa0ca7f296cabc639583bef770ab6130df90f49a6412747"),
    },
    ModelSpec {
        id: "whisper-medium-tokens",
        category: "stt",
        directory: "whisper-medium",
        repository: "csukuangfj/sherpa-onnx-whisper-medium",
        filename: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-medium/resolve/main/medium-tokens.txt",
        expected_size: 816_730,
        expected_sha256: Some("b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126"),
    },
];

/// The sherpa-onnx family a catalog model id must load as.
///
/// Shape matching alone cannot separate Canary from Whisper: both are
/// encoder + decoder + tokens, and `FAMILIES` checks Whisper first. A catalog
/// folder therefore pins its family by id; an unrecognised id falls back to
/// shape matching (a user's own bundle).
pub fn family_for_model(id: &str) -> Option<&'static str> {
    match id {
        "parakeet-unified-en" => Some("transducer"),
        "canary-180m-flash" => Some("canary"),
        "whisper-medium" => Some("whisper"),
        _ => None,
    }
}

pub fn catalog() -> &'static [ModelSpec] {
    CATALOG
}

/// One logical model in Settings, backed by one or more `CATALOG` files.
///
/// `CATALOG` is per *file* (a Parakeet model is four ONNX/text files, Qwen is
/// one GGUF). The Settings table shows per *model*, so this table groups them.
/// The `id` values are load-bearing: they are already stored in
/// `smoothflow.json` as `stt_version` / `llm_model_size`, so renaming one
/// silently orphans a user's downloaded model.
#[derive(Debug, Clone)]
pub struct ModelGroup {
    pub id: &'static str,
    /// `"stt"` or `"llm"` — the same strings `CATALOG` uses as `category`.
    pub kind: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub file_ids: &'static [&'static str],
    pub supported: bool,
    pub languages: &'static [&'static str],
    pub streaming: bool,
    pub language_detection: bool,
    pub benchmark: ModelBenchmark,
}

/// Measured numbers only, copied from `BENCHMARK.md`. `None` means the group was
/// never measured and the UI must say "not benchmarked" — never guess, never
/// extrapolate from another size, never copy a competitor's figure.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelBenchmark {
    /// WER on the earnings22 subset at `BENCHMARK.md` §3.
    pub wer_earnings22: Option<f32>,
    /// Audio seconds ÷ processing seconds, CPU-only (§3 "speed").
    pub realtime_factor: Option<f32>,
}

const UNBENCHMARKED: ModelBenchmark = ModelBenchmark {
    wer_earnings22: None,
    realtime_factor: None,
};

/// Parakeet TDT 0.6B v3's 25 European languages.
const PARAKEET_V3_LANGUAGES: &[&str] = &[
    "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt", "lv", "mt",
    "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
];

/// SenseVoice's headline five. The model card advertises "50+ languages" but
/// does not enumerate a verified 50-code set, and nothing in this repo pins one
/// down, so inventing the other 45 would be a lie in the UI. The full set lands
/// with engine support (phase 3); until then `supported: false` greys the row.
const SENSEVOICE_LANGUAGES: &[&str] = &["yue", "zh", "en", "ja", "ko"];

const GROUPS: &[ModelGroup] = &[
    ModelGroup {
        id: "v2",
        kind: "stt",
        name: "Parakeet TDT 0.6B v2",
        description: "Best for clear spoken English, dictation into any field. Needs a 4-core CPU and 8 GB RAM; no GPU.",
        file_ids: &[
            "parakeet-v2-encoder",
            "parakeet-v2-decoder",
            "parakeet-v2-joiner",
            "parakeet-v2-tokens",
        ],
        supported: true,
        languages: &["en"],
        // The helper runs sherpa-onnx's *offline* recognizer, so partial text
        // arrives after release, not while speaking. The old HTML badge said
        // "Streaming"; that was never true of this pipeline.
        streaming: false,
        language_detection: false,
        // BENCHMARK.md §3, earnings22 row: 15.7% WER at 9.7× realtime.
        benchmark: ModelBenchmark {
            wer_earnings22: Some(0.157),
            realtime_factor: Some(9.7),
        },
    },
    ModelGroup {
        id: "v3",
        kind: "stt",
        name: "Parakeet TDT 0.6B v3",
        description: "Automatic language detection across 25 European languages. Good if you dictate in more than one language.",
        file_ids: &[
            "parakeet-v3-encoder",
            "parakeet-v3-decoder",
            "parakeet-v3-joiner",
            "parakeet-v3-tokens",
        ],
        supported: true,
        languages: PARAKEET_V3_LANGUAGES,
        streaming: false,
        language_detection: true,
        // v3 was never benchmarked on our corpora (BENCHMARK.md cites
        // Superwhisper's published v3 figures, not our own run).
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "sv",
        kind: "stt",
        name: "SenseVoice Small",
        description: "Compact multilingual model, smaller download. Not available in this build yet.",
        // Deliberately empty: there is no SenseVoice entry in `CATALOG`, and
        // `bundle_ids` rejects "sv". The row exists so Settings can grey it out
        // as "not available yet" instead of offering a download that cannot work.
        file_ids: &[],
        supported: false,
        languages: SENSEVOICE_LANGUAGES,
        streaming: false,
        // SenseVoice ships a language-ID head; unwired until phase 3.
        language_detection: true,
        benchmark: UNBENCHMARKED,
    },
    // ---------------------------------------------------------------------
    // STT placeholder rows.
    //
    // A placeholder row here is a promise about the *menu*, not about download
    // support: it tells the user these models exist and are being considered, so
    // the picker is not silently three rows long. Nothing below has a `CATALOG`
    // entry, so `file_ids` stays empty, `supported: false` greys the row in
    // Settings as "Not available yet", `bundle_ids` rejects the id, and
    // `download_group` fails loudly without emitting a single progress event
    // (see `sensevoice_is_unsupported_with_no_files` and
    // `every_unsupported_stt_group_is_empty_and_never_downloads`).
    //
    // `languages` is deliberately empty on all of them: no verified ONNX/sherpa
    // code set exists in this repo for these models, and inventing codes would be
    // a lie in the UI (same reasoning as `SENSEVOICE_LANGUAGES`). Fill a code set
    // in only when a download spec and a real engine path land together.
    ModelGroup {
        id: "parakeet-unified-en",
        kind: "stt",
        name: "Parakeet Unified EN 0.6B",
        // Downloaded from csukuangfj2's sherpa-onnx int8 export and resolved by
        // the same four-file transducer layout as v2 (encoder/decoder/joiner/
        // tokens), so the helper needs no family change.
        description: "Newer unified English Parakeet tier. Same four-file layout as v2.",
        file_ids: &[
            "parakeet-unified-encoder",
            "parakeet-unified-decoder",
            "parakeet-unified-joiner",
            "parakeet-unified-tokens",
        ],
        supported: true,
        languages: &["en"],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "parakeet-rnnt-06b",
        kind: "stt",
        name: "Parakeet RNN-T 0.6B",
        description: "Transducer variant of Parakeet at 0.6B. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "parakeet-rnnt-11b",
        kind: "stt",
        name: "Parakeet RNN-T 1.1B",
        description: "Larger transducer Parakeet, heavier on CPU. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "canary-180m-flash",
        kind: "stt",
        name: "Canary 180M Flash",
        // Canary's file set is encoder + decoder + tokens, which is *shape*
        // identical to Whisper. `stt_local::family_for_model` pins this id to the
        // canary family so it is never loaded with whisper flags.
        description: "Smallest Canary tier (English, Spanish, German, French), aimed at weak devices.",
        file_ids: &[
            "canary-180m-encoder",
            "canary-180m-decoder",
            "canary-180m-tokens",
        ],
        supported: true,
        languages: &["en", "es", "de", "fr"],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "canary-1b-flash",
        kind: "stt",
        name: "Canary 1B Flash",
        description: "Faster Canary 1B. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "canary-1b",
        kind: "stt",
        name: "Canary 1B",
        description: "Multilingual Canary transcriber. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "canary-qwen-25b",
        kind: "stt",
        name: "Canary-Qwen 2.5B",
        description: "Large Canary/Qwen hybrid. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "whisper-medium",
        kind: "stt",
        name: "Whisper Medium",
        // The upstream repo ships `medium-encoder.int8.onnx` etc.; the catalog
        // saves them under the plain names the whisper family looks for.
        description: "Widely used multilingual Whisper tier. Slower than Parakeet on CPU.",
        file_ids: &[
            "whisper-medium-encoder",
            "whisper-medium-decoder",
            "whisper-medium-tokens",
        ],
        supported: true,
        // Whisper's 99-language set is not enumerable from anything verified in
        // this repo, so the codes stay empty rather than invented.
        languages: &[],
        streaming: false,
        language_detection: true,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "nemotron-stream-35",
        kind: "stt",
        name: "Nemotron Streaming 3.5",
        description: "Streaming model for low-latency partial text. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "qwen3-asr-06b",
        kind: "stt",
        name: "Qwen3-ASR 0.6B",
        description: "Compact Qwen speech model. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "qwen3-asr-17b",
        kind: "stt",
        name: "Qwen3-ASR 1.7B",
        description: "Larger Qwen speech model. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "fun-asr-nano",
        kind: "stt",
        name: "Fun-ASR Nano (Multilingual)",
        description: "Compact multilingual nano model. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "granite-4-1-2b",
        kind: "stt",
        name: "Granite Speech 4.1 2B",
        description: "IBM Granite speech tier. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "granite-4-0-1b",
        kind: "stt",
        name: "Granite Speech 4.0 1B",
        description: "Smaller Granite speech tier. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "voxtrak-mini-4b",
        kind: "stt",
        name: "VoxTrak Mini 4B (Realtime)",
        description: "4B realtime model; needs about 16 GB RAM. Not available in this build yet.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "cohere-transcribe",
        kind: "stt",
        name: "Cohere Transcribe",
        description: "Cloud-only service; never usable in offline mode.",
        file_ids: &[],
        supported: false,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "2b",
        kind: "llm",
        name: "Qwen 3.5 2B",
        description: "Lighter cleanup model for 8 GB machines. Fastest of the three.",
        file_ids: &["qwen-2b"],
        supported: true,
        // Text cleanup models do not care about the spoken language strings.
        languages: &[],
        streaming: false,
        language_detection: false,
        // BENCHMARK.md §5 records the 2B cleanup run as "*running*" — no numbers.
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "4b",
        kind: "llm",
        name: "Qwen 3.5 4B",
        description: "Best balance of cleanup quality and memory. Needs 8 GB RAM.",
        file_ids: &["qwen-4b"],
        supported: true,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
    ModelGroup {
        id: "9b",
        kind: "llm",
        name: "Qwen 3.5 9B",
        description: "Highest quality rewriting. Needs 16 GB RAM.",
        file_ids: &["qwen-9b"],
        supported: true,
        languages: &[],
        streaming: false,
        language_detection: false,
        benchmark: UNBENCHMARKED,
    },
];

/// One row of the Settings model table.
#[derive(Debug, Clone, Serialize)]
pub struct ModelGroupStatus {
    /// == `ModelGroup.id`; the value stored in config.
    pub id: String,
    /// `"stt"` | `"llm"`.
    pub kind: String,
    pub name: String,
    pub description: String,
    pub supported: bool,
    pub languages: Vec<String>,
    pub streaming: bool,
    pub language_detection: bool,
    /// Expected total for the whole group.
    pub size_bytes: u64,
    pub installed_bytes: u64,
    /// Every file present and size-valid.
    pub installed: bool,
    /// Some files on disk but the group is incomplete (`installed == false`).
    pub partial: bool,
    pub file_count: usize,
    pub installed_file_count: usize,
    pub benchmark: ModelBenchmark,
}

pub fn models_dir(config_dir: &Path, custom_dir: &str) -> PathBuf {
    if custom_dir.trim().is_empty() {
        config_dir.join("models")
    } else {
        PathBuf::from(custom_dir).components().collect()
    }
}

pub fn status(config_dir: &Path, custom_dir: &str) -> Result<ModelStatus, String> {
    let root = models_dir(config_dir, custom_dir);
    let mut files = Vec::with_capacity(CATALOG.len());
    let mut total_bytes = 0;
    let mut expected_total_bytes = 0;
    let mut installed_files = 0;
    for spec in CATALOG {
        validate_spec(spec)?;
        let path = target_path(&root, spec)?;
        let (installed, verified, installed_bytes) = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => (false, false, 0),
            Ok(metadata) if metadata.is_file() => {
                let installed_bytes = metadata.len();
                // ponytail: size-only here. `download` already SHA-256s every
                // file (offline.rs:228), and re-hashing 3.4 GB on every Settings
                // open is what froze the window. Hash the file here only if the
                // size check is ever suspected of lying.
                let verified = installed_bytes == spec.expected_size;
                (true, verified, installed_bytes)
            }
            Ok(_) => return Err(format!("Model path '{}' is not a regular file.", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, false, 0),
            Err(error) => return Err(format!("Could not inspect model '{}': {}", spec.id, error)),
        };
        if installed {
            installed_files += 1;
            total_bytes += installed_bytes;
        }
        expected_total_bytes += spec.expected_size;
        files.push(FileStatus {
            id: spec.id,
            category: spec.category,
            directory: spec.directory,
            filename: spec.filename,
            expected_size: spec.expected_size,
            installed,
            verified,
            installed_bytes,
        });
    }
    Ok(ModelStatus {
        missing_files: CATALOG.len() - installed_files,
        files,
        installed_files,
        total_bytes,
        expected_total_bytes,
    })
}

pub fn download(
    id: &str,
    config_dir: &Path,
    custom_dir: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    if cancel_requested() {
        // A stale flag would otherwise abort the transfer before its first
        // byte; commands clear it at the start, this is the belt for every
        // other caller.
        return Err(
            "[DL-009] Download cancelled — press DOWNLOAD on the model to resume.".into(),
        );
    }
    let spec = CATALOG
        .iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| format!("Unknown model '{}'.", id))?;
    validate_spec(spec)?;
    let expected_sha256 = spec
        .expected_sha256
        .ok_or_else(|| format!("Model '{}' cannot be downloaded because its SHA-256 manifest entry is unavailable.", spec.id))?;
    let root = models_dir(config_dir, custom_dir);
    let target = target_path(&root, spec)?;
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("Model path '{}' is not a regular file.", target.display()));
        }
        if file_is_valid(&target, spec)? {
            return Ok(target);
        }
        return Err(format!("Existing model '{}' failed verification; remove it before downloading again.", spec.id));
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("Model '{}' has no parent directory.", spec.id))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create model directory '{}': {}", parent.display(), error))?;
    let part = part_path(&target);
    let part_size = match fs::symlink_metadata(&part) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(format!("Partial model path '{}' is not a regular file.", part.display()));
        }
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("Could not inspect partial model '{}': {}", spec.id, error)),
    };
    if part_size > spec.expected_size {
        return Err(format!("Partial model '{}' is larger than its expected size.", spec.id));
    }
    let client = Client::builder()
        .build()
        .map_err(|error| format!("Could not create the model downloader: {}", error))?;
    let mut request = client.get(spec.url);
    if part_size > 0 {
        request = request.header(RANGE, format!("bytes={}-", part_size));
    }
    let mut response = request
        .send()
        .map_err(|error| format!("Could not download model '{}': {}", spec.id, error))?;
    let append = if part_size == 0 {
        false
    } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        true
    } else if response.status().is_success() {
        false
    } else {
        return Err(format!("Model download failed with HTTP status {}.", response.status()));
    };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .map_err(|error| format!("Could not open partial model '{}': {}", part.display(), error))?;
    // Chunked copy, not io::copy: the loop is where the transfer reports real
    // percent progress and honours a user cancel. `expected_size` is verified
    // right after, so using it as the denominator is safe.
    const CHUNK: usize = 256 * 1024;
    let mut buffer = vec![0_u8; CHUNK];
    let expected = spec.expected_size;
    let mut done = part_size;
    loop {
        if cancel_requested() {
            // Keep the .part file: a later press of DOWNLOAD resumes from the
            // same byte instead of starting over.
            return Err(format!(
                "[DL-009] Download cancelled — the partial file was kept in {}. Press DOWNLOAD on the model to resume.",
                part.display()
            ));
        }
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Could not download model '{}': {}", spec.id, error))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| format!("Could not write model '{}': {}", spec.id, error))?;
        done += read as u64;
        progress(done.min(expected), expected);
    }
    file.flush()
        .map_err(|error| format!("Could not flush model '{}': {}", spec.id, error))?;
    let actual_size = fs::metadata(&part)
        .map_err(|error| format!("Could not inspect downloaded model '{}': {}", spec.id, error))?
        .len();
    if actual_size != spec.expected_size {
        return Err(format!("Model '{}' has size {} bytes; expected {}.", spec.id, actual_size, spec.expected_size));
    }
    let actual_sha256 = sha256_file(&part)
        .map_err(|error| format!("Could not verify model '{}': {}", spec.id, error))?;
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!("Model '{}' failed SHA-256 verification.", spec.id));
    }
    fs::rename(&part, &target)
        .map_err(|error| format!("Could not finalize model '{}': {}", spec.id, error))?;
    Ok(target)
}

/// Catalog ids for one download run, in the order they are fetched.
///
/// An empty `stt_version` or `llm_size` means "skip that bundle": a user who
/// turned AI cleanup off must not be made to download 1.3–5.7 GB of Qwen.
fn bundle_ids(stt_version: &str, llm_size: &str) -> Result<Vec<&'static str>, String> {
    let stt_ids: &[&str] = match stt_version {
        "v2" => &["parakeet-v2-encoder", "parakeet-v2-decoder", "parakeet-v2-joiner", "parakeet-v2-tokens"],
        "v3" => &["parakeet-v3-encoder", "parakeet-v3-decoder", "parakeet-v3-joiner", "parakeet-v3-tokens"],
        "parakeet-unified-en" => &[
            "parakeet-unified-encoder",
            "parakeet-unified-decoder",
            "parakeet-unified-joiner",
            "parakeet-unified-tokens",
        ],
        "canary-180m-flash" => &["canary-180m-encoder", "canary-180m-decoder", "canary-180m-tokens"],
        "whisper-medium" => &["whisper-medium-encoder", "whisper-medium-decoder", "whisper-medium-tokens"],
        "" => &[],
        _ => return Err(format!("Unknown STT model variant '{}'.", stt_version)),
    };
    let llm_id = match llm_size {
        "2b" => Some("qwen-2b"),
        "4b" => Some("qwen-4b"),
        "9b" => Some("qwen-9b"),
        "" => None,
        _ => return Err(format!("Unknown LLM model size '{}'.", llm_size)),
    };
    if stt_ids.is_empty() && llm_id.is_none() {
        return Err(
            "Nothing was selected to download. Pick a speech model, an AI cleanup model, or both."
                .to_string(),
        );
    }
    Ok(stt_ids.iter().copied().chain(llm_id).collect())
}

pub fn download_bundle(
    stt_version: &str,
    llm_size: &str,
    config_dir: &Path,
    custom_dir: &str,
    mut progress: impl FnMut(&str, &str, &str),
) -> Result<(), String> {
    for id in bundle_ids(stt_version, llm_size)? {
        progress(id, "starting", "Download starting.");
        // The legacy bundle path predates the percent field; no percent events.
        match download(id, config_dir, custom_dir, |_, _| {}) {
            Ok(_) => progress(id, "complete", "Download complete."),
            Err(error) => {
                progress(id, "failed", &error);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// The Settings model table: one row per logical model, resolved from the
/// existing per-file check.
///
/// Named `group_catalog`, not `catalog`, because `catalog()` above is the
/// per-file accessor that `llm_local` already consumes — Rust has no overloads
/// and that call site is outside this change.
pub fn group_catalog(
    config_dir: &Path,
    custom_dir: &str,
) -> Result<Vec<ModelGroupStatus>, String> {
    // ponytail: reuse `status()` instead of walking the filesystem again. It is
    // already size-only (see the hash note in `status`), so opening Settings
    // stays microseconds even with 3.4 GB of models on disk.
    let files = status(config_dir, custom_dir)?;
    let mut groups = Vec::with_capacity(GROUPS.len());
    for group in GROUPS {
        validate_group(group)?;
        let mut size_bytes = 0_u64;
        let mut installed_bytes = 0_u64;
        let mut file_count = 0_usize;
        let mut installed_file_count = 0_usize;
        let mut every_file_valid = true;
        for file_id in group.file_ids {
            let file = files
                .files
                .iter()
                .find(|file| file.id == *file_id)
                .ok_or_else(|| {
                    format!("Model group '{}' references unknown file '{}'.", group.id, file_id)
                })?;
            file_count += 1;
            size_bytes += file.expected_size;
            if file.installed {
                installed_file_count += 1;
                installed_bytes += file.installed_bytes;
            }
            // `verified` is the same size-only rule `status()` reports.
            if !file.verified {
                every_file_valid = false;
            }
        }
        let installed = group.supported && file_count > 0 && every_file_valid;
        // `partial` also covers "all files are on disk but one failed the size
        // check": that is a broken group, not a missing one, and the UI should
        // offer a re-download rather than claim it is ready.
        let partial = !installed && installed_file_count > 0;
        groups.push(ModelGroupStatus {
            id: group.id.to_string(),
            kind: group.kind.to_string(),
            name: group.name.to_string(),
            description: group.description.to_string(),
            supported: group.supported,
            languages: group.languages.iter().map(|code| code.to_string()).collect(),
            streaming: group.streaming,
            language_detection: group.language_detection,
            size_bytes,
            installed_bytes,
            installed,
            partial,
            file_count,
            installed_file_count,
            benchmark: group.benchmark,
        });
    }
    Ok(groups)
}

/// Download every file of one logical model, in catalog order.
///
/// The only fetch path for the Settings table — there is no automatic or
/// background download (binding offline-contract rule). `progress` is called
/// with `(group id, "starting"|"progress"|"complete"|"failed", message, percent)`;
/// `progress` events carry the percent for the whole group, so the UI fills one
/// smooth bar across the group's files instead of jumping file to file.
pub fn download_group(
    id: &str,
    config_dir: &Path,
    custom_dir: &str,
    mut progress: impl FnMut(&str, &str, &str, Option<f32>),
) -> Result<(), String> {
    let group = GROUPS
        .iter()
        .find(|group| group.id == id)
        .ok_or_else(|| format!("Unknown model '{}'.", id))?;
    if !group.supported {
        return Err(format!(
            "{} is not available in this build yet. Keep Parakeet for speech, or switch to cloud mode in Settings.",
            group.name
        ));
    }
    // Also rejects a supported group with no files: that would be a silent
    // no-op, which reads to the user as "downloaded" when nothing was fetched.
    validate_group(group)?;
    let total: u64 = group
        .file_ids
        .iter()
        .filter_map(|file_id| {
            CATALOG
                .iter()
                .find(|spec| spec.id == *file_id)
                .map(|spec| spec.expected_size)
        })
        .sum();
    let mut done: u64 = 0;
    for file_id in group.file_ids {
        let message = format!("Downloading {} ({}).", group.name, file_id);
        progress(group.id, "starting", &message, None);
        let file_done = done;
        let result = download(file_id, config_dir, custom_dir, |file_bytes, _expected| {
            // Sizes come from the validated catalog, so the denominators match;
            // min keeps the bar from exceeding 100 on a server that could not
            // honour the resume Range and sent a longer body.
            let percent = if total > 0 {
                ((file_done + file_bytes) as f32 / total as f32 * 100.0).min(100.0)
            } else {
                0.0
            };
            progress(
                group.id,
                "progress",
                &format!("{} — {:.0}%.", file_id, percent),
                Some(percent),
            );
            // Also publish it for the poller: events can be missed, the atomic
            // cannot, and this is the only thing the dialog bar needs.
            set_percent(Some(percent));
        });
        // The inner progress closure is a temporary dropped with the `let`
        // above, so its borrow of `progress` has ended here.
        match result {
            Ok(_) => {
                let spec = CATALOG
                    .iter()
                    .find(|spec| spec.id == *file_id)
                    .map(|spec| spec.expected_size)
                    .unwrap_or(0);
                done += spec;
                let percent = done_percent(done, total);
                set_percent(Some(percent));
                progress(
                    group.id,
                    "complete",
                    "Download complete.",
                    Some(percent),
                );
            }
            Err(error) => {
                progress(group.id, "failed", &error, None);
                return Err(error);
            }
        }
    }
    // No separate "group is ready" event: the last file's "complete" is it, and
    // the returned catalog carries the authoritative installed/count fields.
    Ok(())
}

/// Percent helper for the completed-file counter; 0 on a zero-byte group.
fn done_percent(done: u64, total: u64) -> f32 {
    if total > 0 {
        (done as f32 / total as f32 * 100.0).min(100.0)
    } else {
        0.0
    }
}

pub fn delete(config_dir: &Path, custom_dir: &str) -> Result<(), String> {
    let root = models_dir(config_dir, custom_dir);
    let mut directories = BTreeSet::new();
    for spec in CATALOG {
        validate_spec(spec)?;
        let target = target_path(&root, spec)?;
        let part = part_path(&target);
        for path in [target.clone(), part] {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(format!("Refusing to delete non-file model path '{}'.", path.display()));
                }
                Ok(_) => fs::remove_file(&path)
                    .map_err(|error| format!("Could not delete model file '{}': {}", path.display(), error))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Could not inspect model file '{}': {}", path.display(), error)),
            }
        }
        if let Some(parent) = target.parent() {
            directories.insert(parent.to_path_buf());
        }
    }
    let mut directories: Vec<PathBuf> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound || error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => return Err(format!("Could not remove empty model directory '{}': {}", directory.display(), error)),
        }
    }
    match fs::remove_dir(&root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound || error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(format!("Could not remove empty model directory '{}': {}", root.display(), error)),
    }
}

/// Remove exactly one logical model's files, leaving every other downloaded
/// model untouched. The model's own folder is removed only when it is empty, so
/// a file the user dropped in there is never deleted. Returns the refreshed
/// table (same shape as `get_model_catalog`) so Settings needs no second call.
pub fn delete_group(
    id: &str,
    config_dir: &Path,
    custom_dir: &str,
) -> Result<Vec<ModelGroupStatus>, String> {
    let group = GROUPS
        .iter()
        .find(|group| group.id == id)
        .ok_or_else(|| format!("[DL-010] Unknown model '{}'.", id))?;
    validate_group(group)?;
    let root = models_dir(config_dir, custom_dir);
    let mut directories = BTreeSet::new();
    for file_id in group.file_ids {
        let spec = CATALOG
            .iter()
            .find(|spec| spec.id == *file_id)
            .ok_or_else(|| {
                format!("Model '{}' references unknown file '{}'.", id, file_id)
            })?;
        validate_spec(spec)?;
        let target = target_path(&root, spec)?;
        for path in [target.clone(), part_path(&target)] {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(format!("Refusing to delete non-file model path '{}'.", path.display()));
                }
                Ok(_) => fs::remove_file(&path)
                    .map_err(|error| format!("Could not delete model file '{}': {}", path.display(), error))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Could not inspect model file '{}': {}", path.display(), error)),
            }
        }
        if let Some(parent) = target.parent() {
            directories.insert(parent.to_path_buf());
        }
    }
    for directory in directories {
        // `remove_dir` refuses a non-empty folder, which is exactly the wanted
        // behaviour: never delete something this command did not create.
        let _ = fs::remove_dir(&directory);
    }
    group_catalog(config_dir, custom_dir)
}

fn target_path(root: &Path, spec: &ModelSpec) -> Result<PathBuf, String> {
    validate_filename(spec.filename)?;
    Ok(root.join(spec.category).join(spec.directory).join(spec.filename))
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

fn validate_spec(spec: &ModelSpec) -> Result<(), String> {
    validate_component("model id", spec.id)?;
    validate_component("model category", spec.category)?;
    validate_component("model directory", spec.directory)?;
    validate_filename(spec.filename)?;
    let repository_parts: Vec<&str> = spec.repository.split('/').collect();
    if repository_parts.len() != 2 || repository_parts.iter().any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\')) {
        return Err(format!("Model '{}' has an unsafe repository path.", spec.id));
    }
    if has_unsafe_url_path(spec.url) {
        return Err(format!("Model '{}' has unsafe URL path components.", spec.id));
    }
    let url = reqwest::Url::parse(spec.url)
        .map_err(|_| format!("Model '{}' has an invalid URL.", spec.id))?;
    if url.scheme() != "https" || url.host_str() != Some("huggingface.co") || url.port().is_some() || url.username() != "" || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(format!("Model '{}' has an unsafe URL.", spec.id));
    }
    if spec.expected_size == 0 {
        return Err(format!("Model '{}' has an invalid expected size.", spec.id));
    }
    // A spec with no hash is not downloadable: `download()` refuses it before
    // its first byte, which reaches the user as "I clicked DOWNLOAD and nothing
    // happened". Fail here instead, where CI and every catalog load see it.
    let expected_sha256 = spec.expected_sha256.ok_or_else(|| {
        format!(
            "Model '{}' has no SHA-256 manifest entry, so it cannot be downloaded. This is a bug in SmoothFlow, not something you did.",
            spec.id
        )
    })?;
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Model '{}' has an invalid SHA-256 manifest entry.", spec.id));
    }
    Ok(())
}

/// Rejects `.`/`..` path segments in the raw URL string. `Url::parse` resolves
/// dot segments (including `%2e%2e`) while parsing, so a check on the parsed
/// `path_segments()` can never see them. This is the only place they exist.
fn has_unsafe_url_path(raw: &str) -> bool {
    let Some((_, after_scheme)) = raw.split_once("://") else {
        return true;
    };
    let path = &after_scheme[after_scheme.find('/').unwrap_or(after_scheme.len())..];
    path.split('/').any(|segment| {
        let segment = segment.to_ascii_lowercase().replace("%2e", ".");
        segment == "." || segment == ".."
    })
}

fn validate_component(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') || value.contains('\\') || value.contains('\0') {
        return Err(format!("Unsafe {} in model catalog.", name));
    }
    Ok(())
}

fn validate_filename(filename: &str) -> Result<(), String> {
    validate_component("filename", filename)
}

fn validate_group(group: &ModelGroup) -> Result<(), String> {
    validate_component("model group id", group.id)?;
    if group.kind != "stt" && group.kind != "llm" {
        return Err(format!("Model group '{}' has an unknown kind.", group.id));
    }
    if group.name.trim().is_empty() || group.description.trim().is_empty() {
        return Err(format!("Model group '{}' is missing a name or description.", group.id));
    }
    if group.supported && group.file_ids.is_empty() {
        return Err(format!(
            "Model group '{}' is marked supported but lists no files. This is a bug in SmoothFlow, not something you did.",
            group.id
        ));
    }
    for file_id in group.file_ids {
        if !CATALOG.iter().any(|spec| spec.id == *file_id) {
            return Err(format!("Model group '{}' references unknown file '{}'.", group.id, file_id));
        }
    }
    Ok(())
}

fn file_is_valid(path: &Path, spec: &ModelSpec) -> Result<bool, String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect model '{}': {}", spec.id, error))?
        .len();
    if size != spec.expected_size {
        return Ok(false);
    }
    let Some(expected_sha256) = spec.expected_sha256 else {
        return Ok(false);
    };
    let actual_sha256 = sha256_file(path)
        .map_err(|error| format!("Could not verify model '{}': {}", spec.id, error))?;
    Ok(actual_sha256.eq_ignore_ascii_case(expected_sha256))
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_directory_resolution_uses_config_and_normalized_custom_paths() {
        let config = Path::new("config");
        assert_eq!(models_dir(config, ""), PathBuf::from("config").join("models"));
        assert_eq!(models_dir(config, "custom/./models"), PathBuf::from("custom").join("models"));
    }

    #[test]
    fn catalog_contains_verified_qwen_and_parakeet_token_entries() {
        let qwen_2b = CATALOG.iter().find(|spec| spec.id == "qwen-2b").unwrap();
        let qwen_4b = CATALOG.iter().find(|spec| spec.id == "qwen-4b").unwrap();
        let qwen_9b = CATALOG.iter().find(|spec| spec.id == "qwen-9b").unwrap();
        assert_eq!(qwen_2b.expected_size, 1_280_835_840);
        assert_eq!(qwen_4b.expected_size, 2_740_937_888);
        assert_eq!(qwen_9b.expected_size, 5_680_522_464);
        assert!(qwen_2b.expected_sha256.is_some());
        assert!(CATALOG.iter().any(|spec| spec.id == "parakeet-v2-tokens" && spec.expected_sha256.is_some()));
        assert!(CATALOG.iter().any(|spec| spec.id == "parakeet-v3-tokens" && spec.expected_sha256.is_some()));
    }

    #[test]
    fn catalog_targets_use_sherpa_parakeet_files_and_keep_stt_variants_separate() {
        let root = Path::new("models");
        let v2_encoder = CATALOG.iter().find(|spec| spec.id == "parakeet-v2-encoder").unwrap();
        let v2_decoder = CATALOG.iter().find(|spec| spec.id == "parakeet-v2-decoder").unwrap();
        let v2_joiner = CATALOG.iter().find(|spec| spec.id == "parakeet-v2-joiner").unwrap();
        let v2_tokens = CATALOG.iter().find(|spec| spec.id == "parakeet-v2-tokens").unwrap();
        let v3_encoder = CATALOG.iter().find(|spec| spec.id == "parakeet-v3-encoder").unwrap();
        let v3_decoder = CATALOG.iter().find(|spec| spec.id == "parakeet-v3-decoder").unwrap();
        let v3_joiner = CATALOG.iter().find(|spec| spec.id == "parakeet-v3-joiner").unwrap();
        let v3_tokens = CATALOG.iter().find(|spec| spec.id == "parakeet-v3-tokens").unwrap();
        let qwen = CATALOG.iter().find(|spec| spec.id == "qwen-2b").unwrap();
        assert!(CATALOG.iter().filter(|spec| spec.directory == "v2").all(|spec| spec.repository == "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"));
        assert!(CATALOG.iter().filter(|spec| spec.directory == "v3").all(|spec| spec.repository == "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"));
        assert_eq!(target_path(root, v2_encoder).unwrap(), PathBuf::from("models").join("stt").join("v2").join("encoder.int8.onnx"));
        assert_eq!(target_path(root, v2_decoder).unwrap(), PathBuf::from("models").join("stt").join("v2").join("decoder.int8.onnx"));
        assert_eq!(target_path(root, v2_joiner).unwrap(), PathBuf::from("models").join("stt").join("v2").join("joiner.int8.onnx"));
        assert_eq!(target_path(root, v2_tokens).unwrap(), PathBuf::from("models").join("stt").join("v2").join("tokens.txt"));
        assert_eq!(target_path(root, v3_encoder).unwrap(), PathBuf::from("models").join("stt").join("v3").join("encoder.int8.onnx"));
        assert_eq!(target_path(root, v3_decoder).unwrap(), PathBuf::from("models").join("stt").join("v3").join("decoder.int8.onnx"));
        assert_eq!(target_path(root, v3_joiner).unwrap(), PathBuf::from("models").join("stt").join("v3").join("joiner.int8.onnx"));
        assert_eq!(target_path(root, v3_tokens).unwrap(), PathBuf::from("models").join("stt").join("v3").join("tokens.txt"));
        assert_eq!(target_path(root, qwen).unwrap(), PathBuf::from("models").join("llm").join("2b").join("Qwen3.5-2B-Q4_K_M.gguf"));
    }

    #[test]
    fn unsafe_catalog_paths_are_rejected() {
        let mut spec = CATALOG[0].clone();
        spec.filename = "../model.onnx";
        assert!(validate_spec(&spec).is_err());
        spec.filename = "nested/model.onnx";
        assert!(validate_spec(&spec).is_err());
        spec.filename = "model.onnx";
        spec.directory = "../v3";
        assert!(validate_spec(&spec).is_err());
        spec.directory = "v2";
        spec.url = "https://huggingface.co/models/../secret";
        assert!(validate_spec(&spec).is_err(), "raw `..` segment must be rejected");
        spec.url = "https://huggingface.co/models/./secret";
        assert!(validate_spec(&spec).is_err(), "raw `.` segment must be rejected");
        // `Url::parse` also collapses `%2e%2e` to `/secret`, so the raw check decodes it.
        spec.url = "https://huggingface.co/models/%2e%2e/secret";
        assert!(validate_spec(&spec).is_err(), "percent-encoded traversal must be rejected");
        spec.url = "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf";
        assert!(validate_spec(&spec).is_ok(), "a real catalog URL must stay valid");
    }

    #[test]
    fn every_catalog_entry_passes_validation() {
        for spec in CATALOG {
            assert!(validate_spec(spec).is_ok(), "catalog entry '{}' is invalid", spec.id);
        }
    }

    #[test]
    fn an_empty_bundle_selection_downloads_only_what_was_asked_for() {
        // Both chosen: unchanged behaviour, speech first then cleanup.
        let both = bundle_ids("v2", "4b").unwrap();
        assert_eq!(both.len(), 5);
        assert_eq!(both[0], "parakeet-v2-encoder");
        assert_eq!(both[4], "qwen-4b");

        // Speech only: no Qwen bytes at all.
        let stt_only = bundle_ids("v3", "").unwrap();
        assert_eq!(stt_only.len(), 4);
        assert!(stt_only.iter().all(|id| id.starts_with("parakeet-v3-")));

        // Cleanup only: no speech bytes at all.
        assert_eq!(bundle_ids("", "9b").unwrap(), vec!["qwen-9b"]);

        // Nothing selected is an error, not a silent no-op.
        for (stt, llm) in [("", ""), (" ", ""), ("", " ")] {
            assert!(bundle_ids(stt, llm).is_err(), "{stt:?}/{llm:?} should fail");
        }

        // Unknown values still fail, so a typo is never read as "skip".
        assert!(bundle_ids("v4", "4b").is_err());
        assert!(bundle_ids("v2", "12b").is_err());
    }

    /// A path that does not exist, so `status()` reports "not installed" without
    /// touching the real models folder or the network.
    fn missing_models_root() -> (PathBuf, String) {
        (
            std::env::temp_dir().join("smoothflow-group-catalog-test"),
            String::new(),
        )
    }

    #[test]
    fn every_group_id_is_a_value_the_config_can_hold() {
        // The ids are written to smoothflow.json, so they must round-trip through
        // the same parser `download_bundle` uses. A new id here means a new config
        // value, which must be taught to `bundle_ids` in the same change.
        let stt_ids: Vec<&str> = GROUPS.iter().filter(|g| g.kind == "stt").map(|g| g.id).collect();
        let llm_ids: Vec<&str> = GROUPS.iter().filter(|g| g.kind == "llm").map(|g| g.id).collect();
        assert_eq!(
            stt_ids,
            vec![
                "v2",
                "v3",
                "sv",
                "parakeet-unified-en",
                "parakeet-rnnt-06b",
                "parakeet-rnnt-11b",
                "canary-180m-flash",
                "canary-1b-flash",
                "canary-1b",
                "canary-qwen-25b",
                "whisper-medium",
                "nemotron-stream-35",
                "qwen3-asr-06b",
                "qwen3-asr-17b",
                "fun-asr-nano",
                "granite-4-1-2b",
                "granite-4-0-1b",
                "voxtrak-mini-4b",
                "cohere-transcribe",
            ]
        );
        assert_eq!(llm_ids, vec!["2b", "4b", "9b"]);
        for group in GROUPS {
            if group.kind == "stt" {
                assert_eq!(bundle_ids(group.id, "4b").is_ok(), group.supported, "stt '{}' disagrees with bundle_ids", group.id);
            } else {
                assert_eq!(bundle_ids("v2", group.id).is_ok(), group.supported, "llm '{}' disagrees with bundle_ids", group.id);
            }
        }
        // v3 lists the 25 codes Parakeet v3 advertises; v2 is English only.
        let v2 = GROUPS.iter().find(|g| g.id == "v2").unwrap();
        let v3 = GROUPS.iter().find(|g| g.id == "v3").unwrap();
        assert_eq!(v2.languages, &["en"]);
        assert_eq!(v3.languages.len(), 25);
        assert!(v3.languages.contains(&"en"));
        // Cleanup models are language-agnostic and must not claim languages.
        assert!(GROUPS.iter().filter(|g| g.kind == "llm").all(|g| g.languages.is_empty()));
    }

    #[test]
    fn sensevoice_is_unsupported_with_no_files() {
        let sv = GROUPS.iter().find(|g| g.id == "sv").unwrap();
        assert!(!sv.supported);
        assert!(sv.file_ids.is_empty());
        assert!(validate_group(sv).is_ok(), "an unsupported empty group is catalogued, not broken");
        // Nothing in the file catalog backs it, so downloading must fail loudly.
        let (root, custom) = missing_models_root();
        let mut events = 0;
        let error = download_group("sv", &root, &custom, |_, _, _, _| events += 1).unwrap_err();
        assert!(error.contains("not available"), "unexpected error: {error}");
        assert_eq!(events, 0, "an unsupported group must not emit progress as if it downloaded");
    }

    #[test]
    fn every_supported_group_is_actually_downloadable() {
        // A row only appears in Settings when `supported` is true, and clicking
        // DOWNLOAD can only work when every file it pulls has a verified size and
        // SHA-256. A spec missing its hash used to fail at the first byte with no
        // visible error, so the whole feature looked dead.
        for group in GROUPS.iter().filter(|group| group.supported) {
            assert!(!group.file_ids.is_empty(), "{} has no files", group.id);
            for file_id in group.file_ids {
                let spec = CATALOG
                    .iter()
                    .find(|spec| spec.id == *file_id)
                    .unwrap_or_else(|| panic!("{} references unknown file {}", group.id, file_id));
                assert!(
                    validate_spec(spec).is_ok(),
                    "{} / {} is not downloadable: {:?}",
                    group.id,
                    file_id,
                    validate_spec(spec),
                );
                assert!(spec.expected_sha256.is_some(), "{} has no SHA-256", file_id);
                assert!(spec.expected_size > 0, "{} has no size", file_id);
            }
        }
    }

    #[test]
    fn delete_group_removes_only_that_models_files() {
        let (root, custom) = missing_models_root();
        let _ = std::fs::remove_dir_all(&root);
        // Stand-in files: sizes do not matter here, only which paths survive.
        for spec in CATALOG.iter().filter(|spec| {
            spec.directory == "v2" || spec.directory == "whisper-medium"
        }) {
            let path = target_path(&root, spec).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }
        let whisper_encoder = CATALOG
            .iter()
            .find(|spec| spec.id == "whisper-medium-encoder")
            .unwrap();
        let v2_encoder = CATALOG
            .iter()
            .find(|spec| spec.id == "parakeet-v2-encoder")
            .unwrap();

        let catalog = delete_group("v2", &root, &custom).unwrap();

        // The removed model is gone, folder included, and the other model keeps
        // every byte it had.
        assert!(!target_path(&root, v2_encoder).unwrap().exists());
        assert!(!root.join("stt").join("v2").exists(), "empty v2 folder must be removed");
        assert!(target_path(&root, whisper_encoder).unwrap().exists());
        assert_eq!(catalog.len(), GROUPS.len());
        assert!(delete_group("not-a-model", &root, &custom).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The placeholder rows exist so the picker shows the whole menu. That is only
    /// safe while every one of them stays a *menu* row: no files, no claimed
    /// languages, no progress events. A row that silently starts "downloading"
    /// nothing would read as success, which is the failure this guards.
    #[test]
    fn every_unsupported_stt_group_is_empty_and_never_downloads() {
        let (root, custom) = missing_models_root();
        for group in GROUPS.iter().filter(|g| g.kind == "stt" && !g.supported) {
            assert!(group.file_ids.is_empty(), "'{}' must list no files", group.id);
            assert!(group.languages.is_empty(), "'{}' must not claim unverified languages", group.id);
            assert!(validate_group(group).is_ok(), "'{}' is a catalogued placeholder, not broken", group.id);
            let mut events = 0;
            let error = download_group(group.id, &root, &custom, |_, _, _, _| events += 1)
                .unwrap_err();
            assert!(error.contains("not available"), "'{}' error: {error}", group.id);
            assert_eq!(events, 0, "'{}' must not emit progress", group.id);
        }
        // The one STT group that is supported stays downloadable.
        assert!(GROUPS.iter().any(|g| g.kind == "stt" && g.supported));
    }

    #[test]
    fn every_group_size_is_the_sum_of_its_file_specs() {
        let (root, custom) = missing_models_root();
        let catalog = group_catalog(&root, &custom).unwrap();
        assert_eq!(catalog.len(), GROUPS.len());
        for status in &catalog {
            let group = GROUPS.iter().find(|g| g.id == status.id).unwrap();
            let expected: u64 = group
                .file_ids
                .iter()
                .map(|id| CATALOG.iter().find(|spec| spec.id == *id).unwrap().expected_size)
                .sum();
            assert_eq!(status.size_bytes, expected, "group '{}' size mismatch", group.id);
            assert_eq!(status.file_count, group.file_ids.len());
            // No files on disk, so nothing is installed and nothing is partial.
            assert!(!status.installed, "group '{}' cannot be installed in an empty dir", group.id);
            assert!(!status.partial);
            assert_eq!(status.installed_bytes, 0);
            assert_eq!(status.installed_file_count, 0);
        }
        // The Parakeet groups are four files each; the Qwen groups are one GGUF.
        let by_id = |id: &str| catalog.iter().find(|s| s.id == id).unwrap().clone();
        assert_eq!(by_id("v2").file_count, 4);
        assert_eq!(by_id("v3").file_count, 4);
        assert_eq!(by_id("sv").file_count, 0);
        assert_eq!(by_id("2b").file_count, 1);
        assert_eq!(by_id("4b").file_count, 1);
        assert_eq!(by_id("9b").file_count, 1);
        assert_eq!(by_id("sv").size_bytes, 0);
    }

    #[test]
    fn only_the_measured_v2_group_carries_a_benchmark() {
        for group in GROUPS {
            if group.id == "v2" {
                // BENCHMARK.md §3: 15.7% WER on earnings22 at 9.7x realtime.
                assert_eq!(group.benchmark.wer_earnings22, Some(0.157));
                assert_eq!(group.benchmark.realtime_factor, Some(9.7));
            } else {
                assert_eq!(group.benchmark.wer_earnings22, None, "group '{}' must not invent a WER", group.id);
                assert_eq!(group.benchmark.realtime_factor, None, "group '{}' must not invent a speed", group.id);
            }
        }
    }

    #[test]
    fn unknown_and_unsupported_group_downloads_error() {
        let (root, custom) = missing_models_root();
        let unknown = download_group("v4", &root, &custom, |_, _, _, _| {}).unwrap_err();
        assert!(unknown.contains("Unknown model"), "unexpected error: {unknown}");
        assert!(download_group("", &root, &custom, |_, _, _, _| {}).is_err());
        // A supported group without files is a bug, and must never read as success.
        let mut broken = GROUPS[0].clone();
        broken.id = "broken";
        broken.file_ids = &[];
        assert!(validate_group(&broken).is_err());
        assert!(validate_group(&GROUPS[0]).is_ok());
    }
}
