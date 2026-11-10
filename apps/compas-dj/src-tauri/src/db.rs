//! SQLite-backed library + per-track performance state.
//!
//! Stores the track list plus everything worth surviving a restart: hot cues, the last loop,
//! the manual beatgrid nudge, a saved gain trim, and play history. This is *not* on the audio
//! path — it's plain blocking `rusqlite` behind a `Mutex`, called only from Tauri commands.
//! A track's absolute file path is its identity (matching `DeckLoadedEvent.path`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Managed Tauri state: the single long-lived connection behind a mutex.
pub struct Db(pub Mutex<Connection>);

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS tracks (
    path              TEXT PRIMARY KEY,
    title             TEXT NOT NULL,
    artist            TEXT NOT NULL,
    duration_ms       INTEGER NOT NULL,
    bpm               REAL,
    bpm_confidence    REAL,
    first_beat_sec    REAL,
    beat_interval_sec REAL,
    key_camelot       TEXT,
    key_name          TEXT,
    grid_offset_sec   REAL NOT NULL DEFAULT 0,
    gain              REAL NOT NULL DEFAULT 1.0,
    added_at          INTEGER NOT NULL,
    last_played_at    INTEGER,
    play_count        INTEGER NOT NULL DEFAULT 0,
    analysis_rev      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cues (
    track_path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
    slot       INTEGER NOT NULL,
    frame      REAL NOT NULL,
    -- NULL keeps a single-point hot cue. A later frame is the loop out of a loop hot cue.
    out_frame  REAL,
    PRIMARY KEY (track_path, slot)
);

CREATE TABLE IF NOT EXISTS loops (
    track_path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
    slot       INTEGER NOT NULL DEFAULT 0,
    in_frame   REAL NOT NULL,
    out_frame  REAL NOT NULL,
    beats      REAL,
    PRIMARY KEY (track_path, slot)
);

CREATE TABLE IF NOT EXISTS memory_loops (
    track_path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
    slot       INTEGER NOT NULL,
    in_frame   REAL NOT NULL,
    out_frame  REAL NOT NULL,
    active     INTEGER NOT NULL DEFAULT 0,
    -- 0 = loop range. 1 = memory cue; out_frame then repeats in_frame.
    point      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (track_path, slot)
);

CREATE TABLE IF NOT EXISTS history (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    track_path TEXT NOT NULL,
    played_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS crates (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    -- 0 = crate (unordered set), 1 = playlist (ordered by position)
    is_playlist INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    -- Non-null for a smart crate: a saved search-grammar query that populates it dynamically.
    query       TEXT
);

CREATE TABLE IF NOT EXISTS crate_tracks (
    crate_id   INTEGER NOT NULL REFERENCES crates(id) ON DELETE CASCADE,
    track_path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
    position   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (crate_id, track_path)
);

CREATE TABLE IF NOT EXISTS track_tags (
    track_path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
    tag        TEXT NOT NULL,
    PRIMARY KEY (track_path, tag)
);

CREATE TABLE IF NOT EXISTS watch_folders (
    path     TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS track_visuals (
    path          TEXT PRIMARY KEY REFERENCES tracks(path) ON DELETE CASCADE,
    file_size     INTEGER NOT NULL,
    modified_ms   INTEGER NOT NULL,
    peaks         BLOB NOT NULL,
    band_peaks    BLOB NOT NULL,
    artwork       BLOB,
    artwork_mime  TEXT,
    title         TEXT,
    artist        TEXT,
    album         TEXT
);
";

/// SQL fragment selecting a comma-joined `tags` column for a track. `path_expr` is how the track's
/// path is referenced in the enclosing query (`path` or `t.path`).
fn tags_select(path_expr: &str) -> String {
    format!("(SELECT GROUP_CONCAT(tag, ',') FROM track_tags tt WHERE tt.track_path = {path_expr}) AS tags")
}

/// Open (creating if needed) the database at `path` and apply the schema.
pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Db> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    // Idempotent migration for DBs created before smart crates: add the column if missing.
    // (Errors — i.e. "duplicate column" — are expected and ignored.)
    migrate(&conn);
    Ok(Db(Mutex::new(conn)))
}

/// Add columns that older library files do not have. Duplicate-column errors are the steady state.
fn migrate(conn: &Connection) {
    let _ = conn.execute("ALTER TABLE crates ADD COLUMN query TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE tracks ADD COLUMN analysis_rev INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE cues ADD COLUMN out_frame REAL", []);
    let _ = conn.execute(
        "ALTER TABLE memory_loops ADD COLUMN point INTEGER NOT NULL DEFAULT 0",
        [],
    );
}

pub struct OpenedLibrary {
    pub db: Db,
    /// Shown when the previous file could not be read and was kept under a new name.
    pub notice: Option<String>,
}

/// Open the library, or move an unreadable file aside and start an empty one.
/// A busy, locked, or non-file path is left where it is.
pub fn open_library(path: impl AsRef<Path>) -> Result<OpenedLibrary, String> {
    let path = path.as_ref();
    // SQLite may unlink a sidecar while rejecting the main file. A hard link keeps those bytes.
    let wal_hold = hold_sidecar(path, "-wal");
    let shm_hold = hold_sidecar(path, "-shm");
    match open(path) {
        Ok(db) => {
            discard_hold(&wal_hold);
            discard_hold(&shm_hold);
            Ok(OpenedLibrary { db, notice: None })
        }
        Err(error) if library_file_should_be_kept_aside(&error) && path.is_file() => {
            let kept = match quarantine_library_file(path) {
                Ok(kept) => kept,
                Err(quarantine_error) => {
                    discard_hold(&wal_hold);
                    discard_hold(&shm_hold);
                    return Err(quarantine_error);
                }
            };
            attach_hold(&wal_hold, &path_with_suffix(&kept, "-wal"));
            attach_hold(&shm_hold, &path_with_suffix(&kept, "-shm"));
            match open(path) {
                Ok(db) => Ok(OpenedLibrary {
                    db,
                    notice: Some(format!(
                        "The library database could not be read. It was kept as {} and a new empty library was started.",
                        kept.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "the saved copy".to_string())
                    )),
                }),
                Err(replacement) => Err(format!(
                    "The library database could not be read and was kept as {}. A replacement could not be created: {replacement}",
                    kept.display()
                )),
            }
        }
        Err(error) => {
            discard_hold(&wal_hold);
            discard_hold(&shm_hold);
            Err(format!(
                "The library database could not be read ({error}). The existing path was left in place."
            ))
        }
    }
}

fn hold_sidecar(path: &Path, suffix: &str) -> Option<PathBuf> {
    let sidecar = path_with_suffix(path, suffix);
    if !sidecar.is_file() {
        return None;
    }
    let hold = path_with_suffix(&sidecar, &format!(".hold-{}", std::process::id()));
    std::fs::hard_link(&sidecar, &hold).ok()?;
    Some(hold)
}

fn discard_hold(hold: &Option<PathBuf>) {
    if let Some(hold) = hold {
        let _ = std::fs::remove_file(hold);
    }
}

fn attach_hold(hold: &Option<PathBuf>, destination: &Path) {
    let Some(hold) = hold else {
        return;
    };
    if destination.exists() {
        let _ = std::fs::remove_file(hold);
        return;
    }
    let _ = std::fs::rename(hold, destination);
}

fn library_file_should_be_kept_aside(error: &rusqlite::Error) -> bool {
    match error {
        rusqlite::Error::SqliteFailure(details, _) => matches!(
            details.code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        ),
        _ => false,
    }
}

fn quarantine_library_file(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or("Library database path has no directory")?;
    let original = path
        .file_name()
        .ok_or("Library database path has no file name")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let mut kept = None;
    for index in 0..1000 {
        let candidate = parent.join(format!(
            "{}.unreadable-{}-{}-{index}",
            original.to_string_lossy(),
            stamp,
            std::process::id()
        ));
        if !candidate.exists() {
            kept = Some(candidate);
            break;
        }
    }
    let kept = kept.ok_or("Could not choose a name for the unreadable library database")?;
    std::fs::rename(path, &kept).map_err(|error| error.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let sidecar = path_with_suffix(path, suffix);
        if !sidecar.exists() {
            continue;
        }
        let destination = path_with_suffix(&kept, suffix);
        if let Err(error) = std::fs::rename(&sidecar, &destination) {
            for (from, to) in moved.into_iter().rev() {
                let _ = std::fs::rename(from, to);
            }
            let _ = std::fs::rename(&kept, path);
            return Err(error.to_string());
        }
        moved.push((destination, sidecar));
    }
    Ok(kept)
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Bump when the offline tempo/key estimator changes so existing rows are analyzed again.
pub const ANALYSIS_REVISION: i64 = 4;

/// A fresh in-memory connection with the schema applied — shared by unit tests across modules
/// (e.g. the `export` round-trip tests) so the schema lives in exactly one place.
#[cfg(test)]
pub(crate) fn open_in_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(SCHEMA).expect("apply schema");
    conn
}

#[derive(Serialize, Clone)]
pub struct TrackRow {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: i64,
    pub bpm: Option<f64>,
    pub key_camelot: Option<String>,
    pub key_name: Option<String>,
    pub grid_offset_sec: f64,
    pub gain: f64,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    /// User tags (lowercased), for `tag:` search and smart crates.
    pub tags: Vec<String>,
}

#[derive(Serialize)]
pub struct CueRow {
    pub slot: i64,
    pub frame: f64,
    /// Loop out of a hot-cue loop. Absent for a single-point hot cue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_frame: Option<f64>,
}

#[derive(Serialize)]
pub struct LoopRow {
    pub slot: i64,
    pub in_frame: f64,
    pub out_frame: f64,
    pub beats: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MemoryLoopRow {
    pub slot: i64,
    pub in_frame: f64,
    pub out_frame: f64,
    pub active: bool,
    /// A saved cue point. Older rows and exports omit this and stay loops.
    #[serde(default, skip_serializing_if = "is_false")]
    pub point: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Everything needed to restore a deck when a track is reloaded.
#[derive(Serialize)]
pub struct TrackState {
    pub grid_offset_sec: f64,
    pub gain: f64,
    pub cues: Vec<CueRow>,
    pub loops: Vec<LoopRow>,
    pub memory_loops: Vec<MemoryLoopRow>,
}

#[derive(Serialize)]
pub struct HistoryRow {
    pub track_path: String,
    pub title: String,
    pub artist: String,
    pub played_at: i64,
}

fn row_to_track(row: &rusqlite::Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        path: row.get("path")?,
        title: row.get("title")?,
        artist: row.get("artist")?,
        duration_ms: row.get("duration_ms")?,
        bpm: row.get("bpm")?,
        key_camelot: row.get("key_camelot")?,
        key_name: row.get("key_name")?,
        grid_offset_sec: row.get("grid_offset_sec")?,
        gain: row.get("gain")?,
        play_count: row.get("play_count")?,
        last_played_at: row.get("last_played_at")?,
        tags: row
            .get::<_, Option<String>>("tags")?
            .map(|s| {
                s.split(',')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub fn list_tracks(c: &Connection) -> rusqlite::Result<Vec<TrackRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT path, title, artist, duration_ms, bpm, key_camelot, key_name,
                grid_offset_sec, gain, play_count, last_played_at, {}
         FROM tracks ORDER BY added_at DESC, title ASC",
        tags_select("path"),
    ))?;
    let rows = stmt.query_map([], row_to_track)?;
    rows.collect()
}

/// Insert a track if absent (keeping any existing analysis/cues), then return its row.
pub fn add_track(
    c: &Connection,
    path: &str,
    title: &str,
    artist: &str,
    duration_ms: i64,
) -> rusqlite::Result<TrackRow> {
    c.execute(
        "INSERT OR IGNORE INTO tracks (path, title, artist, duration_ms, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![path, title, artist, duration_ms, now()],
    )?;
    c.query_row(
        &format!(
            "SELECT path, title, artist, duration_ms, bpm, key_camelot, key_name,
                    grid_offset_sec, gain, play_count, last_played_at, {}
             FROM tracks WHERE path = ?1",
            tags_select("path"),
        ),
        params![path],
        row_to_track,
    )
}

/// Repair only importer placeholders; leave saved edits, cues and analysis intact.
pub fn fill_missing_metadata(
    c: &Connection,
    path: &str,
    title: Option<&str>,
    artist: Option<&str>,
) -> rusqlite::Result<usize> {
    let existing: Option<(String, String)> = c
        .query_row(
            "SELECT title, artist FROM tracks WHERE path = ?1",
            params![path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((old_title, old_artist)) = existing else {
        return Ok(0);
    };
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let placeholder = |s: &str| s.trim().is_empty() || s.trim().eq_ignore_ascii_case("unknown");
    let new_title = title
        .filter(|s| !s.trim().is_empty())
        .filter(|_| placeholder(&old_title) || old_title == stem)
        .unwrap_or(&old_title);
    let new_artist = artist
        .filter(|s| !s.trim().is_empty())
        .filter(|_| placeholder(&old_artist))
        .unwrap_or(&old_artist);
    if new_title == old_title && new_artist == old_artist {
        return Ok(0);
    }
    c.execute(
        "UPDATE tracks SET title = ?2, artist = ?3 WHERE path = ?1",
        params![path, new_title, new_artist],
    )
}

/// Normalize a tag: trimmed, lowercased, commas (the GROUP_CONCAT separator) removed.
fn clean_tag(tag: &str) -> String {
    tag.trim().to_ascii_lowercase().replace(',', "")
}

/// Tag a track (idempotent; no-op for a blank tag).
pub fn add_tag(c: &Connection, path: &str, tag: &str) -> rusqlite::Result<()> {
    let t = clean_tag(tag);
    if t.is_empty() {
        return Ok(());
    }
    c.execute(
        "INSERT OR IGNORE INTO track_tags (track_path, tag) VALUES (?1, ?2)",
        params![path, t],
    )?;
    Ok(())
}

/// Remove a tag from a track.
pub fn remove_tag(c: &Connection, path: &str, tag: &str) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM track_tags WHERE track_path = ?1 AND tag = ?2",
        params![path, clean_tag(tag)],
    )?;
    Ok(())
}

/// Whether a track path is already in the library (so a folder scan can skip re-probing it).
pub fn track_exists(c: &Connection, path: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT 1 FROM tracks WHERE path = ?1",
        params![path],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

/// Register a folder to auto-import from (idempotent).
pub fn add_watch_folder(c: &Connection, path: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO watch_folders (path, added_at) VALUES (?1, ?2)",
        params![path, now()],
    )?;
    Ok(())
}

pub fn remove_watch_folder(c: &Connection, path: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM watch_folders WHERE path = ?1", params![path])?;
    Ok(())
}

pub fn list_watch_folders(c: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare("SELECT path FROM watch_folders ORDER BY path ASC")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

pub fn list_track_paths(c: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare("SELECT path FROM tracks")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn rename_watch_folder(c: &Connection, from: &str, to: &str) -> rusqlite::Result<()> {
    if from == to {
        return Ok(());
    }
    let updated = c.execute(
        "UPDATE watch_folders SET path = ?2 WHERE path = ?1",
        params![from, to],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

pub fn remove_track(c: &Connection, path: &str) -> rusqlite::Result<()> {
    // Children cascade via the FK; history is intentionally a standalone log and kept.
    c.execute("DELETE FROM tracks WHERE path = ?1", params![path])?;
    Ok(())
}

/// Drop library rows under `roots` whose files are confirmed gone. A root that is not a
/// directory (unmounted volume, renamed folder) is ignored so a missing disk cannot empty
/// the library. A file that cannot be stated is kept.
pub fn forget_tracks_missing_under(c: &Connection, roots: &[String]) -> rusqlite::Result<usize> {
    let roots: Vec<String> = roots
        .iter()
        .map(|root| root.trim_end_matches(['/', '\\']).to_string())
        .filter(|root| !root.is_empty() && Path::new(root).is_dir())
        .collect();
    if roots.is_empty() {
        return Ok(0);
    }
    let mut stmt = c.prepare("SELECT path FROM tracks")?;
    let paths: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut removed = 0usize;
    for path in paths {
        let inside = roots.iter().any(|root| {
            path == *root
                || path.starts_with(&format!("{root}/"))
                || path.starts_with(&format!("{root}\\"))
        });
        if !inside || !crate::library_scan::file_is_confirmed_missing(Path::new(&path)) {
            continue;
        }
        remove_track(c, &path)?;
        removed += 1;
    }
    Ok(removed)
}

/// Point every row that identifies a track at its new file path. Cues, loops, tags, visuals,
/// crates and play history move with it. Foreign keys are off for this transaction because the
/// path is the primary key and the swap goes through a temporary key.
pub fn relocate_tracks(c: &Connection, moves: &[(String, String)]) -> rusqlite::Result<()> {
    let changed: Vec<&(String, String)> = moves.iter().filter(|(from, to)| from != to).collect();
    if changed.is_empty() {
        return Ok(());
    }
    c.execute_batch("PRAGMA foreign_keys = OFF")?;
    let outcome = relocate_tracks_unlocked(c, &changed);
    c.execute_batch("PRAGMA foreign_keys = ON")?;
    outcome
}

fn relocate_tracks_unlocked(c: &Connection, moves: &[&(String, String)]) -> rusqlite::Result<()> {
    let tx = c.unchecked_transaction()?;
    for (index, (from, _)) in moves.iter().enumerate() {
        point_track(&tx, from, &format!("\u{1}opendeck-reorder:{index}"))?;
    }
    for (index, (_, to)) in moves.iter().enumerate() {
        point_track(&tx, &format!("\u{1}opendeck-reorder:{index}"), to)?;
    }
    tx.commit()
}

fn point_track(c: &Connection, from: &str, to: &str) -> rusqlite::Result<()> {
    for statement in [
        "UPDATE cues SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE loops SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE memory_loops SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE history SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE crate_tracks SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE track_tags SET track_path = ?2 WHERE track_path = ?1",
        "UPDATE track_visuals SET path = ?2 WHERE path = ?1",
    ] {
        c.execute(statement, params![from, to])?;
    }
    let updated = c.execute(
        "UPDATE tracks SET path = ?2 WHERE path = ?1",
        params![from, to],
    )?;
    if updated == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

/// Cache analysis results onto an existing track row (no-op if the track isn't in the library).
#[allow(clippy::too_many_arguments)]
pub fn upsert_analysis(
    c: &Connection,
    path: &str,
    bpm: f64,
    bpm_confidence: f64,
    first_beat_sec: f64,
    beat_interval_sec: f64,
    key_camelot: &str,
    key_name: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE tracks SET bpm = ?2, bpm_confidence = ?3, first_beat_sec = ?4,
                beat_interval_sec = ?5, key_camelot = ?6, key_name = ?7,
                analysis_rev = ?8
         WHERE path = ?1",
        params![
            path,
            bpm,
            bpm_confidence,
            first_beat_sec,
            beat_interval_sec,
            key_camelot,
            key_name,
            ANALYSIS_REVISION
        ],
    )?;
    Ok(())
}

/// Paths still needing analysis: no BPM yet, or a successful read from an older estimator.
/// A decode failure stores BPM 0 at the current revision and is not retried.
pub fn list_unanalyzed(c: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare(
        "SELECT path FROM tracks
         WHERE bpm IS NULL OR (bpm > 0 AND analysis_rev < ?1)
         ORDER BY added_at DESC",
    )?;
    let rows = stmt.query_map(params![ANALYSIS_REVISION], |r| r.get::<_, String>(0))?;
    rows.collect()
}

pub fn track_state(c: &Connection, path: &str) -> rusqlite::Result<TrackState> {
    let (grid_offset_sec, gain) = c
        .query_row(
            "SELECT grid_offset_sec, gain FROM tracks WHERE path = ?1",
            params![path],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?)),
        )
        .optional()?
        .unwrap_or((0.0, 1.0));

    let mut cue_stmt =
        c.prepare("SELECT slot, frame, out_frame FROM cues WHERE track_path = ?1 ORDER BY slot")?;
    let cues = cue_stmt
        .query_map(params![path], |r| {
            let frame = r.get::<_, f64>(1)?;
            let out_frame = r.get::<_, Option<f64>>(2)?;
            Ok(CueRow {
                slot: r.get(0)?,
                frame,
                out_frame: out_frame.filter(|out| out.is_finite() && *out > frame),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut loop_stmt = c.prepare(
        "SELECT slot, in_frame, out_frame, beats FROM loops WHERE track_path = ?1 ORDER BY slot",
    )?;
    let loops = loop_stmt
        .query_map(params![path], |r| {
            Ok(LoopRow {
                slot: r.get(0)?,
                in_frame: r.get(1)?,
                out_frame: r.get(2)?,
                beats: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut memory_stmt = c.prepare(
        "SELECT slot, in_frame, out_frame, active, point FROM memory_loops WHERE track_path = ?1 ORDER BY slot",
    )?;
    let memory_loops = memory_stmt
        .query_map(params![path], |r| {
            Ok(MemoryLoopRow {
                slot: r.get(0)?,
                in_frame: r.get(1)?,
                out_frame: r.get(2)?,
                active: r.get::<_, i64>(3)? != 0,
                point: r.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(TrackState {
        grid_offset_sec,
        gain,
        cues,
        loops,
        memory_loops,
    })
}

pub fn set_cue(c: &Connection, path: &str, slot: i64, frame: f64) -> rusqlite::Result<()> {
    write_cue(c, path, slot, frame, None)
}

/// Store a hot cue. `out_frame` is kept only when it is a real loop end after `frame`.
pub fn set_cue_range(
    c: &Connection,
    path: &str,
    slot: i64,
    frame: f64,
    out_frame: Option<f64>,
) -> rusqlite::Result<()> {
    let out_frame = out_frame.filter(|out| out.is_finite() && *out > frame);
    write_cue(c, path, slot, frame, out_frame)
}

fn write_cue(
    c: &Connection,
    path: &str,
    slot: i64,
    frame: f64,
    out_frame: Option<f64>,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO cues (track_path, slot, frame, out_frame) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_path, slot) DO UPDATE SET
            frame = excluded.frame, out_frame = excluded.out_frame",
        params![path, slot, frame, out_frame],
    )?;
    Ok(())
}

pub fn clear_cue(c: &Connection, path: &str, slot: i64) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM cues WHERE track_path = ?1 AND slot = ?2",
        params![path, slot],
    )?;
    Ok(())
}

pub fn set_loop(
    c: &Connection,
    path: &str,
    slot: i64,
    in_frame: f64,
    out_frame: f64,
    beats: Option<f64>,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO loops (track_path, slot, in_frame, out_frame, beats) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(track_path, slot) DO UPDATE SET
            in_frame = excluded.in_frame, out_frame = excluded.out_frame, beats = excluded.beats",
        params![path, slot, in_frame, out_frame, beats],
    )?;
    Ok(())
}

pub fn replace_memory_loops(
    c: &Connection,
    path: &str,
    loops: &[MemoryLoopRow],
) -> rusqlite::Result<()> {
    let tx = c.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM memory_loops WHERE track_path = ?1",
        params![path],
    )?;
    let mut armed = false;
    let mut seen_slots = Vec::new();
    for row in loops {
        if row.slot < 0 || !row.in_frame.is_finite() || seen_slots.contains(&row.slot) {
            continue;
        }
        let point = row.point;
        if !point && (!row.out_frame.is_finite() || row.out_frame <= row.in_frame) {
            continue;
        }
        seen_slots.push(row.slot);
        let out_frame = if point { row.in_frame } else { row.out_frame };
        let active = !point && row.active && !armed;
        if active {
            armed = true;
        }
        tx.execute(
            "INSERT INTO memory_loops (track_path, slot, in_frame, out_frame, active, point)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                path,
                row.slot,
                row.in_frame,
                out_frame,
                active as i64,
                point as i64
            ],
        )?;
    }
    tx.commit()
}

pub fn clear_loop(c: &Connection, path: &str, slot: i64) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM loops WHERE track_path = ?1 AND slot = ?2",
        params![path, slot],
    )?;
    Ok(())
}

pub fn set_grid_offset(c: &Connection, path: &str, sec: f64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE tracks SET grid_offset_sec = ?2 WHERE path = ?1",
        params![path, sec],
    )?;
    Ok(())
}

pub fn set_gain(c: &Connection, path: &str, gain: f64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE tracks SET gain = ?2 WHERE path = ?1",
        params![path, gain],
    )?;
    Ok(())
}

fn folder_like_prefix(folder: &str) -> String {
    let trimmed = folder.trim_end_matches(['/', '\\']);
    let slash = if trimmed.contains('\\') && !trimmed.contains('/') {
        '\\'
    } else {
        '/'
    };
    let mut escaped = String::new();
    for ch in trimmed.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push(slash);
    escaped.push('%');
    escaped
}

/// Clear played marks. `folder` limits the reset to files inside that directory.
pub fn reset_play_counts(c: &Connection, folder: Option<&str>) -> rusqlite::Result<usize> {
    match folder {
        None => c.execute(
            "UPDATE tracks SET play_count = 0, last_played_at = NULL WHERE play_count > 0",
            [],
        ),
        Some(folder) => c.execute(
            "UPDATE tracks SET play_count = 0, last_played_at = NULL
             WHERE play_count > 0 AND path LIKE ?1 ESCAPE '\\'",
            params![folder_like_prefix(folder)],
        ),
    }
}

/// Bump play count + last-played and append to the history log.
pub fn record_play(c: &Connection, path: &str) -> rusqlite::Result<()> {
    let ts = now();
    c.execute(
        "UPDATE tracks SET play_count = play_count + 1, last_played_at = ?2 WHERE path = ?1",
        params![path, ts],
    )?;
    c.execute(
        "INSERT INTO history (track_path, played_at) VALUES (?1, ?2)",
        params![path, ts],
    )?;
    Ok(())
}

pub fn history(c: &Connection, limit: i64) -> rusqlite::Result<Vec<HistoryRow>> {
    let mut stmt = c.prepare(
        "SELECT h.track_path, COALESCE(t.title, ''), COALESCE(t.artist, ''), h.played_at
         FROM history h LEFT JOIN tracks t ON t.path = h.track_path
         ORDER BY h.played_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok(HistoryRow {
            track_path: r.get(0)?,
            title: r.get(1)?,
            artist: r.get(2)?,
            played_at: r.get(3)?,
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------------
// Search query language
// ---------------------------------------------------------------------------------

const PITCH_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn flat_to_sharp(note: &str) -> Option<&'static str> {
    match note {
        "DB" => Some("C#"),
        "EB" => Some("D#"),
        "GB" => Some("F#"),
        "AB" => Some("G#"),
        "BB" => Some("A#"),
        _ => None,
    }
}

fn camelot_code(token: &str) -> Option<String> {
    let upper = token.to_ascii_uppercase();
    let (number, mode) = upper.split_at(upper.len().saturating_sub(1));
    if mode != "A" && mode != "B" {
        return None;
    }
    let wheel = number.parse::<u8>().ok()?;
    if (1..=12).contains(&wheel) {
        Some(format!("{wheel}{mode}"))
    } else {
        None
    }
}

fn note_suffix(lower: &str) -> Option<(&'static str, bool)> {
    if lower.ends_with("minor") {
        Some(("minor", true))
    } else if lower.ends_with("major") {
        Some(("major", false))
    } else if lower.ends_with("min") && lower.len() > 3 {
        Some(("min", true))
    } else if lower.ends_with("maj") && lower.len() > 3 {
        Some(("maj", false))
    } else if lower.ends_with('m') && lower.len() > 1 {
        Some(("m", true))
    } else {
        None
    }
}

/// Musical names a token should match: the stored short name (`Am`, `F#`) and the long form.
fn musical_names(token: &str) -> Vec<String> {
    let compact: String = token
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .replace('♯', "#")
        .replace('♭', "b");
    let lower = compact.to_ascii_lowercase();
    let (suffix, minor) = note_suffix(&lower).unwrap_or(("", false));
    let note = &compact[..compact.len() - suffix.len()];
    let mut upper = note.to_ascii_uppercase();
    if let Some(sharp) = flat_to_sharp(&upper) {
        upper = sharp.to_string();
    }
    if !PITCH_NAMES.contains(&upper.as_str()) {
        return Vec::new();
    }
    if minor {
        vec![format!("{upper}m"), format!("{upper} minor")]
    } else {
        vec![upper.clone(), format!("{upper} major")]
    }
}

fn parse_bpm_endpoint(raw: &str) -> Option<f64> {
    let normalized = raw.trim().replace(',', ".");
    let value = normalized.parse::<f64>().ok()?;
    if value.is_finite() && value > 0.0 && value < 400.0 {
        Some(value)
    } else {
        None
    }
}

/// Inclusive display window for one tempo token. `117` covers 117.00–117.99; `117.69` covers the
/// values that round to that text.
fn bpm_window(token: &str) -> Option<(f64, f64)> {
    let normalized = token.trim().replace(',', ".");
    if normalized.contains('-') {
        return None;
    }
    let value = parse_bpm_endpoint(&normalized)?;
    match normalized.split_once('.') {
        Some((_, fraction)) if !fraction.is_empty() => {
            let half = 10f64.powi(-(fraction.len() as i32)) / 2.0;
            Some((value - half, value + half))
        }
        Some(_) => None,
        None => Some((value, value + 1.0)),
    }
}

fn push_bpm_match(token: &str, params: &mut Vec<rusqlite::types::Value>) -> Option<String> {
    use rusqlite::types::Value;
    if let Some((start, end)) = token.split_once('-') {
        let first = parse_bpm_endpoint(start)?;
        let second = parse_bpm_endpoint(end)?;
        let (low, high) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        params.push(Value::Real(low));
        params.push(Value::Real(high));
        return Some("(bpm >= ? AND bpm <= ?)".to_string());
    }
    let (low, high) = bpm_window(token)?;
    params.push(Value::Real(low));
    params.push(Value::Real(high));
    Some("(bpm >= ? AND bpm < ?)".to_string())
}

fn push_key_match(token: &str, params: &mut Vec<rusqlite::types::Value>) -> Option<String> {
    use rusqlite::types::Value;
    let mut parts = Vec::new();
    if let Some(code) = camelot_code(token) {
        params.push(Value::Text(code));
        parts.push("UPPER(key_camelot) = ?".to_string());
    }
    for name in musical_names(token) {
        params.push(Value::Text(name.to_uppercase()));
        parts.push("UPPER(key_name) = ?".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("({})", parts.join(" OR ")))
    }
}

/// Compile a search string into a SQL `WHERE` body + bound parameters. Grammar (space-separated):
/// `bpm:120-128` (range) · `bpm:117` or `117.69` (the tempo as shown) · `key:8A` / `8A` / `Am` ·
/// `artist:foo` / `title:foo` (fuzzy LIKE) · `tag:foo` (exact tag) · a bare word matches title,
/// artist, the displayed tempo, or the key · a `-` prefix negates any term.
/// Unknown `field:` tokens are ignored. Terms are AND-ed within a group; a literal `OR` (or `|`)
/// token starts a new group and groups are OR-ed.
/// Returns `("1=1", [])` for an empty query.
pub fn build_search(query: &str) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value;
    // Each group is a list of AND-ed conditions; groups are OR-ed together.
    let mut groups: Vec<Vec<String>> = vec![Vec::new()];
    let mut params: Vec<Value> = Vec::new();

    for raw in query.split_whitespace() {
        // `OR` / `|` (uppercase, so a lowercase "or" stays a literal search word) opens a new group.
        if raw == "OR" || raw == "|" {
            groups.push(Vec::new());
            continue;
        }
        let (neg, tok) = match raw.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest),
            _ => (false, raw),
        };
        let cond: Option<String> = if let Some((field, val)) = tok.split_once(':') {
            if val.is_empty() {
                None
            } else {
                match field.to_ascii_lowercase().as_str() {
                    "bpm" => push_bpm_match(val, &mut params),
                    "key" => push_key_match(val, &mut params),
                    "artist" => {
                        params.push(Value::Text(format!("%{val}%")));
                        Some("artist LIKE ?".to_string())
                    }
                    "title" => {
                        params.push(Value::Text(format!("%{val}%")));
                        Some("title LIKE ?".to_string())
                    }
                    "tag" => {
                        params.push(Value::Text(val.to_ascii_lowercase()));
                        Some(
                            "EXISTS (SELECT 1 FROM track_tags tt \
                             WHERE tt.track_path = path AND tt.tag = ?)"
                                .to_string(),
                        )
                    }
                    _ => None,
                }
            }
        } else {
            let like = format!("%{tok}%");
            params.push(Value::Text(like.clone()));
            params.push(Value::Text(like));
            let mut clauses = vec!["title LIKE ?".to_string(), "artist LIKE ?".to_string()];
            if let Some(bpm) = push_bpm_match(tok, &mut params) {
                clauses.push(bpm);
            }
            if let Some(key) = push_key_match(tok, &mut params) {
                clauses.push(key);
            }
            Some(format!("({})", clauses.join(" OR ")))
        };

        if let Some(c) = cond {
            let group = groups.last_mut().expect("at least one group");
            group.push(if neg { format!("NOT ({c})") } else { c });
        }
    }

    // Render: AND within each (non-empty) group, OR across groups. A multi-condition group is
    // parenthesised so the OR precedence is correct.
    let rendered: Vec<String> = groups
        .iter()
        .filter(|g| !g.is_empty())
        .map(|g| {
            if g.len() == 1 {
                g[0].clone()
            } else {
                format!("({})", g.join(" AND "))
            }
        })
        .collect();

    match rendered.len() {
        0 => ("1=1".to_string(), params),
        1 => (rendered.into_iter().next().unwrap(), params),
        _ => (rendered.join(" OR "), params),
    }
}

/// Search the library with the [`build_search`] grammar.
pub fn search_tracks(c: &Connection, query: &str) -> rusqlite::Result<Vec<TrackRow>> {
    let (where_body, params) = build_search(query);
    let sql = format!(
        "SELECT path, title, artist, duration_ms, bpm, key_camelot, key_name,
                grid_offset_sec, gain, play_count, last_played_at, {tags}
         FROM tracks WHERE {where_body} ORDER BY added_at DESC, title ASC",
        tags = tags_select("path"),
    );
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_track)?;
    rows.collect()
}

// ---------------------------------------------------------------------------------
// Crates & playlists
// ---------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CrateRow {
    pub id: i64,
    pub name: String,
    pub is_playlist: bool,
    pub track_count: i64,
    /// True for a smart crate (populated by a saved search rather than manual membership).
    pub is_smart: bool,
}

/// Create a crate (`is_playlist = false`) or ordered playlist and return its id.
pub fn create_crate(c: &Connection, name: &str, is_playlist: bool) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO crates (name, is_playlist, created_at) VALUES (?1, ?2, ?3)",
        params![name, is_playlist as i64, now()],
    )?;
    Ok(c.last_insert_rowid())
}

/// Create a smart crate: a saved search-grammar `query` that populates the crate dynamically
/// (see [`build_search`]). Smart crates ignore manual membership.
pub fn create_smart_crate(c: &Connection, name: &str, query: &str) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO crates (name, is_playlist, created_at, query) VALUES (?1, 0, ?2, ?3)",
        params![name, now(), query],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn delete_crate(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM crates WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_crates(c: &Connection) -> rusqlite::Result<Vec<CrateRow>> {
    let mut stmt = c.prepare(
        "SELECT cr.id, cr.name, cr.is_playlist,
                (SELECT COUNT(*) FROM crate_tracks ct WHERE ct.crate_id = cr.id),
                cr.query
         FROM crates cr ORDER BY cr.name ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let query: Option<String> = r.get(4)?;
        Ok(CrateRow {
            id: r.get(0)?,
            name: r.get(1)?,
            is_playlist: r.get::<_, i64>(2)? != 0,
            track_count: r.get(3)?,
            is_smart: query.is_some(),
        })
    })?;
    rows.collect()
}

/// Add a track to a crate at the end (idempotent). Position = current max + 1.
pub fn add_to_crate(c: &Connection, crate_id: i64, path: &str) -> rusqlite::Result<()> {
    let next: i64 = c.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM crate_tracks WHERE crate_id = ?1",
        params![crate_id],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT OR IGNORE INTO crate_tracks (crate_id, track_path, position) VALUES (?1, ?2, ?3)",
        params![crate_id, path, next],
    )?;
    Ok(())
}

pub fn remove_from_crate(c: &Connection, crate_id: i64, path: &str) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM crate_tracks WHERE crate_id = ?1 AND track_path = ?2",
        params![crate_id, path],
    )?;
    Ok(())
}

/// List a crate's tracks. A smart crate (non-null `query`) runs its saved search instead of
/// returning stored membership; manual crates/playlists return their members (playlists ordered
/// by position, crates by title).
pub fn crate_tracks(c: &Connection, crate_id: i64) -> rusqlite::Result<Vec<TrackRow>> {
    // Smart crate? Run its saved query.
    let query: Option<String> = c
        .query_row(
            "SELECT query FROM crates WHERE id = ?1",
            params![crate_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if let Some(q) = query {
        return search_tracks(c, &q);
    }

    let mut stmt = c.prepare(&format!(
        "SELECT t.path, t.title, t.artist, t.duration_ms, t.bpm, t.key_camelot, t.key_name,
                t.grid_offset_sec, t.gain, t.play_count, t.last_played_at, {}
         FROM crate_tracks ct
         JOIN tracks t ON t.path = ct.track_path
         JOIN crates cr ON cr.id = ct.crate_id
         WHERE ct.crate_id = ?1
         ORDER BY CASE WHEN cr.is_playlist = 1 THEN ct.position END ASC,
                  CASE WHEN cr.is_playlist = 0 THEN t.title END ASC",
        tags_select("t.path"),
    ))?;
    let rows = stmt.query_map(params![crate_id], row_to_track)?;
    rows.collect()
}

pub struct CachedVisual {
    pub peaks: Vec<f32>,
    pub band_peaks: Vec<[f32; 3]>,
    pub artwork: Option<Vec<u8>>,
    pub artwork_mime: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

fn pack_f32(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn unpack_f32(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn unpack_bands(bytes: &[u8]) -> Option<Vec<[f32; 3]>> {
    let values = unpack_f32(bytes)?;
    if !values.len().is_multiple_of(3) {
        return None;
    }
    Some(
        values
            .chunks_exact(3)
            .map(|band| [band[0], band[1], band[2]])
            .collect(),
    )
}

pub fn load_track_visual(
    c: &Connection,
    path: &str,
    file_size: i64,
    modified_ms: i64,
) -> rusqlite::Result<Option<CachedVisual>> {
    let row = c
        .query_row(
            "SELECT peaks, band_peaks, artwork, artwork_mime, title, artist, album
             FROM track_visuals WHERE path = ?1 AND file_size = ?2 AND modified_ms = ?3",
            params![path, file_size, modified_ms],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((peaks, bands, artwork, artwork_mime, title, artist, album)) = row else {
        return Ok(None);
    };
    let Some(peaks) = unpack_f32(&peaks).filter(|values| values.len() == 128) else {
        return Ok(None);
    };
    let Some(band_peaks) = unpack_bands(&bands).filter(|values| values.len() == 128) else {
        return Ok(None);
    };
    Ok(Some(CachedVisual {
        peaks,
        band_peaks,
        artwork,
        artwork_mime,
        title,
        artist,
        album,
    }))
}

pub fn save_track_visual(
    c: &Connection,
    path: &str,
    file_size: i64,
    modified_ms: i64,
    peaks: &[f32],
    band_peaks: &[[f32; 3]],
    artwork: Option<&[u8]>,
    artwork_mime: Option<&str>,
    title: Option<&str>,
    artist: Option<&str>,
    album: Option<&str>,
) -> rusqlite::Result<()> {
    if peaks.len() != 128 || band_peaks.len() != 128 {
        return Ok(());
    }
    let flat_bands: Vec<f32> = band_peaks.iter().flatten().copied().collect();
    c.execute(
        "INSERT INTO track_visuals (path, file_size, modified_ms, peaks, band_peaks, artwork, artwork_mime, title, artist, album)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(path) DO UPDATE SET
            file_size = excluded.file_size,
            modified_ms = excluded.modified_ms,
            peaks = excluded.peaks,
            band_peaks = excluded.band_peaks,
            artwork = excluded.artwork,
            artwork_mime = excluded.artwork_mime,
            title = excluded.title,
            artist = excluded.artist,
            album = excluded.album",
        params![
            path,
            file_size,
            modified_ms,
            pack_f32(peaks),
            pack_f32(&flat_bands),
            artwork,
            artwork_mime,
            title,
            artist,
            album,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c
    }

    #[test]
    fn older_cues_and_memory_loops_gain_the_new_columns_as_points_and_loops() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE tracks (
                path TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                grid_offset_sec REAL NOT NULL DEFAULT 0,
                gain REAL NOT NULL DEFAULT 1,
                added_at INTEGER NOT NULL,
                play_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE cues (
                track_path TEXT NOT NULL,
                slot INTEGER NOT NULL,
                frame REAL NOT NULL,
                PRIMARY KEY (track_path, slot)
            );
            CREATE TABLE loops (
                track_path TEXT NOT NULL,
                slot INTEGER NOT NULL DEFAULT 0,
                in_frame REAL NOT NULL,
                out_frame REAL NOT NULL,
                beats REAL,
                PRIMARY KEY (track_path, slot)
            );
            CREATE TABLE memory_loops (
                track_path TEXT NOT NULL,
                slot INTEGER NOT NULL,
                in_frame REAL NOT NULL,
                out_frame REAL NOT NULL,
                active INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (track_path, slot)
            );",
        )
        .unwrap();
        c.execute(
            "INSERT INTO tracks (path, title, artist, duration_ms, added_at) VALUES ('/old.wav', 'Old', 'A', 1, 0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO cues (track_path, slot, frame) VALUES ('/old.wav', 2, 42.0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO memory_loops (track_path, slot, in_frame, out_frame, active) VALUES ('/old.wav', 0, 5.0, 9.0, 0)",
            [],
        )
        .unwrap();
        migrate(&c);
        let state = track_state(&c, "/old.wav").unwrap();
        assert_eq!(state.cues[0].frame, 42.0);
        assert_eq!(state.cues[0].out_frame, None);
        assert!(!state.memory_loops[0].point);
        assert_eq!(state.memory_loops[0].out_frame, 9.0);
        set_cue_range(&c, "/old.wav", 2, 42.0, Some(80.0)).unwrap();
        set_cue(&c, "/old.wav", 1, 10.0).unwrap();
        replace_memory_loops(
            &c,
            "/old.wav",
            &[
                MemoryLoopRow {
                    slot: 0,
                    in_frame: 5.0,
                    out_frame: 9.0,
                    active: false,
                    point: false,
                },
                MemoryLoopRow {
                    slot: 1,
                    in_frame: 12.0,
                    out_frame: 12.0,
                    active: true,
                    point: true,
                },
            ],
        )
        .unwrap();
        let updated = track_state(&c, "/old.wav").unwrap();
        assert_eq!(
            updated
                .cues
                .iter()
                .find(|cue| cue.slot == 2)
                .unwrap()
                .out_frame,
            Some(80.0)
        );
        assert_eq!(
            updated
                .cues
                .iter()
                .find(|cue| cue.slot == 1)
                .unwrap()
                .out_frame,
            None
        );
        let cue = updated
            .memory_loops
            .iter()
            .find(|row| row.slot == 1)
            .unwrap();
        assert!(cue.point);
        assert!(!cue.active);
        assert_eq!(cue.in_frame, 12.0);
    }

    #[test]
    fn memory_loops_replace_and_move_with_the_file() {
        let c = mem();
        add_track(&c, "/a.wav", "A", "Artist", 1000).unwrap();
        replace_memory_loops(
            &c,
            "/a.wav",
            &[
                MemoryLoopRow {
                    slot: 0,
                    in_frame: 10.0,
                    out_frame: 20.0,
                    active: true,
                    point: false,
                },
                MemoryLoopRow {
                    slot: 1,
                    in_frame: 30.0,
                    out_frame: 40.0,
                    active: true,
                    point: false,
                },
            ],
        )
        .unwrap();
        let stored = track_state(&c, "/a.wav").unwrap();
        assert_eq!(stored.memory_loops.len(), 2);
        assert!(stored.memory_loops[0].active);
        assert!(!stored.memory_loops[1].active);
        replace_memory_loops(
            &c,
            "/a.wav",
            &[MemoryLoopRow {
                slot: 3,
                in_frame: 50.0,
                out_frame: 80.0,
                active: false,
                point: false,
            }],
        )
        .unwrap();
        let replaced = track_state(&c, "/a.wav").unwrap();
        assert_eq!(replaced.memory_loops.len(), 1);
        assert_eq!(replaced.memory_loops[0].slot, 3);
        relocate_tracks(&c, &[("/a.wav".to_string(), "/b.wav".to_string())]).unwrap();
        assert!(track_state(&c, "/a.wav").unwrap().memory_loops.is_empty());
        assert_eq!(
            track_state(&c, "/b.wav").unwrap().memory_loops[0].in_frame,
            50.0
        );
    }

    #[test]
    fn more_than_eight_memory_loops_round_trip() {
        let c = mem();
        add_track(&c, "/many.wav", "Many", "Artist", 1000).unwrap();
        let rows: Vec<MemoryLoopRow> = (0..12)
            .map(|slot| MemoryLoopRow {
                slot,
                in_frame: slot as f64 * 100.0,
                out_frame: slot as f64 * 100.0 + 40.0,
                active: slot == 11,
                point: slot == 3,
            })
            .collect();
        replace_memory_loops(&c, "/many.wav", &rows).unwrap();
        let stored = track_state(&c, "/many.wav").unwrap();
        assert_eq!(stored.memory_loops.len(), 12);
        assert_eq!(stored.memory_loops[11].slot, 11);
        assert!(stored.memory_loops[11].active);
        assert_eq!(
            stored.memory_loops.iter().filter(|row| row.active).count(),
            1
        );
        let cue = stored
            .memory_loops
            .iter()
            .find(|row| row.slot == 3)
            .unwrap();
        assert!(cue.point);
        assert!(!cue.active);
        assert_eq!(cue.out_frame, cue.in_frame);
    }

    #[test]
    fn metadata_repair_preserves_saved_state_and_explicit_text() {
        let c = open_in_memory();
        let path = "/tmp/track.flac";
        add_track(&c, path, "track", "Unknown", 1000).unwrap();
        set_gain(&c, path, 0.7).unwrap();
        set_cue(&c, path, 0, 123.0).unwrap();
        add_tag(&c, path, "favorite").unwrap();
        record_play(&c, path).unwrap();
        assert_eq!(
            fill_missing_metadata(&c, path, Some("Real title"), Some("Real artist")).unwrap(),
            1
        );
        assert_eq!(
            fill_missing_metadata(&c, path, Some("Other"), Some("Other")).unwrap(),
            0
        );
        let row = list_tracks(&c).unwrap().remove(0);
        assert_eq!(row.title, "Real title");
        assert_eq!(row.artist, "Real artist");
        assert_eq!(row.gain, 0.7);
        assert_eq!(row.play_count, 1);
        assert_eq!(row.tags, vec!["favorite"]);
        assert_eq!(track_state(&c, path).unwrap().cues[0].frame, 123.0);
    }

    #[test]
    fn add_is_idempotent_and_listed() {
        let c = mem();
        add_track(&c, "/a.mp3", "A", "Artist", 1000).unwrap();
        // Re-adding must not duplicate or clobber.
        add_track(&c, "/a.mp3", "A2", "Artist2", 2000).unwrap();
        let rows = list_tracks(&c).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "A"); // INSERT OR IGNORE keeps the first
        assert_eq!(rows[0].gain, 1.0);
    }

    #[test]
    fn cue_loop_grid_gain_round_trip_and_restore() {
        let c = mem();
        add_track(&c, "/t.flac", "T", "Ar", 5000).unwrap();
        set_cue(&c, "/t.flac", 0, 44_100.5).unwrap();
        set_cue(&c, "/t.flac", 3, 88_200.0).unwrap();
        set_cue(&c, "/t.flac", 0, 100.0).unwrap(); // upsert replaces slot 0
        clear_cue(&c, "/t.flac", 3).unwrap();
        set_loop(&c, "/t.flac", 0, 10.0, 20.0, Some(4.0)).unwrap();
        set_grid_offset(&c, "/t.flac", 0.012).unwrap();
        set_gain(&c, "/t.flac", 1.25).unwrap();

        let st = track_state(&c, "/t.flac").unwrap();
        assert_eq!(st.cues.len(), 1);
        assert_eq!(st.cues[0].slot, 0);
        assert_eq!(st.cues[0].frame, 100.0);
        assert_eq!(st.loops.len(), 1);
        assert_eq!(st.loops[0].beats, Some(4.0));
        assert!((st.grid_offset_sec - 0.012).abs() < 1e-9);
        assert_eq!(st.gain, 1.25);
    }

    #[test]
    fn remove_cascades_children() {
        let c = mem();
        add_track(&c, "/x.wav", "X", "Ar", 1).unwrap();
        set_cue(&c, "/x.wav", 1, 5.0).unwrap();
        set_loop(&c, "/x.wav", 0, 1.0, 2.0, None).unwrap();
        remove_track(&c, "/x.wav").unwrap();
        assert!(list_tracks(&c).unwrap().is_empty());
        let st = track_state(&c, "/x.wav").unwrap(); // defaults for an absent track
        assert!(st.cues.is_empty());
        assert!(st.loops.is_empty());
        assert_eq!(st.gain, 1.0);
    }

    #[test]
    fn record_play_bumps_count_and_history() {
        let c = mem();
        add_track(&c, "/p.mp3", "P", "Ar", 1).unwrap();
        record_play(&c, "/p.mp3").unwrap();
        record_play(&c, "/p.mp3").unwrap();
        let rows = list_tracks(&c).unwrap();
        assert_eq!(rows[0].play_count, 2);
        assert!(rows[0].last_played_at.is_some());
        assert_eq!(history(&c, 10).unwrap().len(), 2);
    }

    #[test]
    fn reset_play_counts_can_clear_one_folder_or_the_library() {
        let c = mem();
        add_track(&c, "/music/br/a.mp3", "A", "Ar", 1).unwrap();
        add_track(&c, "/music/br/deep/b.mp3", "B", "Ar", 1).unwrap();
        add_track(&c, "/music/br2/c.mp3", "C", "Ar", 1).unwrap();
        add_track(&c, "/music/house/d.mp3", "D", "Ar", 1).unwrap();
        for path in [
            "/music/br/a.mp3",
            "/music/br/deep/b.mp3",
            "/music/br2/c.mp3",
            "/music/house/d.mp3",
        ] {
            record_play(&c, path).unwrap();
        }
        assert_eq!(reset_play_counts(&c, Some("/music/br")).unwrap(), 2);
        let played = |path: &str| {
            list_tracks(&c)
                .unwrap()
                .into_iter()
                .find(|row| row.path == path)
                .unwrap()
                .play_count
        };
        assert_eq!(played("/music/br/a.mp3"), 0);
        assert_eq!(played("/music/br/deep/b.mp3"), 0);
        assert_eq!(played("/music/br2/c.mp3"), 1);
        assert_eq!(played("/music/house/d.mp3"), 1);
        assert_eq!(reset_play_counts(&c, None).unwrap(), 2);
        assert_eq!(played("/music/house/d.mp3"), 0);
        assert_eq!(history(&c, 10).unwrap().len(), 4);
    }

    #[test]
    fn analysis_cache_updates_existing_row() {
        let c = mem();
        add_track(&c, "/m.flac", "M", "Ar", 1).unwrap();
        upsert_analysis(&c, "/m.flac", 128.0, 0.9, 0.1, 0.46875, "8A", "A minor").unwrap();
        let rows = list_tracks(&c).unwrap();
        assert_eq!(rows[0].bpm, Some(128.0));
        assert_eq!(rows[0].key_camelot.as_deref(), Some("8A"));
    }

    #[test]
    fn search_filters_fielded_range_key_and_negation() {
        let c = mem();
        add_track(&c, "/a.mp3", "Da Funk", "Daft Punk", 1).unwrap();
        upsert_analysis(&c, "/a.mp3", 123.0, 0.9, 0.0, 0.5, "8A", "A minor").unwrap();
        add_track(&c, "/b.mp3", "Live Set", "Other", 1).unwrap();
        upsert_analysis(&c, "/b.mp3", 128.0, 0.9, 0.0, 0.5, "9A", "E minor").unwrap();

        let r = search_tracks(&c, "artist:daft bpm:120-125").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "/a.mp3");

        let r = search_tracks(&c, "key:9a").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "/b.mp3");

        let r = search_tracks(&c, "-live").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "/a.mp3");

        assert_eq!(search_tracks(&c, "funk").unwrap().len(), 1);
        assert_eq!(search_tracks(&c, "  ").unwrap().len(), 2); // empty → all
    }

    #[test]
    fn search_matches_displayed_bpm_and_key() {
        let c = mem();
        add_track(&c, "/a.mp3", "Samba", "Maya", 1).unwrap();
        upsert_analysis(&c, "/a.mp3", 117.694, 0.9, 0.0, 0.5, "8B", "C").unwrap();
        add_track(&c, "/b.mp3", "Other", "Someone", 1).unwrap();
        upsert_analysis(&c, "/b.mp3", 87.97, 0.9, 0.0, 0.5, "7B", "F#m").unwrap();

        assert_eq!(search_tracks(&c, "117").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "117.69").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "117,69").unwrap()[0].path, "/a.mp3");
        assert!(search_tracks(&c, "118").unwrap().is_empty());
        assert_eq!(search_tracks(&c, "8B").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "8b").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "C").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "F#m").unwrap()[0].path, "/b.mp3");
        assert_eq!(search_tracks(&c, "key:F#m").unwrap()[0].path, "/b.mp3");
        assert_eq!(search_tracks(&c, "bpm:87.97").unwrap()[0].path, "/b.mp3");
        assert_eq!(search_tracks(&c, "Gbm").unwrap()[0].path, "/b.mp3");
    }

    #[test]
    fn search_supports_or_groups() {
        let c = mem();
        add_track(&c, "/a.mp3", "Da Funk", "Daft Punk", 1).unwrap();
        upsert_analysis(&c, "/a.mp3", 123.0, 0.9, 0.0, 0.5, "8A", "A minor").unwrap();
        add_track(&c, "/b.mp3", "Genesis", "Justice", 1).unwrap();
        upsert_analysis(&c, "/b.mp3", 128.0, 0.9, 0.0, 0.5, "9A", "E minor").unwrap();
        add_track(&c, "/c.mp3", "Other", "Someone", 1).unwrap();
        upsert_analysis(&c, "/c.mp3", 100.0, 0.9, 0.0, 0.5, "1A", "Ab minor").unwrap();

        // OR across two artist groups returns both, not the third.
        let mut r = search_tracks(&c, "artist:daft OR artist:justice").unwrap();
        r.sort_by(|x, y| x.path.cmp(&y.path));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].path, "/a.mp3");
        assert_eq!(r[1].path, "/b.mp3");

        // `|` is an alias for OR.
        assert_eq!(search_tracks(&c, "key:8A | key:9A").unwrap().len(), 2);

        // AND binds tighter than OR: (daft AND 120-125) OR 170-175 → only /a.
        let r = search_tracks(&c, "artist:daft bpm:120-125 OR bpm:170-175").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "/a.mp3");

        // A trailing/leading OR with an empty group is ignored.
        assert_eq!(search_tracks(&c, "artist:justice OR").unwrap().len(), 1);
    }

    #[test]
    fn crates_and_playlists_crud_and_ordering() {
        let c = mem();
        add_track(&c, "/1.mp3", "One", "Ar", 1).unwrap();
        add_track(&c, "/2.mp3", "Two", "Ar", 1).unwrap();
        let id = create_crate(&c, "Set A", true).unwrap();
        add_to_crate(&c, id, "/2.mp3").unwrap();
        add_to_crate(&c, id, "/1.mp3").unwrap();
        add_to_crate(&c, id, "/2.mp3").unwrap(); // idempotent

        let tracks = crate_tracks(&c, id).unwrap();
        assert_eq!(tracks.len(), 2);
        // Playlist preserves insertion order.
        assert_eq!(tracks[0].path, "/2.mp3");
        assert_eq!(tracks[1].path, "/1.mp3");

        let crates = list_crates(&c).unwrap();
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].track_count, 2);
        assert!(crates[0].is_playlist);

        remove_from_crate(&c, id, "/2.mp3").unwrap();
        assert_eq!(crate_tracks(&c, id).unwrap().len(), 1);
        // Deleting a track cascades out of the crate.
        remove_track(&c, "/1.mp3").unwrap();
        assert!(crate_tracks(&c, id).unwrap().is_empty());

        delete_crate(&c, id).unwrap();
        assert!(list_crates(&c).unwrap().is_empty());
    }

    #[test]
    fn smart_crate_runs_its_saved_query() {
        let c = mem();
        add_track(&c, "/a.mp3", "Da Funk", "Daft Punk", 1).unwrap();
        upsert_analysis(&c, "/a.mp3", 123.0, 0.9, 0.0, 0.5, "8A", "A minor").unwrap();
        add_track(&c, "/b.mp3", "Genesis", "Justice", 1).unwrap();
        upsert_analysis(&c, "/b.mp3", 128.0, 0.9, 0.0, 0.5, "9A", "E minor").unwrap();

        let id = create_smart_crate(&c, "Daft only", "artist:daft").unwrap();
        // list_crates flags it smart.
        let cr = list_crates(&c).unwrap();
        assert_eq!(cr.len(), 1);
        assert!(cr[0].is_smart);
        assert!(!cr[0].is_playlist);

        // crate_tracks runs the saved query, not stored membership.
        let tracks = crate_tracks(&c, id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, "/a.mp3");

        // Adding a second matching track updates the smart crate automatically.
        add_track(&c, "/c.mp3", "Aerodynamic", "Daft Punk", 1).unwrap();
        assert_eq!(crate_tracks(&c, id).unwrap().len(), 2);

        // A plain crate still returns manual membership (no query).
        let plain = create_crate(&c, "Manual", false).unwrap();
        add_to_crate(&c, plain, "/b.mp3").unwrap();
        assert!(
            !list_crates(&c)
                .unwrap()
                .iter()
                .find(|x| x.id == plain)
                .unwrap()
                .is_smart
        );
        assert_eq!(crate_tracks(&c, plain).unwrap().len(), 1);
    }

    #[test]
    fn tags_attach_search_and_appear_on_rows() {
        let c = mem();
        add_track(&c, "/a.mp3", "A", "Ar", 1).unwrap();
        add_track(&c, "/b.mp3", "B", "Ar", 1).unwrap();
        add_tag(&c, "/a.mp3", "Warmup").unwrap(); // normalized to lowercase
        add_tag(&c, "/a.mp3", "warmup").unwrap(); // idempotent
        add_tag(&c, "/a.mp3", "deep house").unwrap();
        add_tag(&c, "/b.mp3", "peak").unwrap();

        // Tags ride along on track rows.
        let a = list_tracks(&c).unwrap();
        let a_row = a.iter().find(|t| t.path == "/a.mp3").unwrap();
        assert!(a_row.tags.contains(&"warmup".to_string()));
        assert!(a_row.tags.contains(&"deep house".to_string()));
        assert_eq!(a_row.tags.len(), 2);

        // tag: search (exact), and negation.
        assert_eq!(search_tracks(&c, "tag:warmup").unwrap().len(), 1);
        assert_eq!(search_tracks(&c, "tag:warmup").unwrap()[0].path, "/a.mp3");
        assert_eq!(search_tracks(&c, "-tag:warmup").unwrap().len(), 1); // → /b
        assert_eq!(search_tracks(&c, "-tag:warmup").unwrap()[0].path, "/b.mp3");
        // Combines with OR and other fields → smart-crate-able.
        assert_eq!(
            search_tracks(&c, "tag:warmup OR tag:peak").unwrap().len(),
            2
        );

        // Remove a tag; it drops from rows and search.
        remove_tag(&c, "/a.mp3", "warmup").unwrap();
        assert!(search_tracks(&c, "tag:warmup").unwrap().is_empty());

        // Deleting a track cascades its tags.
        remove_track(&c, "/b.mp3").unwrap();
        assert!(search_tracks(&c, "tag:peak").unwrap().is_empty());
    }

    #[test]
    fn watch_folders_crud_and_track_exists() {
        let c = mem();
        assert!(list_watch_folders(&c).unwrap().is_empty());
        add_watch_folder(&c, "/music/house").unwrap();
        add_watch_folder(&c, "/music/house").unwrap(); // idempotent
        add_watch_folder(&c, "/music/techno").unwrap();
        let f = list_watch_folders(&c).unwrap();
        assert_eq!(
            f,
            vec!["/music/house".to_string(), "/music/techno".to_string()]
        );

        add_track(&c, "/music/house/a.mp3", "A", "Ar", 1).unwrap();
        assert!(track_exists(&c, "/music/house/a.mp3").unwrap());
        assert!(!track_exists(&c, "/music/house/missing.mp3").unwrap());

        remove_watch_folder(&c, "/music/house").unwrap();
        assert_eq!(
            list_watch_folders(&c).unwrap(),
            vec!["/music/techno".to_string()]
        );
    }

    #[test]
    fn list_unanalyzed_returns_only_tracks_without_bpm() {
        let c = mem();
        add_track(&c, "/a.mp3", "A", "Ar", 1).unwrap();
        add_track(&c, "/b.mp3", "B", "Br", 1).unwrap();
        // Both start un-analyzed (BPM NULL).
        assert_eq!(list_unanalyzed(&c).unwrap().len(), 2);

        // Caching analysis onto /a.mp3 drops it from the work list.
        upsert_analysis(&c, "/a.mp3", 128.0, 0.9, 0.01, 0.468, "8A", "A minor").unwrap();
        assert_eq!(list_unanalyzed(&c).unwrap(), vec!["/b.mp3".to_string()]);

        // A 0-BPM sentinel (decode failure / no beat) still counts as attempted → not retried.
        upsert_analysis(&c, "/b.mp3", 0.0, 0.0, 0.0, 0.0, "", "").unwrap();
        assert!(list_unanalyzed(&c).unwrap().is_empty());
    }

    #[test]
    fn track_visual_cache_matches_file_stamp_and_discards_a_changed_file() {
        let c = mem();
        add_track(&c, "/music/a.mp3", "A", "Ar", 1).unwrap();
        let peaks = vec![0.25_f32; 128];
        let bands = vec![[0.1_f32, 0.2, 0.3]; 128];
        save_track_visual(
            &c,
            "/music/a.mp3",
            100,
            50,
            &peaks,
            &bands,
            Some(&[0xff, 0xd8, 0xff]),
            Some("image/jpeg"),
            Some("Title"),
            Some("Artist"),
            Some("Album"),
        )
        .unwrap();
        let hit = load_track_visual(&c, "/music/a.mp3", 100, 50)
            .unwrap()
            .expect("cached preview");
        assert_eq!(hit.peaks, peaks);
        assert_eq!(hit.band_peaks, bands);
        assert_eq!(hit.artwork.as_deref(), Some(&[0xff, 0xd8, 0xff][..]));
        assert_eq!(hit.title.as_deref(), Some("Title"));
        assert!(load_track_visual(&c, "/music/a.mp3", 101, 50)
            .unwrap()
            .is_none());
    }

    #[test]
    fn relocating_a_track_keeps_its_cue_on_the_new_path() {
        let c = mem();
        add_track(&c, "/music/a.mp3", "A", "Ar", 1).unwrap();
        set_cue(&c, "/music/a.mp3", 0, 42.0).unwrap();
        relocate_tracks(
            &c,
            &[("/music/a.mp3".to_string(), "/music/01 - a.mp3".to_string())],
        )
        .unwrap();
        assert!(!track_exists(&c, "/music/a.mp3").unwrap());
        assert!(track_exists(&c, "/music/01 - a.mp3").unwrap());
        let state = track_state(&c, "/music/01 - a.mp3").unwrap();
        assert_eq!(state.cues.len(), 1);
        assert_eq!(state.cues[0].slot, 0);
        assert_eq!(state.cues[0].frame, 42.0);
    }

    #[test]
    fn a_deleted_file_inside_a_watched_folder_leaves_the_library() {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.mp3");
        std::fs::write(&kept, b"a").unwrap();
        let deleted = dir.join("20 - Music.mp3");
        let outside = "/other/stay.mp3";
        let c = mem();
        add_track(&c, kept.to_str().unwrap(), "Kept", "A", 1).unwrap();
        add_track(&c, deleted.to_str().unwrap(), "Gone", "A", 1).unwrap();
        add_track(&c, outside, "Stay", "A", 1).unwrap();
        let root = dir.to_str().unwrap().to_string();
        assert_eq!(forget_tracks_missing_under(&c, &[root]).unwrap(), 1);
        assert!(track_exists(&c, kept.to_str().unwrap()).unwrap());
        assert!(!track_exists(&c, deleted.to_str().unwrap()).unwrap());
        assert!(track_exists(&c, outside).unwrap());
        assert_eq!(
            forget_tracks_missing_under(&c, &["/no/such/opendeck-folder".to_string()]).unwrap(),
            0
        );
        assert!(track_exists(&c, kept.to_str().unwrap()).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-db-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn startup_library_cold_open_creates_an_empty_database() {
        let dir = scratch_dir("cold");
        let path = dir.join("compas.db");
        let opened = open_library(&path).unwrap();
        assert!(opened.notice.is_none());
        let connection = opened.db.0.lock().unwrap();
        assert!(list_tracks(&connection).unwrap().is_empty());
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_library_empty_file_is_a_new_database() {
        let dir = scratch_dir("empty");
        let path = dir.join("compas.db");
        std::fs::write(&path, b"").unwrap();
        let opened = open_library(&path).unwrap();
        assert!(opened.notice.is_none());
        let connection = opened.db.0.lock().unwrap();
        assert!(list_tracks(&connection).unwrap().is_empty());
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_library_keeps_an_old_schema_and_its_tracks() {
        let dir = scratch_dir("old");
        let path = dir.join("compas.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE tracks (
                    path TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    artist TEXT NOT NULL,
                    duration_ms INTEGER NOT NULL,
                    bpm REAL,
                    bpm_confidence REAL,
                    first_beat_sec REAL,
                    beat_interval_sec REAL,
                    key_camelot TEXT,
                    key_name TEXT,
                    grid_offset_sec REAL NOT NULL DEFAULT 0,
                    gain REAL NOT NULL DEFAULT 1.0,
                    added_at INTEGER NOT NULL,
                    last_played_at INTEGER,
                    play_count INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE crates (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    is_playlist INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO tracks (path, title, artist, duration_ms, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["/music/old.mp3", "Old", "Catalog", 1000, 1],
            )
            .unwrap();
        drop(connection);
        let opened = open_library(&path).unwrap();
        assert!(opened.notice.is_none());
        let connection = opened.db.0.lock().unwrap();
        assert!(track_exists(&connection, "/music/old.mp3").unwrap());
        assert_eq!(
            list_unanalyzed(&connection).unwrap(),
            vec!["/music/old.mp3".to_string()]
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_library_quarantines_a_corrupt_database_without_deleting_it() {
        let dir = scratch_dir("corrupt");
        let path = dir.join("compas.db");
        let original = b"not a sqlite database";
        std::fs::write(&path, original).unwrap();
        std::fs::write(dir.join("compas.db-wal"), b"wal-bytes").unwrap();
        let opened = open_library(&path).unwrap();
        let notice = opened.notice.expect("corrupt file is reported");
        assert!(notice.contains("kept as"));
        let connection = opened.db.0.lock().unwrap();
        assert!(list_tracks(&connection).unwrap().is_empty());
        drop(connection);
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|entry| {
                entry
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".unreadable-"))
                    && !name_ends_with_sidecar(entry)
            })
            .expect("quarantine file");
        assert_eq!(std::fs::read(&kept).unwrap(), original);
        assert_eq!(
            std::fs::read(path_with_suffix(&kept, "-wal")).unwrap(),
            b"wal-bytes"
        );
        if dir.join("compas.db-wal").exists() {
            assert_ne!(
                std::fs::read(dir.join("compas.db-wal")).unwrap(),
                b"wal-bytes"
            );
        }
        assert!(path.is_file());
        let held = std::fs::read_dir(&dir).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.contains(".hold-"))
        });
        assert!(!held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn name_ends_with_sidecar(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-wal") || name.ends_with("-shm"))
    }

    #[test]
    fn startup_library_leaves_a_database_directory_untouched() {
        let dir = scratch_dir("directory");
        let path = dir.join("compas.db");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep.txt"), b"keep").unwrap();
        let error = match open_library(&path) {
            Err(error) => error,
            Ok(_) => panic!("a directory is not a library database"),
        };
        assert!(error.contains("left in place"));
        assert_eq!(std::fs::read(path.join("keep.txt")).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_artwork_cache_with_a_short_blob_is_a_miss() {
        let connection = mem();
        add_track(&connection, "/music/a.mp3", "A", "Ar", 1).unwrap();
        connection
            .execute(
                "INSERT INTO track_visuals (path, file_size, modified_ms, peaks, band_peaks)
                 VALUES (?1, 10, 20, ?2, ?3)",
                params!["/music/a.mp3", vec![1_u8, 2, 3], vec![4_u8, 5, 6, 7]],
            )
            .unwrap();
        assert!(load_track_visual(&connection, "/music/a.mp3", 10, 20)
            .unwrap()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn startup_rescan_keeps_a_track_behind_an_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("unreadable");
        let hidden = dir.join("hidden");
        std::fs::create_dir(&hidden).unwrap();
        let track = hidden.join("song.mp3");
        std::fs::write(&track, b"ID3").unwrap();
        let connection = mem();
        let track_path = track.to_str().unwrap().to_string();
        add_track(&connection, &track_path, "Hidden", "A", 1).unwrap();
        let mut permissions = std::fs::metadata(&hidden).unwrap().permissions();
        permissions.set_mode(0);
        std::fs::set_permissions(&hidden, permissions.clone()).unwrap();
        let removed =
            forget_tracks_missing_under(&connection, &[dir.to_str().unwrap().to_string()]).unwrap();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hidden, permissions).unwrap();
        assert_eq!(removed, 0);
        assert!(track_exists(&connection, &track_path).unwrap());
        std::fs::remove_file(&track).unwrap();
        assert_eq!(
            forget_tracks_missing_under(&connection, &[dir.to_str().unwrap().to_string()]).unwrap(),
            1
        );
        assert!(!track_exists(&connection, &track_path).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
