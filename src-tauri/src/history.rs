//! SQLite-backed dictation history.
//!
//! Modelled on Handy's history manager (cjpais/Handy): one `dictations` table,
//! schema versioned through `rusqlite_migration` (the `user_version` pragma),
//! and a short-lived `Connection` per operation instead of a pooled or
//! long-lived handle. No async, no audio, no network — every call here is a
//! sub-millisecond-to-millisecond SQLite statement, which is why the Tauri
//! commands in `lib.rs` stay synchronous (AGENTS.md: only multi-hundred-ms work
//! needs `spawn_blocking`).

use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Unsaved rows beyond this many newest rows are pruned after every insert.
/// Starred (`saved = 1`) rows are never pruned automatically.
pub const HISTORY_CAP: usize = 100;

/// Hard ceiling on one page of history, whatever the caller asks for.
const MAX_PAGE: usize = 100;

const SELECT_COLUMNS: &str = "SELECT id, timestamp, transcription_text, \
     post_processed_text, post_process_prompt, post_process_requested, saved, \
     source_name, language, language_source, audio_file, words, model, offline \
     FROM dictations";

/// One row of `dictations`, as the frontend sees it.
///
/// Field names stay snake_case on purpose: the UI reads `transcription_text`
/// and friends directly, and `title` is formatted in JS from `timestamp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub timestamp: i64,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
    pub saved: bool,
    pub source_name: Option<String>,
    pub language: Option<String>,
    pub language_source: Option<String>,
    pub audio_file: Option<String>,
    pub words: u64,
    pub model: Option<String>,
    pub offline: bool,
}

/// A page of history. `has_more` means "another page exists below this one".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

/// Derived stats — there is no counters table, every number is a `COUNT`/`SUM`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStats {
    pub total_dictations: u64,
    pub total_words: u64,
    pub today_dictations: u64,
    pub today_words: u64,
}

/// Fields a new dictation supplies. `source_name` is `Some` for a dictation
/// that came from a picked file (the file's name, kept for reference — the file
/// is never copied), `None` for a normal dictation. `language` /
/// `language_source` stay NULL in this phase (language detection is a later
/// phase). `audio_file` is `Some` only when the user turned `keep_audio_clips`
/// on — the clip is already on disk by then, see `save_audio`.
pub struct NewDictation<'a> {
    pub transcription_text: &'a str,
    pub post_processed_text: Option<&'a str>,
    pub post_process_prompt: Option<&'a str>,
    pub post_process_requested: bool,
    pub audio_file: Option<&'a str>,
    pub source_name: Option<&'a str>,
    pub model: Option<&'a str>,
    pub offline: bool,
}

/// Emitted as the `history-updated` Tauri event. Internally tagged, so the
/// payload is `{"action":"added","entry":{...}}` / `{"action":"deleted","id":n}`
/// / `{"action":"toggled","id":n,"saved":bool}` / `{"action":"updated","entry":{...}}`
/// / `{"action":"cleared"}`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HistoryUpdatePayload {
    Added { entry: HistoryEntry },
    Deleted { id: i64 },
    Toggled { id: i64, saved: bool },
    /// The row's text changed in place — a re-dictation from a saved clip.
    /// The whole row comes back, not just the id, because the text and the
    /// `words` count both moved.
    Updated { entry: HistoryEntry },
    Cleared,
}

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(
        "CREATE TABLE dictations (
            id                     INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp              INTEGER NOT NULL,
            transcription_text     TEXT NOT NULL,
            post_processed_text    TEXT,
            post_process_prompt    TEXT,
            post_process_requested INTEGER NOT NULL DEFAULT 0,
            saved                  INTEGER NOT NULL DEFAULT 0,
            source_name            TEXT,
            language               TEXT,
            language_source        TEXT,
            audio_file             TEXT,
            words                  INTEGER NOT NULL DEFAULT 0,
            model                  TEXT,
            offline                INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX idx_dictations_ts ON dictations(timestamp DESC);",
    )])
}

// Mirrors config.rs `config_dir()`: %APPDATA%/SmoothFlow with a home-dir
// fallback (macOS / Linux). Not reusing the config.rs fn because it's private
// and this module is not allowed to touch config.rs.
fn config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let mut p = PathBuf::from(appdata);
        p.push("SmoothFlow");
        let _ = std::fs::create_dir_all(&p);
        p
    } else {
        // ponytail: fallback to home dir if APPDATA not set (macOS / Linux)
        std::env::home_dir().unwrap_or_default().join("SmoothFlow")
    }
}

pub fn db_path() -> PathBuf {
    let mut p = config_dir();
    p.push("history.db");
    p
}

/// Where dictation clips live: `config_dir()/recordings`.
///
/// This directory only ever holds files this module wrote, which is what makes
/// `clear()` / `reset()` safe to empty wholesale.
pub fn recordings_dir() -> PathBuf {
    let mut p = config_dir();
    p.push("recordings");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Persist one dictation's audio as `sf-<unix_secs>.wav`.
///
/// Returns the bare file name (no directory) to store in the `audio_file`
/// column, or `None` if the write failed. A clip that cannot be written must
/// never fail the dictation — the transcript is the product, the clip is a
/// convenience. Two dictations can land in the same second, so a taken name
/// gets a `-1`, `-2`, … suffix instead of overwriting.
pub fn save_audio(samples: &[f32], sample_rate: u32) -> Option<String> {
    let dir = recordings_dir();
    let stem = format!("sf-{}", now_secs());
    let mut name = format!("{stem}.wav");
    let mut suffix = 0u32;
    while dir.join(&name).exists() {
        suffix += 1;
        name = format!("{stem}-{suffix}.wav");
    }
    match crate::wav::write_wav_16bit_mono(&dir.join(&name), samples, sample_rate) {
        Ok(_) => Some(name),
        Err(e) => {
            crate::sf_log!("HISTORY: could not write the audio clip {name}: {e}");
            None
        }
    }
}

/// Resolve a stored `audio_file` value to an absolute path.
///
/// The value comes straight from the DB (and from the frontend), so it is
/// treated as untrusted: no separators, no `..`, no drive letters. The guard is
/// cheap and this is the only place a clip path is built.
pub fn audio_path(file_name: &str) -> Result<PathBuf, String> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || file_name.contains(':')
    {
        return Err(format!("[HIST-013] Invalid audio file name: {file_name}"));
    }
    Ok(recordings_dir().join(file_name))
}

/// Best-effort delete of one clip. A missing file is not an error — the row may
/// predate clips, or the user may have cleaned the folder by hand.
pub fn remove_audio(file_name: &str) {
    let path = match audio_path(file_name) {
        Ok(p) => p,
        Err(e) => {
            crate::sf_log!("HISTORY: refusing to delete a clip: {e}");
            return;
        }
    };
    match std::fs::remove_file(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => crate::sf_log!("HISTORY: could not delete the clip {}: {e}", path.display()),
    }
}

/// Empty `recordings_dir()`. Only ever called after every row is gone.
fn remove_all_recordings() {
    let dir = recordings_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            crate::sf_log!(
                "HISTORY: could not list the clips in {}: {e}",
                dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            crate::sf_log!("HISTORY: could not delete the clip {}: {e}", path.display());
        }
    }
}

/// Pre-SQLite storage, imported once and then renamed to `history.json.bak`.
fn legacy_json_path() -> PathBuf {
    let mut p = config_dir();
    p.push("history.json");
    p
}

/// Open the DB, bring the schema up to date and run the one-shot importer.
/// One connection per call — never cached, never shared across threads.
fn open() -> Result<Connection, String> {
    let path = db_path();
    let mut conn = Connection::open(&path)
        .map_err(|e| format!("[HIST-001] Could not open the history database: {e}"))?;
    // Two dictation threads (command + hotkey) can collide on a write.
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|e| format!("[HIST-002] Could not configure the history database: {e}"))?;
    migrations()
        .to_latest(&mut conn)
        .map_err(|e| format!("[HIST-003] Could not migrate the history database: {e}"))?;
    import_legacy(&conn)?;
    Ok(conn)
}

/// Called once from `run()`'s setup so the DB (and the legacy import) exists
/// before the first dictation. Failures are logged, never fatal: a broken
/// history DB must not stop the app from transcribing.
pub fn init() {
    match open() {
        Ok(_) => crate::sf_log!("HISTORY: database ready at {}", db_path().display()),
        Err(e) => crate::sf_log!("HISTORY: init failed: {e}"),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        transcription_text: row.get(2)?,
        post_processed_text: row.get(3)?,
        post_process_prompt: row.get(4)?,
        post_process_requested: row.get(5)?,
        saved: row.get(6)?,
        source_name: row.get(7)?,
        language: row.get(8)?,
        language_source: row.get(9)?,
        audio_file: row.get(10)?,
        words: row.get::<_, i64>(11)?.max(0) as u64,
        model: row.get(12)?,
        offline: row.get(13)?,
    })
}

/// Insert a dictation, prune the oldest unsaved rows, and hand back the row.
pub fn add(new: NewDictation<'_>) -> Result<HistoryEntry, String> {
    let conn = open()?;
    let timestamp = now_secs();
    let final_text = new.post_processed_text.unwrap_or(new.transcription_text);
    let words = final_text.split_whitespace().count() as u64;
    conn.execute(
        "INSERT INTO dictations (
             timestamp, transcription_text, post_processed_text, post_process_prompt,
             post_process_requested, saved, audio_file, source_name, words, model, offline
         ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10)",
        params![
            timestamp,
            new.transcription_text,
            new.post_processed_text,
            new.post_process_prompt,
            new.post_process_requested,
            new.audio_file,
            new.source_name,
            words as i64,
            new.model,
            new.offline
        ],
    )
    .map_err(|e| format!("[HIST-004] Could not save the dictation to history: {e}"))?;
    let id = conn.last_insert_rowid();
    prune(&conn)?;
    Ok(HistoryEntry {
        id,
        timestamp,
        transcription_text: new.transcription_text.to_string(),
        post_processed_text: new.post_processed_text.map(str::to_string),
        post_process_prompt: new.post_process_prompt.map(str::to_string),
        post_process_requested: new.post_process_requested,
        saved: false,
        source_name: new.source_name.map(str::to_string),
        language: None,
        language_source: None,
        audio_file: new.audio_file.map(str::to_string),
        words,
        model: new.model.map(str::to_string),
        offline: new.offline,
    })
}

/// Record a dictation whose transcription failed, keeping its clip.
///
/// The row carries no text — `transcription_text` is empty and
/// `post_processed_text` is NULL — so the UI can show it as failed and the
/// user can re-run speech-to-text over `audio_file` via `load_clip` +
/// `update_after_redictation`. Callers must only pass a clip that exists.
///
/// A failed row is identified by convention, not by a schema column:
/// `transcription_text = ''` **and** `audio_file IS NOT NULL`. That is
/// unambiguous — before this existed an empty transcript produced no row at
/// all, and a failure row is never written without a clip — so no migration is
/// needed. Every ordinary row has text, and a clip-less row is only ever a
/// real dictation.
pub fn add_failed(
    audio_file: &str,
    model: Option<&str>,
    offline: bool,
) -> Result<HistoryEntry, String> {
    add(NewDictation {
        transcription_text: "",
        post_processed_text: None,
        post_process_prompt: None,
        post_process_requested: false,
        audio_file: Some(audio_file),
        source_name: None,
        model,
        offline,
    })
}

/// Keep the newest `HISTORY_CAP` rows; drop older ones only if unstarred.
///
/// The clips of exactly the rows about to go are read first, so a pruned row
/// never leaves a WAV behind. `saved = 0` already excludes starred rows.
fn prune(conn: &Connection) -> Result<(), String> {
    let doomed: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT audio_file FROM dictations
                 WHERE saved = 0
                   AND audio_file IS NOT NULL
                   AND id NOT IN (SELECT id FROM dictations ORDER BY id DESC LIMIT ?1)",
            )
            .map_err(|e| format!("[HIST-005] Could not prune history: {e}"))?;
        let rows = stmt
            .query_map(params![HISTORY_CAP as i64], |r| r.get::<_, String>(0))
            .map_err(|e| format!("[HIST-005] Could not prune history: {e}"))?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row.map_err(|e| format!("[HIST-005] Could not prune history: {e}"))?);
        }
        names
    };

    conn.execute(
        "DELETE FROM dictations
         WHERE saved = 0
           AND id NOT IN (SELECT id FROM dictations ORDER BY id DESC LIMIT ?1)",
        params![HISTORY_CAP as i64],
    )
    .map_err(|e| format!("[HIST-005] Could not prune history: {e}"))?;

    for name in &doomed {
        remove_audio(name);
    }
    Ok(())
}

/// Newest-first page. `cursor` is the smallest `id` the caller already has;
/// `limit: None` returns everything (Handy's behaviour), capped at `MAX_PAGE`
/// when a limit is given.
pub fn list(cursor: Option<i64>, limit: Option<usize>) -> Result<HistoryPage, String> {
    let conn = open()?;
    let limit = limit.map(|l| l.clamp(1, MAX_PAGE));

    let mut sql = String::from(SELECT_COLUMNS);
    let mut args: Vec<i64> = Vec::new();
    if let Some(c) = cursor {
        sql.push_str(" WHERE id < ?1");
        args.push(c);
    }
    sql.push_str(" ORDER BY id DESC");
    if let Some(l) = limit {
        // `l` is a clamped usize — inlining it cannot inject anything, and it
        // keeps the parameter indices independent of which filters are active.
        sql.push_str(&format!(" LIMIT {}", l + 1));
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("[HIST-006] Could not read history: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), row_to_entry)
        .map_err(|e| format!("[HIST-006] Could not read history: {e}"))?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("[HIST-006] Could not read history: {e}"))?);
    }

    let has_more = limit.map(|l| entries.len() > l).unwrap_or(false);
    if let Some(l) = limit {
        entries.truncate(l);
    }
    Ok(HistoryPage { entries, has_more })
}

pub fn stats() -> Result<HistoryStats, String> {
    let conn = open()?;
    let (total_dictations, total_words): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(words), 0) FROM dictations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("[HIST-007] Could not read history stats: {e}"))?;
    // Local midnight as a UTC epoch: shift `now` into localtime, truncate the
    // day, then reinterpret as UTC. Keeps the comparison index-friendly.
    let today_start: i64 = conn
        .query_row(
            "SELECT CAST(strftime('%s','now','localtime','start of day','utc') AS INTEGER)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("[HIST-007] Could not read history stats: {e}"))?;
    let (today_dictations, today_words): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(words), 0) FROM dictations WHERE timestamp >= ?1",
            params![today_start],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("[HIST-007] Could not read history stats: {e}"))?;

    Ok(HistoryStats {
        total_dictations: total_dictations.max(0) as u64,
        total_words: total_words.max(0) as u64,
        today_dictations: today_dictations.max(0) as u64,
        today_words: today_words.max(0) as u64,
    })
}

pub fn delete(id: i64) -> Result<(), String> {
    let conn = open()?;
    // Read the clip name before the row goes — afterwards there is nothing to
    // look up and the file would orphan.
    let audio_file: Option<String> = conn
        .query_row(
            "SELECT audio_file FROM dictations WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| format!("[HIST-008] Could not delete the history entry: {e}"))?
        .flatten();
    conn.execute("DELETE FROM dictations WHERE id = ?1", params![id])
        .map_err(|e| format!("[HIST-008] Could not delete the history entry: {e}"))?;
    if let Some(name) = audio_file {
        remove_audio(&name);
    }
    Ok(())
}

/// Load a row's saved clip as `(samples, sample_rate)`.
///
/// `Ok(None)` means "this row has no clip": the setting was off when it was
/// dictated, or it predates clips. That is a normal state, not a failure. A
/// row whose file has since vanished from disk is an error instead, so the
/// caller can tell the user why re-dictation is unavailable rather than
/// silently doing nothing.
pub fn load_clip(id: i64) -> Result<Option<(Vec<f32>, u32)>, String> {
    let conn = open()?;
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT audio_file FROM dictations WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| format!("[HIST-014] Could not read the history entry: {e}"))?;
    // Outer None = no such row. Inner None = the row exists but has no clip.
    let Some(stored) = stored else {
        return Err(format!("[HIST-014] History entry {id} no longer exists"));
    };
    let Some(name) = stored else {
        return Ok(None);
    };
    let path = audio_path(&name)?;
    if !path.is_file() {
        return Err(format!("[HIST-014] The saved clip {name} is no longer on disk"));
    }
    let (samples, rate) = crate::wav::read_wav_16bit_mono(&path)?;
    Ok(Some((samples, rate)))
}

/// Replace a row's text with a fresh re-dictation result.
///
/// Both halves move together on purpose. Re-running the clip produces a new
/// `raw` transcript, and the cleaned text is derived from *that* — storing a
/// new cleaned text beside the original raw would leave the row describing two
/// different recordings. `words` is recomputed from the cleaned text because
/// it feeds the stats page. Returns the refreshed row so the caller can
/// broadcast it to the UI.
pub fn update_after_redictation(
    id: i64,
    raw: &str,
    text: &str,
) -> Result<HistoryEntry, String> {
    let conn = open()?;
    let words = text.split_whitespace().count() as u64;
    let changed = conn
        .execute(
            "UPDATE dictations
                SET transcription_text = ?1, post_processed_text = ?2, words = ?3
              WHERE id = ?4",
            params![raw, text, words as i64, id],
        )
        .map_err(|e| format!("[HIST-015] Could not update the history entry: {e}"))?;
    if changed == 0 {
        return Err(format!("[HIST-015] History entry {id} no longer exists"));
    }
    let sql = format!("{SELECT_COLUMNS} WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_entry)
        .map_err(|e| format!("[HIST-015] Could not read back the history entry: {e}"))
}

/// Flip `saved` and return the new value, so the caller can broadcast it.
pub fn toggle_saved(id: i64) -> Result<bool, String> {
    let conn = open()?;
    conn.execute(
        "UPDATE dictations SET saved = CASE saved WHEN 0 THEN 1 ELSE 0 END WHERE id = ?1",
        params![id],
    )
    .map_err(|e| format!("[HIST-009] Could not update the history entry: {e}"))?;
    let saved: Option<bool> = conn
        .query_row("SELECT saved FROM dictations WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| format!("[HIST-009] Could not update the history entry: {e}"))?;
    saved.ok_or_else(|| format!("[HIST-009] History entry {id} no longer exists"))
}

fn delete_all() -> Result<(), String> {
    let conn = open()?;
    conn.execute("DELETE FROM dictations", [])
        .map_err(|e| format!("[HIST-010] Could not clear history: {e}"))?;
    Ok(())
}

/// Delete every row and every clip. Stats are derived from the table, so
/// clearing history and resetting stats are the same operation — `reset_stats`
/// is kept as its own command because the UI asks for it separately.
pub fn clear() -> Result<(), String> {
    delete_all()?;
    remove_all_recordings();
    Ok(())
}

pub fn reset() -> Result<(), String> {
    delete_all()?;
    remove_all_recordings();
    Ok(())
}

#[derive(Deserialize)]
struct LegacyHistory {
    #[serde(default)]
    entries: Vec<LegacyEntry>,
}

#[derive(Deserialize)]
struct LegacyEntry {
    text: String,
    timestamp: u64,
    words: u64,
}

/// One-shot importer for the old `history.json`.
///
/// Runs only when the JSON exists *and* the table is empty, so it is
/// idempotent. Malformed JSON is logged and skipped — the app continues with an
/// empty DB and the file is left untouched, never deleted. A successful import
/// renames it to `history.json.bak`, which is kept.
fn import_legacy(conn: &Connection) -> Result<(), String> {
    let json_path = legacy_json_path();
    if !json_path.is_file() {
        return Ok(());
    }
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM dictations", [], |r| r.get(0))
        .map_err(|e| format!("[HIST-011] Could not inspect history for import: {e}"))?;
    if existing > 0 {
        return Ok(());
    }
    let raw = match std::fs::read_to_string(&json_path) {
        Ok(s) => s,
        Err(e) => {
            crate::sf_log!("HISTORY: cannot read legacy history.json ({e}) — left in place");
            return Ok(());
        }
    };
    let legacy: LegacyHistory = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            crate::sf_log!(
                "HISTORY: legacy history.json is malformed ({e}) — skipping import, file left alone"
            );
            return Ok(());
        }
    };

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[HIST-011] Could not start the history import: {e}"))?;
    for entry in &legacy.entries {
        tx.execute(
            "INSERT INTO dictations (timestamp, transcription_text, words) VALUES (?1, ?2, ?3)",
            params![entry.timestamp as i64, entry.text, entry.words as i64],
        )
        .map_err(|e| format!("[HIST-011] Could not import a legacy history entry: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("[HIST-011] Could not finish the history import: {e}"))?;

    let backup = json_path.with_extension("json.bak");
    match std::fs::rename(&json_path, &backup) {
        Ok(_) => crate::sf_log!(
            "HISTORY: imported {} legacy entries into history.db (backup: {})",
            legacy.entries.len(),
            backup.display()
        ),
        Err(e) => crate::sf_log!(
            "HISTORY: imported {} entries but could not rename history.json ({e})",
            legacy.entries.len()
        ),
    }
    Ok(())
}
