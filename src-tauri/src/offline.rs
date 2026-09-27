use reqwest::blocking::Client;
use reqwest::header::RANGE;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
];

pub fn catalog() -> &'static [ModelSpec] {
    CATALOG
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

pub fn download(id: &str, config_dir: &Path, custom_dir: &str) -> Result<PathBuf, String> {
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
    std::io::copy(&mut response, &mut file)
        .map_err(|error| format!("Could not write model '{}': {}", spec.id, error))?;
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

pub fn download_bundle(
    stt_version: &str,
    llm_size: &str,
    config_dir: &Path,
    custom_dir: &str,
    mut progress: impl FnMut(&str, &str, &str),
) -> Result<(), String> {
    let stt_ids: &[&str] = match stt_version {
        "v2" => &["parakeet-v2-encoder", "parakeet-v2-decoder", "parakeet-v2-joiner", "parakeet-v2-tokens"],
        "v3" => &["parakeet-v3-encoder", "parakeet-v3-decoder", "parakeet-v3-joiner", "parakeet-v3-tokens"],
        _ => return Err(format!("Unknown STT model variant '{}'.", stt_version)),
    };
    let llm_id = match llm_size {
        "2b" => "qwen-2b",
        "4b" => "qwen-4b",
        "9b" => "qwen-9b",
        _ => return Err(format!("Unknown LLM model size '{}'.", llm_size)),
    };
    for id in stt_ids.iter().copied().chain(std::iter::once(llm_id)) {
        progress(id, "starting", "Download starting.");
        match download(id, config_dir, custom_dir) {
            Ok(_) => progress(id, "complete", "Download complete."),
            Err(error) => {
                progress(id, "failed", &error);
                return Err(error);
            }
        }
    }
    Ok(())
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
    let url = reqwest::Url::parse(spec.url)
        .map_err(|_| format!("Model '{}' has an invalid URL.", spec.id))?;
    if url.scheme() != "https" || url.host_str() != Some("huggingface.co") || url.port().is_some() || url.username() != "" || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(format!("Model '{}' has an unsafe URL.", spec.id));
    }
    if url.path_segments().map(|mut segments| segments.any(|segment| segment == ".." || segment == ".")).unwrap_or(true) {
        return Err(format!("Model '{}' has unsafe URL path components.", spec.id));
    }
    if spec.expected_size == 0 {
        return Err(format!("Model '{}' has an invalid expected size.", spec.id));
    }
    if let Some(expected_sha256) = spec.expected_sha256 {
        if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("Model '{}' has an invalid SHA-256 manifest entry.", spec.id));
        }
    }
    Ok(())
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
        assert!(validate_spec(&spec).is_err());
    }
}
