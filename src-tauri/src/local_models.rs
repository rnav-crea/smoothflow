//! Discovery of LLM model files the user *already* has on this machine, so a
//! local-cleanup user never has to download our catalog model first.
//!
//! Strictly read-only: no network, no writes, no child processes. Every I/O
//! failure is absorbed by skipping that one entry — one unreadable directory
//! must never fail the whole scan.
//!
//! Two families are covered:
//!   1. Ollama stores no `*.gguf`; it stores content-addressed blobs under
//!      `<root>/blobs/sha256-<hex>` plus manifests under `<root>/manifests`.
//!      The weights live in the layer whose `mediaType` is
//!      `application/vnd.ollama.image.model`, so that is the path llama.cpp
//!      can load directly.
//!   2. LM Studio / Jan / GPT4All keep plain `*.gguf` files on disk.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

/// Hard cap: a pathological model folder must not stall the overlay.
const MAX_RESULTS: usize = 100;
/// Deep enough for `publisher/repo/quant/weights.gguf`, shallow enough to be fast.
const MAX_WALK_DEPTH: usize = 6;
/// Ollama manifests nest one level deeper than a plain gguf layout.
const MAX_MANIFEST_DEPTH: usize = 8;
/// Anything smaller is a tokenizer/config stub, not weights.
const MIN_GGUF_BYTES: u64 = 50 * 1024 * 1024;

const OLLAMA_MODEL_MEDIA_TYPE: &str = "application/vnd.ollama.image.model";

#[derive(serde::Serialize, Clone, Debug)]
pub struct LocalModel {
    /// Human-facing name, e.g. `qwen2.5-coder:7b — Ollama`.
    pub label: String,
    /// Absolute path to a file llama.cpp can load.
    pub path: String,
    /// `ollama` | `lmstudio` | `jan` | `gpt4all`.
    pub source: String,
    pub size_bytes: u64,
    /// e.g. `7.6B`; empty when unknown.
    pub parameters: String,
    /// e.g. `Q4_K_M`; empty when unknown.
    pub quant: String,
}

/// List LLM models already present on this computer.
///
/// Async on purpose: a recursive walk on a slow disk would otherwise block the
/// Tauri main thread (a sync `#[tauri::command]` runs on the main thread).
#[tauri::command]
pub async fn list_local_models() -> Result<Vec<LocalModel>, String> {
    tauri::async_runtime::spawn_blocking(discover)
        .await
        .map_err(|e| format!("[LM-001] The local model scan task failed: {e}"))
}

/// Never fails by design: returns an empty list when nothing is found.
fn discover() -> Vec<LocalModel> {
    merge_models(ollama_models(), gguf_models())
}

// ------------------------------------------------ speech model folders -----

/// One Parakeet bundle on disk, ready to hand to the native helper.
#[derive(serde::Serialize, Debug)]
pub struct SttModelFiles {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
}

/// Filenames inside a Parakeet bundle, as `(prefix, extension)`.
/// Prefix, not exact name: the same part ships as `encoder.onnx` or
/// `encoder.int8.onnx` depending on the quantization, so the prefix is what
/// identifies a file and `pick` breaks the tie.
pub(crate) const STT_PARTS: [(&str, &str); 4] = [
    ("encoder", ".onnx"),
    ("decoder", ".onnx"),
    ("joiner", ".onnx"),
    ("tokens", ".txt"),
];

/// Report which Parakeet files a folder actually holds, so a broken pick is
/// visible in Settings instead of failing a dictation.
///
/// Async for the same reason as `list_local_models`: a folder on a network
/// share stalls, and a sync command runs on the Tauri main thread.
#[tauri::command]
pub async fn inspect_stt_dir(path: String) -> Result<SttModelFiles, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_dir(Path::new(path.trim())))
        .await
        .map_err(|e| format!("[STT-103] The model folder scan task failed: {e}"))?
}

/// Resolve the four model files in one folder. Read-only: no writes, no
/// network, no subdirectories.
fn inspect_dir(dir: &Path) -> Result<SttModelFiles, String> {
    if dir.as_os_str().is_empty() {
        return Err("[STT-102] No folder was given. Pick the folder that holds the Parakeet encoder, decoder and joiner (.onnx) files plus tokens.txt.".to_string());
    }
    let dir = absolute(dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "[STT-102] This folder does not exist: {}. Pick the folder that holds the Parakeet encoder, decoder and joiner (.onnx) files plus tokens.txt.",
                dir.display()
            ))
        }
        Err(error) => {
            return Err(format!(
                "[STT-102] This folder could not be read: {}. ({error}) Check its permissions, or pick it again.",
                dir.display()
            ))
        }
    };

    let files = collect_files(entries);

    let mut resolved: Vec<String> = Vec::with_capacity(STT_PARTS.len());
    let mut missing: Vec<&str> = Vec::new();
    for (prefix, extension) in STT_PARTS {
        match pick(&files, prefix, extension) {
            Some(path) => resolved.push(path),
            None => missing.push(prefix),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "[STT-101] This folder is missing: {}. A Parakeet model folder needs encoder, decoder and joiner (.onnx) plus tokens.txt.",
            missing.join(", ")
        ));
    }

    let [encoder, decoder, joiner, tokens] = resolved.as_slice() else {
        // Unreachable: `missing` was empty, so all four were pushed.
        return Err("[STT-101] This folder is missing a Parakeet model file. A Parakeet model folder needs encoder, decoder and joiner (.onnx) plus tokens.txt.".to_string());
    };
    Ok(SttModelFiles {
        encoder: encoder.clone(),
        decoder: decoder.clone(),
        joiner: joiner.clone(),
        tokens: tokens.clone(),
    })
}

/// Lowercased filename plus real path for every regular file in a listing.
///
/// Shared with `stt_local`, so the folder the Settings UI validated is resolved
/// by exactly the same rules the engine uses at spawn — a folder that says "All
/// 4 files found" cannot then fail to load.
pub(crate) fn collect_files(entries: std::fs::ReadDir) -> Vec<(String, String)> {
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some((name.to_lowercase(), path.to_string_lossy().into_owned()))
        })
        .collect()
}

/// Best match for one part: an `int8` build beats any other, then the shortest
/// name, then alphabetical (deterministic, never random).
pub(crate) fn pick(files: &[(String, String)], prefix: &str, extension: &str) -> Option<String> {
    let mut matches: Vec<&(String, String)> = files
        .iter()
        .filter(|(name, _)| name.starts_with(prefix) && name.ends_with(extension))
        .collect();
    matches.sort_by(|a, b| {
        (!a.0.contains("int8"))
            .cmp(&!b.0.contains("int8"))
            .then_with(|| a.0.len().cmp(&b.0.len()))
            .then_with(|| a.0.cmp(&b.0))
    });
    matches.first().map(|(_, path)| path.clone())
}

/// Absolute form of a user-supplied folder, so the helper never resolves a
/// relative path against its own working directory.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

// ------------------------------------------------ models folder (catalog) ---

/// One entry the models-folder UI can offer for a picker.
#[derive(serde::Serialize, Clone, Debug)]
pub struct FolderModel {
    /// `v2`/`v3` for a catalog speech model, `2b`/`4b`/`9b` for a catalog
    /// cleanup model, otherwise the folder/file stem.
    pub id: String,
    /// Human name, e.g. `Parakeet v2 — English`, `Qwen 4B`, or the file stem.
    pub label: String,
    /// Speech: the folder. Cleanup: the `.gguf` file.
    pub path: String,
    /// Speech: sum of the four model files. Cleanup: the file size.
    pub size_bytes: u64,
    /// Speech: all four parts found. Cleanup: always true.
    pub ready: bool,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct FolderScan {
    pub models_dir: String,
    pub stt: Vec<FolderModel>,
    pub llm: Vec<FolderModel>,
}

/// `<models root>`, resolved exactly the way `stt_local::model_dir` does, so the
/// folder this lists is the folder the engines load from.
fn models_root() -> PathBuf {
    // `Config::load()` never fails (it falls back to the defaults), so a broken
    // config file yields the default models dir rather than an error.
    let custom_dir = crate::config::Config::load().models_dir;
    let config_dir = crate::config::Config::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    crate::offline::models_dir(&config_dir, &custom_dir)
}

/// What is actually inside the models folder — the only place models are looked
/// for. Read-only: no network, no writes, no recursion beyond
/// `<models>/stt/<bundle>` and `<models>/llm/[<name>/]<file>.gguf`.
///
/// Async on purpose: a models folder on a slow or network drive would otherwise
/// block the Tauri main thread (a sync `#[tauri::command]` runs on it).
#[tauri::command]
pub async fn scan_models_folder() -> Result<FolderScan, String> {
    tauri::async_runtime::spawn_blocking(scan)
        .await
        .map_err(|e| format!("[LM-002] The models folder scan task failed: {e}"))
}

fn scan() -> FolderScan {
    let root = models_root();
    FolderScan {
        stt: sort_entries(stt_entries(&root.join("stt"))),
        llm: sort_entries(llm_entries(&root.join("llm"))),
        models_dir: root.display().to_string(),
    }
}

/// Open the models folder in the OS file manager, creating it first: on a fresh
/// install it does not exist yet, and the user needs somewhere to put a model.
#[tauri::command]
pub fn open_models_folder() -> Result<String, String> {
    let root = models_root();
    std::fs::create_dir_all(&root).map_err(|e| {
        format!(
            "[LM-003] Could not create the models folder at {}. ({e})",
            root.display()
        )
    })?;
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&root)
        .spawn()
        .map_err(|e| {
            format!(
                "[LM-004] Could not open the models folder at {}. Open it manually. ({e})",
                root.display()
            )
        })?;
    Ok(root.display().to_string())
}

/// Direct subdirectories of `<models>/stt/`. An incomplete bundle is still
/// returned with `ready: false` so the UI can show it and refuse to offer it.
fn stt_entries(dir: &Path) -> Vec<FolderModel> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(stem) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        out.push(FolderModel {
            label: stt_label(&stem),
            path: path.to_string_lossy().into_owned(),
            size_bytes: bundle_size(&path),
            ready: inspect_dir(&path).is_ok(),
            id: stem,
        });
    }
    out
}

/// Sum of the four Parakeet parts present. A partial bundle reports what it has,
/// so a half-finished download is not shown as 0 bytes.
fn bundle_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let files = collect_files(entries);
    STT_PARTS
        .iter()
        .filter_map(|(prefix, extension)| pick(&files, prefix, extension))
        .filter_map(|path| file_size(Path::new(&path)))
        .sum()
}

/// `.gguf` files at `<models>/llm/<name>/<file>.gguf` and
/// `<models>/llm/<file>.gguf`. One level of nesting, no deeper.
fn llm_entries(dir: &Path) -> Vec<FolderModel> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let container = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let Ok(inner) = std::fs::read_dir(&path) else {
                continue;
            };
            for file in inner.flatten() {
                push_gguf(&file.path(), container.as_deref(), &mut out);
            }
        } else {
            push_gguf(&path, None, &mut out);
        }
    }
    out
}

/// One `.gguf` file. In the catalog layout the enclosing folder *is* the model id
/// (`<models>/llm/4b/Qwen3.5-4B-Q4_K_M.gguf`); any other layout is identified by
/// the file stem.
fn push_gguf(file: &Path, container: Option<&str>, out: &mut Vec<FolderModel>) {
    if !file.is_file() {
        return;
    }
    let Some(name) = file
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
    else {
        return;
    };
    if !name.ends_with(".gguf") {
        return;
    }
    let Some(stem) = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    else {
        return;
    };
    // A stat failure (locked file, race with a delete) drops just this entry.
    let Some(size) = file_size(file) else {
        return;
    };
    let key = match container {
        Some(folder) if is_catalog_llm_id(folder) => folder,
        _ => stem.as_str(),
    };
    out.push(FolderModel {
        id: key.to_string(),
        label: llm_label(key),
        path: file.to_string_lossy().into_owned(),
        size_bytes: size,
        ready: true,
    });
}

fn is_catalog_llm_id(id: &str) -> bool {
    matches!(id.trim().to_ascii_lowercase().as_str(), "2b" | "4b" | "9b")
}

/// Human name for a speech bundle folder; an unrecognised folder keeps its own
/// name.
fn stt_label(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "v2" => "Parakeet v2 — English".to_string(),
        "v3" => "Parakeet v3 — 25 languages".to_string(),
        "parakeet-unified-en" => "Parakeet Unified EN".to_string(),
        "canary-180m-flash" => "Canary 180M Flash".to_string(),
        "whisper-medium" => "Whisper Medium".to_string(),
        _ => id.to_string(),
    }
}

/// Human name for a cleanup model; anything unrecognised keeps its own name.
fn llm_label(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "2b" => "Qwen 2B".to_string(),
        "4b" => "Qwen 4B".to_string(),
        "9b" => "Qwen 9B".to_string(),
        _ => id.to_string(),
    }
}

/// Ready entries first, then label alphabetically; one entry per path. Pure, so
/// the ordering rules are testable without a real models folder.
fn sort_entries(entries: Vec<FolderModel>) -> Vec<FolderModel> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut kept: Vec<FolderModel> = Vec::new();
    for entry in entries {
        // Windows paths are case-insensitive.
        if seen.insert(entry.path.to_lowercase()) {
            kept.push(entry);
        }
    }
    kept.sort_by(|a, b| {
        b.ready
            .cmp(&a.ready)
            .then_with(|| {
                a.label
                    .to_lowercase()
                    .cmp(&b.label.to_lowercase())
            })
            // Path breaks a label tie, so the order is never arbitrary.
            .then_with(|| a.path.cmp(&b.path))
    });
    kept
}

/// Ollama entries win over plain `.gguf` entries for the same path, then the
/// list is sorted biggest-first and capped.
fn merge_models(primary: Vec<LocalModel>, secondary: Vec<LocalModel>) -> Vec<LocalModel> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut models: Vec<LocalModel> = Vec::new();
    for model in primary.into_iter().chain(secondary.into_iter()) {
        // Windows paths are case-insensitive.
        if seen.insert(model.path.to_lowercase()) {
            models.push(model);
        }
    }
    models.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    models.truncate(MAX_RESULTS);
    models
}

// ---------------------------------------------------------------- Ollama ----

#[derive(Deserialize)]
struct OllamaManifest {
    #[serde(default)]
    config: Option<OllamaLayer>,
    #[serde(default)]
    layers: Option<Vec<OllamaLayer>>,
}

#[derive(Deserialize)]
struct OllamaLayer {
    #[serde(default, rename = "mediaType")]
    media_type: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Deserialize)]
struct OllamaConfig {
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    file_type: Option<String>,
}

/// `<OLLAMA_MODELS>` when set and non-empty, else `%USERPROFILE%\.ollama\models`
/// (`$HOME` elsewhere). Read at runtime — the env var is frequently relocated.
fn ollama_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("OLLAMA_MODELS") {
        let path = PathBuf::from(root);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    home_dir().map(|home| home.join(".ollama").join("models"))
}

fn ollama_models() -> Vec<LocalModel> {
    let Some(root) = ollama_root() else {
        return Vec::new();
    };
    let manifests = root.join("manifests");
    let mut files = Vec::new();
    walk(&manifests, MAX_MANIFEST_DEPTH, &mut files);
    files
        .iter()
        .filter_map(|manifest| ollama_entry(&root, &manifests, manifest))
        .collect()
}

fn ollama_entry(root: &Path, manifests: &Path, manifest_path: &Path) -> Option<LocalModel> {
    let raw = std::fs::read(manifest_path).ok()?;
    let manifest: OllamaManifest = serde_json::from_slice(&raw).ok()?;
    let layer = manifest
        .layers?
        .into_iter()
        .find(|l| l.media_type.as_deref() == Some(OLLAMA_MODEL_MEDIA_TYPE))?;
    let digest = layer.digest?;

    // A pruned blob would offer a path llama.cpp cannot load — drop the entry.
    let blob = root.join("blobs").join(blob_file_name(&digest));
    if !blob.is_file() {
        return None;
    }

    let segments: Vec<String> = manifest_path
        .strip_prefix(manifests)
        .ok()?
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let name = friendly_name(&segments);
    if name.is_empty() {
        return None;
    }

    let (parameters, quant) = model_meta(root, manifest.config.as_ref());
    Some(LocalModel {
        label: format!("{name} — Ollama"),
        path: blob.to_string_lossy().into_owned(),
        source: "ollama".to_string(),
        size_bytes: layer
            .size
            .unwrap_or_else(|| file_size(&blob).unwrap_or_default()),
        parameters,
        quant,
    })
}

/// `registry.ollama.ai/library/qwen2.5-coder/7b` -> `qwen2.5-coder:7b`.
/// Fewer than two segments must not panic (one segment is returned as-is).
fn friendly_name(segments: &[String]) -> String {
    match segments.len() {
        0 => String::new(),
        1 => segments[0].clone(),
        n => format!("{}:{}", segments[n - 2], segments[n - 1]),
    }
}

/// `sha256:abc` -> `sha256-abc` (blobs are named without the colon).
fn blob_file_name(digest: &str) -> String {
    digest.replace(':', "-")
}

/// `model_type` / `file_type` from the tiny config blob. Missing or garbage
/// metadata yields empty strings, never an error.
fn model_meta(root: &Path, config: Option<&OllamaLayer>) -> (String, String) {
    let Some(digest) = config.and_then(|c| c.digest.as_deref()) else {
        return (String::new(), String::new());
    };
    let Ok(raw) = std::fs::read(root.join("blobs").join(blob_file_name(digest))) else {
        return (String::new(), String::new());
    };
    let Ok(parsed) = serde_json::from_slice::<OllamaConfig>(&raw) else {
        return (String::new(), String::new());
    };
    (
        parsed.model_type.unwrap_or_default().trim().to_string(),
        parsed.file_type.unwrap_or_default().trim().to_string(),
    )
}

// ------------------------------------------------------- other apps (gguf) ---

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Candidate model directories per app, keeping only the ones that exist.
/// A missing app is silent — no error, no log spam.
fn model_roots() -> Vec<(&'static str, &'static str, PathBuf)> {
    let mut roots: Vec<(&'static str, &'static str, PathBuf)> = Vec::new();
    let Some(home) = home_dir() else {
        return roots;
    };
    let user_config = env_dir("APPDATA").into_iter().chain([
        home.join(".config"),
        home.join("Library").join("Application Support"),
    ]);
    let user_data = env_dir("LOCALAPPDATA").into_iter().chain([
        home.join(".local").join("share"),
        home.join("Library").join("Application Support"),
    ]);

    roots.push((
        "LM Studio",
        "lmstudio",
        home.join(".lmstudio").join("models"),
    ));
    for base in user_config {
        roots.push(("Jan", "jan", base.join("Jan").join("data").join("models")));
    }
    for base in user_data {
        roots.push(("GPT4All", "gpt4all", base.join("nomic.ai").join("GPT4All")));
    }

    roots.retain(|(_, _, path)| path.is_dir());
    roots
}

fn gguf_models() -> Vec<LocalModel> {
    let mut models = Vec::new();
    for (app, source, root) in model_roots() {
        let mut files = Vec::new();
        walk(&root, MAX_WALK_DEPTH, &mut files);
        for file in files {
            let Some(file_name) = file.file_name().map(|n| n.to_string_lossy().to_lowercase())
            else {
                continue;
            };
            // `mmproj` files are multimodal projectors, not loadable weights.
            if !file_name.ends_with(".gguf") || file_name.contains("mmproj") {
                continue;
            }
            let Some(size) = file_size(&file) else {
                continue;
            };
            if size < MIN_GGUF_BYTES {
                continue;
            }
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (parameters, quant) = parse_params_quant(&stem);
            models.push(LocalModel {
                label: format!("{stem} — {app}"),
                path: file.to_string_lossy().into_owned(),
                source: source.to_string(),
                size_bytes: size,
                parameters,
                quant,
            });
        }
    }
    models
}

// ------------------------------------------------------------- helpers -----

fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Recursive regular-file walk. Directories are followed by depth only (never
/// symlinked ones, which avoids cycles and broken-link failures); every
/// unreadable entry is skipped.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, out);
            }
        } else if kind.is_file() {
            out.push(path);
        }
    }
}

/// Best-effort `parameters` (`7B`, `1.5B`, `7.6B`) and `quant` (`Q4_K_M`,
/// `IQ3_XS`, `F16`) from a filename stem. Empty strings when absent.
fn parse_params_quant(stem: &str) -> (String, String) {
    (find_parameters(stem), find_quant(stem))
}

fn find_parameters(stem: &str) -> String {
    let bytes = stem.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut seen_dot = false;
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_digit() {
                end += 1;
            } else if b == b'.' && !seen_dot {
                seen_dot = true;
                end += 1;
            } else {
                break;
            }
        }
        // The size suffix must follow immediately: `4B`, `1.5B`, `7.6B`.
        if let Some(&unit) = bytes.get(end) {
            let is_unit = matches!(unit, b'B' | b'b' | b'M' | b'm');
            let boundary = bytes
                .get(end + 1)
                .map_or(true, |next| !next.is_ascii_alphanumeric());
            if is_unit && boundary {
                return stem[start..=end].to_string();
            }
        }
        i = end.max(start + 1);
    }
    String::new()
}

fn find_quant(stem: &str) -> String {
    let upper = stem.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = quant_token_len(&bytes[i..]) {
            return upper[i..i + len].to_string();
        }
        i += 1;
    }
    String::new()
}

/// Length of a quantization token starting at `rest[0]`, or `None`.
/// Recognizes `Q4_K_M`, `Q8_0`, `IQ3_XS`, `F16`, `BF16` — a letter prefix
/// (Q/IQ/F/BF) immediately followed by a digit, then `[A-Z0-9_]`.
fn quant_token_len(rest: &[u8]) -> Option<usize> {
    let prefix = if rest.starts_with(b"BF16") {
        4
    } else if rest.starts_with(b"IQ") {
        2
    } else if rest.starts_with(b"Q") || rest.starts_with(b"F") {
        1
    } else {
        return None;
    };
    if !rest.get(prefix).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut end = prefix;
    while end < rest.len() {
        const OK: &[u8] = &[b'_'];
        if !rest[end].is_ascii_digit()
            && !rest[end].is_ascii_uppercase()
            && !OK.contains(&rest[end])
        {
            break;
        }
        end += 1;
    }
    // A token cannot run straight into another word (`F16GGUF` is not a quant).
    if rest.get(end).is_some_and(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(path: &str, source: &str) -> LocalModel {
        LocalModel {
            label: format!("{source} entry"),
            path: path.to_string(),
            source: source.to_string(),
            size_bytes: 1,
            parameters: String::new(),
            quant: String::new(),
        }
    }

    #[test]
    fn friendly_name_uses_last_two_segments() {
        let segments = ["registry.ollama.ai", "library", "qwen2.5-coder", "7b"].map(String::from);
        assert_eq!(friendly_name(&segments), "qwen2.5-coder:7b");
    }

    #[test]
    fn friendly_name_single_segment_does_not_panic() {
        assert_eq!(friendly_name(&["latest".to_string()]), "latest");
        assert_eq!(friendly_name(&[]), "");
    }

    #[test]
    fn digest_maps_to_blob_file_name() {
        assert_eq!(blob_file_name("sha256:abc"), "sha256-abc");
    }

    #[test]
    fn filename_yields_parameters_and_quant() {
        let (parameters, quant) = parse_params_quant("Qwen3.5-4B-Q4_K_M");
        assert_eq!(parameters, "4B");
        assert_eq!(quant, "Q4_K_M");
    }

    #[test]
    fn unknown_filename_leaves_both_empty() {
        let (parameters, quant) = parse_params_quant("my-finetune-weights");
        assert!(parameters.is_empty());
        assert!(quant.is_empty());
    }

    fn folder_model(id: &str, path: &str, ready: bool) -> FolderModel {
        FolderModel {
            id: id.to_string(),
            label: id.to_string(),
            path: path.to_string(),
            size_bytes: 1,
            ready,
        }
    }

    #[test]
    fn catalog_ids_get_human_labels() {
        assert_eq!(stt_label("v2"), "Parakeet v2 — English");
        assert_eq!(stt_label("v3"), "Parakeet v3 — 25 languages");
        assert_eq!(stt_label("V2"), "Parakeet v2 — English");
        assert_eq!(stt_label("my-parakeet"), "my-parakeet");
        assert_eq!(llm_label("2b"), "Qwen 2B");
        assert_eq!(llm_label("4b"), "Qwen 4B");
        assert_eq!(llm_label("9b"), "Qwen 9B");
        assert_eq!(llm_label("4B"), "Qwen 4B");
        assert_eq!(llm_label("Qwen3.5-4B-Q4_K_M"), "Qwen3.5-4B-Q4_K_M");
    }

    #[test]
    fn only_catalog_sizes_are_model_ids() {
        assert!(is_catalog_llm_id("2b"));
        assert!(is_catalog_llm_id(" 9B "));
        assert!(!is_catalog_llm_id(""));
        assert!(!is_catalog_llm_id("7b"));
        assert!(!is_catalog_llm_id("Qwen3.5-4B-Q4_K_M"));
    }

    #[test]
    fn ready_entries_sort_first_then_by_label() {
        let sorted = sort_entries(vec![
            folder_model("Qwen 9B", "C:/models/llm/9b/a.gguf", true),
            folder_model("Broken", "C:/models/stt/broken", false),
            folder_model("Parakeet v2 — English", "C:/models/stt/v2", true),
            folder_model("Alpha", "C:/models/llm/alpha.gguf", false),
        ]);
        let order: Vec<&str> = sorted.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            order,
            vec!["Parakeet v2 — English", "Qwen 9B", "Alpha", "Broken"]
        );
    }

    #[test]
    fn sorting_deduplicates_by_path() {
        let sorted = sort_entries(vec![
            folder_model("a", r"C:\models\llm\x.gguf", true),
            // Same file, differently cased (Windows paths are case-insensitive).
            folder_model("b", r"c:\models\llm\X.GGUF", true),
        ]);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].label, "a");
    }

    #[test]
    fn folder_scan_lists_ready_and_incomplete_bundles() {
        let root = temp_dir("scan-bundles");
        let stt_root = root.join("stt");
        let ready = stt_root.join("v2");
        let broken = stt_root.join("broken");
        std::fs::create_dir_all(&ready).unwrap();
        std::fs::create_dir_all(&broken).unwrap();
        bundle(
            &ready,
            &["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"],
        );
        bundle(&broken, &["encoder.onnx"]);

        let entries = stt_entries(&stt_root);
        assert_eq!(entries.len(), 2);
        // Sorted: the ready bundle first, then the incomplete one by label.
        assert_eq!(entries[0].label, "Parakeet v2 — English");
        assert!(entries[0].ready);
        assert_eq!(entries[0].size_bytes, 4);
        assert_eq!(entries[1].label, "broken");
        assert!(!entries[1].ready);
        assert_eq!(entries[1].size_bytes, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_scan_finds_gguf_files_one_level_deep() {
        let root = temp_dir("scan-gguf");
        let catalog = root.join("4b");
        std::fs::create_dir_all(&catalog).unwrap();
        std::fs::write(catalog.join("Qwen3.5-4B-Q4_K_M.gguf"), b"xxxxx").unwrap();
        std::fs::write(root.join("mine.GGUF"), b"xx").unwrap();
        // Not weights; must be ignored.
        std::fs::write(root.join("__MACOSX-x.bin"), b"x").unwrap();

        let entries = llm_entries(&root);
        assert_eq!(entries.len(), 2);
        // The catalog folder names the model, not the file.
        let qwen = entries.iter().find(|e| e.path.contains("4b")).unwrap();
        assert_eq!(qwen.id, "4b");
        assert_eq!(qwen.label, "Qwen 4B");
        assert_eq!(qwen.size_bytes, 5);
        assert!(qwen.ready);
        // A loose file is identified by its own stem.
        let mine = entries.iter().find(|e| e.path.ends_with("mine.GGUF")).unwrap();
        assert_eq!(mine.id, "mine");
        assert_eq!(mine.label, "mine");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_folder_scan_is_empty_not_an_error() {
        let entries: Vec<FolderModel> = Vec::new();
        assert!(sort_entries(entries).is_empty());
        let missing = temp_dir("scan-missing");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(stt_entries(&missing).is_empty());
        assert!(llm_entries(&missing).is_empty());
        assert!(stt_entries(Path::new("")).is_empty());
    }

    #[test]
    fn merge_keeps_ollama_entry_over_duplicate_path() {
        let shared = r"D:\OllamaModels\blobs\sha256-60e05f2100071479f596b964f89f510f057ce397ea22f2833a0cfe029bfc2463";
        let merged = merge_models(
            vec![model(shared, "ollama")],
            // Same file reached via a differently-cased plain .gguf entry.
            vec![model(&shared.to_uppercase(), "lmstudio")],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, "ollama");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "smoothflow-stt-folder-{tag}-{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn bundle(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
    }

    #[test]
    fn inspect_accepts_plain_and_int8_bundle_names() {
        for names in [
            ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"],
            [
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ],
        ] {
            let dir = temp_dir("names");
            bundle(&dir, &names);
            let files = inspect_dir(&dir).unwrap();
            assert_eq!(file_name(&files.encoder), names[0]);
            assert_eq!(file_name(&files.decoder), names[1]);
            assert_eq!(file_name(&files.joiner), names[2]);
            assert_eq!(file_name(&files.tokens), "tokens.txt");
            // Absolute, so the helper can load it from any working directory.
            assert!(Path::new(&files.encoder).is_absolute(), "{}", files.encoder);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn int8_wins_over_an_unquantized_file_of_the_same_part() {
        let dir = temp_dir("int8");
        bundle(
            &dir,
            &[
                "encoder.int8.onnx",
                "encoder.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ],
        );
        assert_eq!(
            file_name(&inspect_dir(&dir).unwrap().encoder),
            "encoder.int8.onnx"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let dir = temp_dir("case");
        bundle(
            &dir,
            &["ENCODER.ONNX", "Decoder.Onnx", "JOINER.onnx", "Tokens.TXT"],
        );
        let files = inspect_dir(&dir).unwrap();
        assert_eq!(file_name(&files.encoder), "ENCODER.ONNX");
        assert_eq!(file_name(&files.tokens), "Tokens.TXT");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partial_bundle_names_every_missing_part() {
        let dir = temp_dir("partial");
        bundle(&dir, &["encoder.onnx"]);
        let err = inspect_dir(&dir).unwrap_err();
        assert!(err.starts_with("[STT-101]"), "got: {err}");
        assert!(err.contains("missing: decoder, joiner, tokens"), "got: {err}");
        assert!(err.contains("tokens.txt"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_reported_not_panicked() {
        let dir = temp_dir("gone");
        let _ = std::fs::remove_dir_all(&dir);
        let err = inspect_dir(&dir).unwrap_err();
        assert!(err.starts_with("[STT-102]"), "got: {err}");
        assert!(err.contains("does not exist"), "got: {err}");
        assert!(inspect_dir(Path::new("")).is_err());
    }
}
