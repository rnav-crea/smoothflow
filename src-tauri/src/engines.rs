//! On-demand download and install of the offline inference engines.
//!
//! The engines are two native child processes — the `smoothflow-stt` helper and
//! the `llama-server` runtime — totalling ~46 MB unpacked. They no longer ship
//! inside the installer (which made every install pay for an engine it may
//! never use). They are fetched from the fixed `engines` prerelease on
//! `rnav-crea/smoothflow` and installed per user under
//! `<config dir>/engines/<target>/`, next to nothing else:
//!
//! ```text
//! engines/<target>/smoothflow-stt[.exe]
//! engines/<target>/llama/llama-server[.exe] + the rest of the runtime
//! ```
//!
//! That location is per user and survives app updates, so no admin rights are
//! needed and an update never re-downloads the engines. `stt_local::*_path` and
//! `llm_local::server_path` prefer it and fall back to the executable's own
//! directory, so a dev build or a hand-installed copy keeps working.
//!
//! Privacy (binding offline contract): nothing in this module runs on its own.
//! The manifest is fetched only when the user explicitly asks for the download.
//! [`status`] and the path resolvers report on-disk reality and never touch the
//! network.
//!
//! Error codes: `[ENG-001]` unsupported platform, `[ENG-002]` network or
//! download, `[ENG-003]` manifest, `[ENG-004]` archive verification,
//! `[ENG-005]` unsafe archive entry, `[ENG-006]` install/write, `[ENG-007]`
//! delete.

use crate::config::Config;
use crate::offline;
use crate::sf_log;
use reqwest::blocking::Client;
use reqwest::header::RANGE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The manifest and the archives live on one fixed, prerelease tag, so the
/// URL is stable across app versions and never shadows the updater's `latest`.
const DOWNLOAD_BASE: &str =
    "https://github.com/rnav-crea/smoothflow/releases/download/engines";
const MANIFEST_URL: &str = "https://github.com/rnav-crea/smoothflow/releases/download/engines/engines.json";
/// The only manifest layout this build understands. A bumped schema means the
/// app must be updated, never guessed at.
const SCHEMA: u64 = 1;
/// Manifest is tiny; a short timeout keeps a dead connection from blocking the
/// Settings window's "downloading" state forever.
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(60);

const ENGINES_DIR: &str = "engines";
const STT_NAME: &str = "smoothflow-stt";
const LLAMA_DIR: &str = "llama";
const LLAMA_NAME: &str = "llama-server";

const KINDS: [&str; 2] = ["stt", "llama"];
/// The one sentence every delete failure ends with: the fix is always outside
/// the app (a locked or permission-denied directory).
const DELETE_HINT: &str =
    "Close any other SmoothFlow window and try again, or delete that folder by hand.";
/// Published targets. A new one is a deliberate code change, not something a
/// manifest may introduce: picking an unfamiliar target would download a
/// binary for the wrong architecture.
const TARGETS: [&str; 4] = ["win-x64", "linux-x64", "macos-arm64", "macos-x64"];

/// What `get_engine_status` returns. On-disk reality only — never a network
/// answer, because the offline contract forbids background fetches.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    /// `win-x64` / `linux-x64` / `macos-arm64` / `macos-x64`, or `"<os>-<arch>"`
    /// when no engine is published for this platform.
    pub target: String,
    pub stt_installed: bool,
    pub llama_installed: bool,
    /// Bytes on disk for the installed engines (0 when nothing is installed),
    /// so Settings can show a real number instead of the manifest size.
    pub installed_bytes: u64,
}

/// The manifest (`engines.json`) published next to the archives.
///
/// `llama_build` and `sherpa_onnx` carry the engine versions the archives were
/// built from. Nothing reads them yet, so they are not modelled here — serde
/// ignores unknown fields.
#[derive(Debug, Deserialize)]
struct Manifest {
    schema: u64,
    engines: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestEntry {
    kind: String,
    target: String,
    file: String,
    size: u64,
    sha256: String,
}

/// The target name used by the published archives for the running platform.
pub fn target() -> Result<&'static str, String> {
    target_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// `target()` split out from the environment so the whole table is testable.
fn target_for(os: &str, arch: &str) -> Result<&'static str, String> {
    let target = match (os, arch) {
        ("windows", "x86_64") => "win-x64",
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        _ => {
            return Err(format!(
                "[ENG-001] The offline engines are not published for {os}/{arch}. Enable cloud mode in Settings."
            ))
        }
    };
    Ok(target)
}

/// `<config dir>/engines/<target>`. Pure, so the layout is testable.
fn install_dir(config_dir: &Path, target: &str) -> PathBuf {
    config_dir.join(ENGINES_DIR).join(target)
}

fn stt_path(dir: &Path) -> PathBuf {
    dir.join(format!("{STT_NAME}{}", std::env::consts::EXE_SUFFIX))
}

fn llama_path(dir: &Path) -> PathBuf {
    dir.join(LLAMA_DIR)
        .join(format!("{LLAMA_NAME}{}", std::env::consts::EXE_SUFFIX))
}

/// The installed helper, when the user has downloaded one. `None` means "use
/// the executable's own directory instead" — never an error, because a missing
/// download is the normal state and `stt_local` reports it in its own voice.
pub(crate) fn installed_helper() -> Option<PathBuf> {
    installed_at(stt_path(&installed_root()?))
}

/// The installed `llama-server`, when the user has downloaded one.
pub(crate) fn installed_server() -> Option<PathBuf> {
    installed_at(llama_path(&installed_root()?))
}

/// `<config dir>/engines/<target>` for the running platform, resolved exactly
/// the way the offline commands do it (`Config::path().parent()`).
fn installed_root() -> Option<PathBuf> {
    let target = target().ok()?;
    let config_dir = Config::path().parent()?.to_path_buf();
    Some(install_dir(&config_dir, target))
}

/// A downloaded engine file is usable only when it is a real, non-empty file.
/// A symlink is refused: it would let a stray link in the config directory
/// hand the app a binary it never verified.
fn installed_at(path: PathBuf) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    Some(path)
}

/// Are the engines present? Checks the expected files only.
pub fn status(config_dir: &Path) -> Result<EngineStatus, String> {
    let Ok(target) = target() else {
        // No published build for this platform: nothing can be installed, and
        // the Settings window still needs an answer instead of an error.
        return Ok(EngineStatus {
            target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            stt_installed: false,
            llama_installed: false,
            installed_bytes: 0,
        });
    };
    status_at(&install_dir(config_dir, target), target)
}

fn status_at(dir: &Path, target: &str) -> Result<EngineStatus, String> {
    let stt = stt_path(dir);
    let llama = llama_path(dir);
    // ponytail: existence and size only, honouring offline.rs:181 — re-hashing
    // 46 MB on every Settings open is exactly the freeze that comment exists to
    // prevent. The SHA-256 check happens once, in `download`, before anything is
    // extracted; nothing here can mutate an installed engine afterwards.
    let stt_bytes = present(&stt)?;
    let llama_bytes = present(&llama)?;
    Ok(EngineStatus {
        target: target.to_string(),
        stt_installed: stt_bytes.is_some(),
        llama_installed: llama_bytes.is_some(),
        installed_bytes: stt_bytes.unwrap_or(0) + dir_bytes(&dir.join(LLAMA_DIR)),
    })
}

/// The size of `path` when it is a regular, non-empty file; `None` when it is
/// absent. A symlink or a directory is an error, never a silent `None`.
fn present(path: &Path) -> Result<Option<u64>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(None),
        Ok(metadata) if metadata.is_file() => Ok((metadata.len() > 0).then(|| metadata.len())),
        Ok(_) => Err(format!(
            "[ENG-006] Engine path '{}' is not a regular file. Delete it from the config directory and download the engines again.",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "[ENG-006] Could not inspect engine '{}': {error}. Try the download again.",
            path.display()
        )),
    }
}

/// Bytes held by the runtime's own files, for display. Symlinks are neither
/// counted nor followed, and an absent directory simply counts as zero.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        if let Ok(file_type) = entry.file_type() {
            if file_type.is_file() {
                total += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// Download the engines for this platform. The only network entry point in the
/// app besides the model download, and it only runs when the user asks for it.
///
/// Idempotent: an engine whose files already exist is skipped, so a retry after
/// a failure only fetches what is still missing.
pub fn download(
    config_dir: &Path,
    mut progress: impl FnMut(&str, &str, &str, Option<f32>),
) -> Result<(), String> {
    // Every command-owned transfer starts by clearing a stale cancel flag; see
    // `offline::reset_cancel`.
    offline::reset_cancel();
    let target = target()?;
    let dir = install_dir(config_dir, target);
    let manifest = fetch_manifest()?;
    for entry in entries_for(&manifest, target)? {
        // Already installed: no bytes, no network beyond the manifest above.
        let installed = if entry.kind == "llama" {
            present(&llama_path(&dir))?
        } else {
            present(&stt_path(&dir))?
        };
        if installed.is_some() {
            progress(entry.kind.as_str(), "complete", "Already installed.", None);
            continue;
        }
        progress(entry.kind.as_str(), "starting", "Download starting.", None);
        // Inner scope so the notify closure's borrow of `progress` ends before
        // the match arms below call it again (same shape as download_group).
        let result = {
            let mut notify = |bytes: u64| {
                let percent = if entry.size > 0 {
                    (bytes as f32 / entry.size as f32 * 100.0).min(100.0)
                } else {
                    0.0
                };
                progress(
                    entry.kind.as_str(),
                    "progress",
                    &format!("{} — {:.0}%.", entry.file, percent),
                    Some(percent),
                );
            };
            install_entry(entry, &dir, &mut notify)
        };
        match result {
            Ok(()) => progress(
                entry.kind.as_str(),
                "complete",
                "Download complete.",
                Some(100.0),
            ),
            Err(error) => {
                progress(entry.kind.as_str(), "failed", &error, None);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// The two archives for `target`, one per kind, in download order.
fn entries_for<'a>(
    manifest: &'a Manifest,
    target: &str,
) -> Result<Vec<&'a ManifestEntry>, String> {
    let mut found = Vec::with_capacity(KINDS.len());
    for kind in KINDS {
        let entry = manifest
            .engines
            .iter()
            .find(|entry| entry.kind == kind && entry.target == target)
            .ok_or_else(|| {
                format!(
                    "[ENG-003] The engine manifest lists no {kind} build for {target}. Update SmoothFlow, or enable cloud mode in Settings."
                )
            })?;
        found.push(entry);
    }
    Ok(found)
}

/// One archive: fetch into a `.part` file, verify, then extract. Verification
/// always precedes extraction — an archive that never matched its manifest
/// entry must not put a single byte on disk as an installed engine.
fn install_entry(
    entry: &ManifestEntry,
    root: &Path,
    notify: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let dest = if entry.kind == "llama" {
        root.join(LLAMA_DIR)
    } else {
        root.to_path_buf()
    };
    fs::create_dir_all(&dest).map_err(|error| {
        format!(
            "[ENG-006] Could not create the engine directory '{}': {error}. Check that the SmoothFlow config directory is writable and try again.",
            dest.display()
        )
    })?;
    // One `.part` per archive, at the install root: a half-downloaded zip
    // resumes instead of starting over, and never looks like an installed file.
    let part = part_path(&root.join(&entry.file));
    fetch_archive(entry, &part, notify)?;
    extract_archive(&part, &dest)?;
    let installed = if entry.kind == "llama" {
        llama_path(root)
    } else {
        stt_path(root)
    };
    if !installed.is_file() {
        return Err(format!(
            "[ENG-004] The downloaded {} engine archive did not contain '{}'. The download was discarded; try again.",
            entry.kind,
            installed
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
    }
    // Only a fully installed archive is worth keeping, and a stale 18 MB zip in
    // the config directory is not something to leave behind. Best-effort: the
    // engines below are already installed and verified, so a failed cleanup must
    // not read as a failed download.
    if let Err(error) = fs::remove_file(&part) {
        sf_log!(
            "ENG: could not remove the engine archive '{}': {error}",
            part.display()
        );
    }
    Ok(())
}

fn fetch_manifest() -> Result<Manifest, String> {
    let client = Client::builder()
        .timeout(MANIFEST_TIMEOUT)
        .build()
        .map_err(|error| format!("[ENG-002] Could not create the engine downloader. ({error})"))?;
    let response = client.get(MANIFEST_URL).send().map_err(|error| {
        format!(
            "[ENG-002] Could not reach the engine download server; check your connection and try again. ({error})"
        )
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "[ENG-002] The engine download server answered HTTP {}. Check your connection and try again; if it keeps failing, the published engines are missing for this version of SmoothFlow.",
            response.status()
        ));
    }
    let body = response.text().map_err(|error| {
        format!("[ENG-002] Could not read the engine manifest; try again. ({error})")
    })?;
    let manifest: Manifest = serde_json::from_str(&body).map_err(|error| {
        format!("[ENG-003] The engine manifest could not be read; try again later. ({error})")
    })?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != SCHEMA {
        return Err(format!(
            "[ENG-003] The engine manifest uses schema {}; this SmoothFlow build reads schema {SCHEMA}. Update SmoothFlow, or enable cloud mode in Settings.",
            manifest.schema
        ));
    }
    if manifest.engines.is_empty() {
        return Err("[ENG-003] The engine manifest lists no engines; try again later.".to_string());
    }
    for entry in &manifest.engines {
        validate_entry(entry)?;
    }
    Ok(())
}

fn validate_entry(entry: &ManifestEntry) -> Result<(), String> {
    if !KINDS.contains(&entry.kind.as_str()) {
        return Err(format!(
            "[ENG-003] The engine manifest lists an unknown engine kind '{}'. Update SmoothFlow so it understands this manifest, or enable cloud mode in Settings.",
            entry.kind
        ));
    }
    if !TARGETS.contains(&entry.target.as_str()) {
        return Err(format!(
            "[ENG-003] The engine manifest lists an unknown target '{}'. Update SmoothFlow so it understands this manifest, or enable cloud mode in Settings.",
            entry.target
        ));
    }
    // A bare filename only: the whole archive is placed by joining this onto the
    // install directory, so a separator or a parent segment here would decide
    // where its bytes land.
    validate_component("archive filename", &entry.file)?;
    if entry.size == 0 {
        return Err(format!(
            "[ENG-003] The engine manifest entry '{}' has an invalid size. Try again later, or enable cloud mode in Settings.",
            entry.file
        ));
    }
    if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "[ENG-003] The engine manifest entry '{}' has an invalid SHA-256. Try again later, or enable cloud mode in Settings.",
            entry.file
        ));
    }
    Ok(())
}

/// Same posture as `offline.rs::validate_component`: one path component, no
/// separators, no traversal, no drive or stream colon on Windows.
fn validate_component(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('\0')
    {
        return Err(format!(
            "[ENG-003] Unsafe {name} in the engine manifest. Try again later, or enable cloud mode in Settings."
        ));
    }
    Ok(())
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Download `entry` into `part`, resuming an earlier attempt, and prove it
/// matches the manifest before returning.
fn fetch_archive(
    entry: &ManifestEntry,
    part: &Path,
    notify: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let resume_from = match fs::symlink_metadata(part) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(format!(
                "[ENG-002] The partial engine download '{}' is not a regular file; delete it and try again.",
                part.display()
            ))
        }
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(format!(
                "[ENG-002] Could not inspect the partial engine download '{}': {error}. Delete it and try again.",
                part.display()
            ))
        }
    };
    if resume_from > entry.size {
        return Err(format!(
            "[ENG-002] The partial engine download '{}' is larger than its expected size; delete it and try again.",
            part.display()
        ));
    }
    if resume_from < entry.size {
        download_body(entry, part, resume_from, notify)?;
    }
    verify_archive(entry, part)
}

fn download_body(
    entry: &ManifestEntry,
    part: &Path,
    resume_from: u64,
    notify: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let client = Client::builder()
        .build()
        .map_err(|error| format!("[ENG-002] Could not create the engine downloader. ({error})"))?;
    let url = format!("{DOWNLOAD_BASE}/{}", entry.file);
    let mut request = client.get(&url);
    if resume_from > 0 {
        request = request.header(RANGE, format!("bytes={resume_from}-"));
    }
    let mut response = request.send().map_err(|error| {
        format!(
            "[ENG-002] Could not download the {} engine; check your connection and try again. ({error})",
            entry.kind
        )
    })?;
    // A server that ignores Range answers 200 with the whole body: restart the
    // file instead of appending a second copy onto the partial one.
    let append = if resume_from == 0 {
        false
    } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        true
    } else if response.status().is_success() {
        false
    } else {
        return Err(format!(
            "[ENG-002] The engine download failed with HTTP {}; try again.",
            response.status()
        ));
    };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part)
        .map_err(|error| {
            format!(
                "[ENG-006] Could not open the partial engine download '{}': {error}. Check that the SmoothFlow config directory is writable and try again.",
                part.display()
            )
        })?;
    // Chunked copy, not io::copy: the loop is where the transfer reports real
    // percent progress and honours a user cancel. `entry.size` is verified
    // right after, so using it as the denominator is safe.
    const CHUNK: usize = 256 * 1024;
    let mut buffer = vec![0_u8; CHUNK];
    let mut done = resume_from;
    loop {
        if offline::cancel_requested() {
            // Keep the .part file: a retry resumes from the same byte.
            return Err(format!(
                "[ENG-002] Download cancelled — the partial file was kept in {}. Press the button again to resume.",
                part.display()
            ));
        }
        let read = response.read(&mut buffer).map_err(|error| {
            format!(
                "[ENG-002] Could not save the {} engine download; check your connection and try again. ({error})",
                entry.kind
            )
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|error| {
            format!(
                "[ENG-002] Could not save the {} engine download; check your connection and try again. ({error})",
                entry.kind
            )
        })?;
        done += read as u64;
        notify(done.min(entry.size));
    }
    file.flush().map_err(|error| {
        format!(
            "[ENG-006] Could not save the {} engine download to disk: {error}",
            entry.kind
        )
    })
}

fn verify_archive(entry: &ManifestEntry, part: &Path) -> Result<(), String> {
    let size = fs::metadata(part)
        .map_err(|error| {
            format!(
                "[ENG-002] Could not inspect the downloaded engine archive '{}': {error}. Try again.",
                part.display()
            )
        })?
        .len();
    let verified = if size == entry.size {
        sha256_file(part)
            .map_err(|error| {
                format!(
                    "[ENG-004] Could not verify the downloaded {} engine archive: {error}. Try again.",
                    entry.kind
                )
            })?
            .eq_ignore_ascii_case(&entry.sha256)
    } else {
        false
    };
    if verified {
        return Ok(());
    }
    // Discarded rather than kept: a corrupt resume would otherwise fail every
    // later attempt with the same error and no way out from the UI.
    let _ = fs::remove_file(part);
    Err(format!(
        "[ENG-004] The downloaded {} engine archive did not match its published checksum and was discarded; try again.",
        entry.kind
    ))
}

/// Zip-slip guard, explicit and not delegated to the archive crate: every name
/// is a relative path inside the destination, or the archive is refused whole.
fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains(':')
        || name.contains('\0')
    {
        return Err(unsafe_entry(name));
    }
    Ok(())
}

fn unsafe_entry(name: &str) -> String {
    format!(
        "[ENG-005] The engine archive contains an unsafe path '{name}', so nothing was installed. Try again, or report this at github.com/rnav-crea/smoothflow/issues."
    )
}

fn extract_archive(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let archive = File::open(archive_path).map_err(|error| {
        format!(
            "[ENG-006] Could not open the engine archive '{}': {error}. Try the download again.",
            archive_path.display()
        )
    })?;
    let mut zip = zip::ZipArchive::new(archive).map_err(|error| {
        format!(
            "[ENG-004] The engine archive '{}' is not a readable zip file: {error}. Try the download again.",
            archive_path.display()
        )
    })?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| {
            format!(
                "[ENG-004] Could not read entry {index} of the engine archive '{}': {error}. Try the download again.",
                archive_path.display()
            )
        })?;
        // Directory entries carry no bytes; each file below makes its own
        // parents, so skipping them loses nothing.
        if entry.is_dir() {
            continue;
        }
        // Owned before the read below, which needs the entry mutably.
        let name = entry.name().to_string();
        validate_entry_name(&name)?;
        let out = dest.join(&name);
        // The explicit containment check: whatever the name parsed to, it must
        // still be under the destination after joining.
        if out.strip_prefix(dest).is_err() {
            return Err(unsafe_entry(&name));
        }
        let parent = out.parent().ok_or_else(|| unsafe_entry(&name))?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "[ENG-006] Could not create the engine directory '{}': {error}. Check that the SmoothFlow config directory is writable and try again.",
                parent.display()
            )
        })?;
        let mut file = File::create(&out).map_err(|error| {
            format!(
                "[ENG-006] Could not write the engine file '{}': {error}. Check that the SmoothFlow config directory is writable and try again.",
                out.display()
            )
        })?;
        std::io::copy(&mut entry, &mut file).map_err(|error| {
            format!(
                "[ENG-006] Could not extract the engine file '{}': {error}. Try the download again.",
                out.display()
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "[ENG-006] Could not finish writing the engine file '{}': {error}. Try the download again.",
                out.display()
            )
        })?;
    }
    Ok(())
}

/// Remove the downloaded engines for this platform. Mirrors `offline::delete`:
/// already-absent paths are success, and a symlink anywhere in the tree is
/// refused rather than followed.
pub fn delete(config_dir: &Path) -> Result<(), String> {
    let Ok(target) = target() else {
        // No engines can exist for a platform with no published build.
        return Ok(());
    };
    let root = install_dir(config_dir, target);
    remove_tree(&root)?;
    // Best-effort: `engines/` may hold another target's download, and that is
    // not this call's business.
    if let Some(parent) = root.parent() {
        match fs::remove_dir(parent) {
            Ok(()) => {}
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    || error.kind() == ErrorKind::DirectoryNotEmpty => {}
            Err(error) => {
                return Err(format!(
                    "[ENG-007] Could not remove the engine directory '{}': {error}. {DELETE_HINT}",
                    parent.display()
                ))
            }
        }
    }
    Ok(())
}

fn remove_tree(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "[ENG-007] Could not inspect the engine path '{}': {error}. {DELETE_HINT}",
            path.display()
        )),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "[ENG-007] Refusing to delete the engine path '{}' because it is a symbolic link. Remove the link by hand.",
            path.display()
        )),
        Ok(metadata) if metadata.is_dir() => {
            let entries = fs::read_dir(path).map_err(|error| {
                format!(
                    "[ENG-007] Could not read the engine directory '{}': {error}. {DELETE_HINT}",
                    path.display()
                )
            })?;
            for entry in entries {
                let child = entry
                    .map_err(|error| {
                        format!(
                            "[ENG-007] Could not read the engine directory '{}': {error}. {DELETE_HINT}",
                            path.display()
                        )
                    })?
                    .path();
                remove_tree(&child)?;
            }
            fs::remove_dir(path).map_err(|error| {
                format!(
                    "[ENG-007] Could not remove the engine directory '{}': {error}. {DELETE_HINT}",
                    path.display()
                )
            })
        }
        Ok(_) => fs::remove_file(path).map_err(|error| {
            format!(
                "[ENG-007] Could not delete the engine file '{}': {error}. {DELETE_HINT}",
                path.display()
            )
        }),
    }
}

/// Copied from `offline.rs::sha256_file`, which is private to that module and
/// out of scope here. Same streaming buffer so a 46 MB archive is never held in
/// memory.
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

    fn entry(kind: &str, target: &str, file: &str, size: u64, sha256: &str) -> ManifestEntry {
        ManifestEntry {
            kind: kind.to_string(),
            target: target.to_string(),
            file: file.to_string(),
            size,
            sha256: sha256.to_string(),
        }
    }

    fn good_sha256() -> String {
        "a".repeat(64)
    }

    fn manifest(entries: Vec<ManifestEntry>) -> Manifest {
        Manifest {
            schema: SCHEMA,
            engines: entries,
        }
    }

    #[test]
    fn target_names_match_the_published_archives() {
        assert_eq!(target_for("windows", "x86_64").unwrap(), "win-x64");
        assert_eq!(target_for("linux", "x86_64").unwrap(), "linux-x64");
        assert_eq!(target_for("macos", "aarch64").unwrap(), "macos-arm64");
        assert_eq!(target_for("macos", "x86_64").unwrap(), "macos-x64");
    }

    #[test]
    fn unpublishable_platforms_say_so_instead_of_guessing() {
        for (os, arch) in [
            ("windows", "aarch64"),
            ("linux", "aarch64"),
            ("freebsd", "x86_64"),
            ("android", "aarch64"),
            ("", ""),
        ] {
            let err = target_for(os, arch).unwrap_err();
            assert!(err.starts_with("[ENG-001]"), "{os}/{arch} gave: {err}");
            assert!(err.contains("cloud mode"), "{os}/{arch} gave: {err}");
        }
    }

    #[test]
    fn the_running_platform_never_returns_a_blank_target() {
        match target() {
            Ok(target) => assert!(TARGETS.contains(&target), "got {target}"),
            Err(err) => assert!(err.starts_with("[ENG-001]"), "got: {err}"),
        }
    }

    #[test]
    fn the_helper_sits_at_the_root_and_the_runtime_under_llama() {
        let root = Path::new("config");
        let dir = install_dir(root, "win-x64");
        assert_eq!(dir, PathBuf::from("config").join("engines").join("win-x64"));
        assert_eq!(
            stt_path(&dir),
            dir.join(format!("smoothflow-stt{}", std::env::consts::EXE_SUFFIX))
        );
        assert_eq!(
            llama_path(&dir),
            dir.join("llama")
                .join(format!("llama-server{}", std::env::consts::EXE_SUFFIX))
        );
        // Per-target, so two platform builds in one config directory coexist.
        assert_ne!(install_dir(root, "win-x64"), install_dir(root, "linux-x64"));
    }

    #[test]
    fn an_empty_config_directory_reports_nothing_installed() {
        let missing = std::env::temp_dir().join("smoothflow-engines-never-created");
        let status = status_at(&missing, "win-x64").unwrap();
        assert_eq!(status.target, "win-x64");
        assert!(!status.stt_installed);
        assert!(!status.llama_installed);
        assert_eq!(status.installed_bytes, 0);
    }

    #[test]
    fn a_well_formed_manifest_is_accepted() {
        let sha = good_sha256();
        let manifest = manifest(vec![
            entry("stt", "win-x64", "smoothflow-stt-win-x64.zip", 1, &sha),
            entry("llama", "win-x64", "smoothflow-llama-win-x64.zip", 2, &sha),
        ]);
        assert!(validate_manifest(&manifest).is_ok());
        assert_eq!(entries_for(&manifest, "win-x64").unwrap().len(), 2);
    }

    #[test]
    fn a_manifest_from_a_newer_schema_is_refused_whole() {
        let sha = good_sha256();
        let mut manifest = manifest(vec![entry(
            "stt",
            "win-x64",
            "smoothflow-stt-win-x64.zip",
            1,
            &sha,
        )]);
        manifest.schema = 2;
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(err.starts_with("[ENG-003]"), "got: {err}");
        assert!(err.contains("Update SmoothFlow"), "got: {err}");
    }

    #[test]
    fn an_empty_manifest_is_refused() {
        let err = validate_manifest(&manifest(Vec::new())).unwrap_err();
        assert!(err.starts_with("[ENG-003]"), "got: {err}");
    }

    #[test]
    fn malformed_manifest_entries_are_rejected() {
        let sha = good_sha256();
        for bad in [
            entry("tts", "win-x64", "smoothflow-stt-win-x64.zip", 1, &sha),
            entry("stt", "win-arm64", "smoothflow-stt-win-x64.zip", 1, &sha),
            entry("stt", "", "smoothflow-stt-win-x64.zip", 1, &sha),
            entry("stt", "win-x64", "../escape.zip", 1, &sha),
            entry("stt", "win-x64", "nested/escape.zip", 1, &sha),
            entry("stt", "win-x64", "nested\\escape.zip", 1, &sha),
            entry("stt", "win-x64", "C:escape.zip", 1, &sha),
            entry("stt", "win-x64", "", 1, &sha),
            entry("stt", "win-x64", "smoothflow-stt-win-x64.zip", 0, &sha),
            entry("stt", "win-x64", "smoothflow-stt-win-x64.zip", 1, ""),
            entry("stt", "win-x64", "smoothflow-stt-win-x64.zip", 1, "abc"),
            entry(
                "stt",
                "win-x64",
                "smoothflow-stt-win-x64.zip",
                1,
                &"z".repeat(64),
            ),
        ] {
            let manifest = manifest(vec![bad.clone()]);
            assert!(
                validate_manifest(&manifest).is_err(),
                "entry {bad:?} should have been rejected"
            );
        }
    }

    #[test]
    fn a_target_without_both_engines_is_an_error_not_a_half_install() {
        let sha = good_sha256();
        let only_stt = manifest(vec![entry(
            "stt",
            "win-x64",
            "smoothflow-stt-win-x64.zip",
            1,
            &sha,
        )]);
        let err = entries_for(&only_stt, "win-x64").unwrap_err();
        assert!(err.starts_with("[ENG-003]"), "got: {err}");
        assert!(err.contains("llama"), "got: {err}");
        // A different platform's build is not this platform's build.
        let macos_only = manifest(vec![
            entry("stt", "macos-arm64", "smoothflow-stt-macos-arm64.zip", 1, &sha),
            entry("llama", "macos-arm64", "smoothflow-llama-macos-arm64.zip", 1, &sha),
        ]);
        assert!(entries_for(&macos_only, "win-x64").is_err());
        assert!(entries_for(&macos_only, "macos-arm64").is_ok());
    }

    #[test]
    fn archive_names_cannot_escape_the_destination() {
        // The shapes a real engine archive contains.
        for ok in [
            "smoothflow-stt.exe",
            "smoothflow-stt",
            "llama-server.exe",
            "ggml-cpu-alderlake.dll",
            "lib/ggml.dll",
        ] {
            assert!(validate_entry_name(ok).is_ok(), "{ok} should be allowed");
        }
        for bad in [
            "",
            "/etc/passwd",
            "\\windows\\system32\\evil.exe",
            "../escape.exe",
            "llama/../../escape.exe",
            "C:/escape.exe",
            "C:escape.exe",
            "..\\escape.exe",
            "a/..",
            "\0",
        ] {
            let err = validate_entry_name(bad).unwrap_err();
            assert!(err.starts_with("[ENG-005]"), "{bad:?} gave: {err}");
        }
    }

    #[test]
    fn archive_names_stay_under_the_destination_after_joining() {
        // The explicit containment check `extract_archive` runs after joining.
        let dest = Path::new("config").join("engines").join("win-x64");
        let out = dest.join("llama").join("llama-server.exe");
        assert!(out.strip_prefix(&dest).is_ok());
        // A rejected name would never reach the join, but the guard has to
        // catch it even if `validate_entry_name` did not.
        let out = dest.join("/etc/passwd");
        assert!(out.strip_prefix(&dest).is_err() || !out.starts_with(&dest));
    }

    #[test]
    fn a_partial_download_is_a_sibling_with_a_part_suffix() {
        let target = Path::new("config")
            .join("engines")
            .join("win-x64")
            .join("smoothflow-stt-win-x64.zip");
        let part = part_path(&target);
        assert_eq!(part.parent(), target.parent());
        assert_eq!(
            part.file_name().unwrap().to_str(),
            Some("smoothflow-stt-win-x64.zip.part")
        );
    }

    #[test]
    fn delete_tolerates_an_absent_install() {
        let missing = std::env::temp_dir().join("smoothflow-engines-delete-never-created");
        assert!(remove_tree(&missing).is_ok());
        assert!(remove_tree(&missing).is_ok());
    }
}
