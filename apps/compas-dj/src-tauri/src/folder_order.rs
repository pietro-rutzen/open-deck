//! Rename the audio files in one folder so name order matches a chosen sequence.
//! Finder's list view follows that name order. The library database moves cues and
//! analysis onto the new paths in the same operation.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::db;
use crate::is_audio_file;

pub struct FolderReorder {
    pub moves: Vec<(String, String)>,
    pub removed: Vec<String>,
}

/// `paths` is the desired order of library tracks that share one folder.
/// A path whose file is already gone is removed from the library and skipped, so the
/// remaining files can still be renamed. Returns only the paths that actually changed.
pub fn reorder_folder(c: &Connection, paths: &[String]) -> Result<FolderReorder, String> {
    let (present, missing) = present_files(paths)?;
    for path in &missing {
        db::remove_track(c, path).map_err(|error| error.to_string())?;
    }
    if present.len() < 2 {
        return Ok(FolderReorder {
            moves: Vec::new(),
            removed: missing,
        });
    }
    let planned = plan_folder_names(&present)?;
    for (from, _) in &planned {
        if !db::track_exists(c, from).map_err(|e| e.to_string())? {
            return Err(format!("{from} is not in the library"));
        }
    }
    let moves: Vec<(String, String)> = planned
        .into_iter()
        .filter(|(from, to)| from != to)
        .collect();
    if moves.is_empty() {
        return Ok(FolderReorder {
            moves,
            removed: missing,
        });
    }
    apply_renames(&moves)?;
    if let Err(error) = db::relocate_tracks(c, &moves) {
        let reverse: Vec<(String, String)> = moves
            .iter()
            .map(|(from, to)| (to.clone(), from.clone()))
            .collect();
        if let Err(undo) = apply_renames(&reverse) {
            return Err(format!(
                "renamed the files but could not update the library ({error}); restoring them also failed ({undo})"
            ));
        }
        return Err(error.to_string());
    }
    Ok(FolderReorder {
        moves,
        removed: missing,
    })
}

/// Keep files that are still on disk, in the requested order. Paths that no longer
/// exist are returned separately so the library can drop those rows.
fn present_files(paths: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut seen = HashSet::new();
    let mut present = Vec::new();
    let mut missing = Vec::new();
    let mut folder: Option<PathBuf> = None;
    for path in paths {
        if !seen.insert(path.clone()) {
            return Err("The same track was listed twice".to_string());
        }
        let file = PathBuf::from(path);
        if !file.is_file() {
            missing.push(path.clone());
            continue;
        }
        if !is_audio_file(&file) {
            return Err(format!("{path} is not an audio file"));
        }
        let parent = file
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{path} has no folder"))?;
        if let Some(expected) = &folder {
            if expected != &parent {
                return Err("Tracks can only be reordered inside one folder".to_string());
            }
        } else {
            folder = Some(parent);
        }
        present.push(path.clone());
    }
    Ok((present, missing))
}

fn plan_folder_names(paths: &[String]) -> Result<Vec<(String, String)>, String> {
    if paths.len() < 2 {
        return Err("Choose a folder with at least two tracks to reorder".to_string());
    }
    let mut seen = HashSet::new();
    let mut files = Vec::with_capacity(paths.len());
    let mut folder: Option<PathBuf> = None;
    for path in paths {
        if !seen.insert(path.clone()) {
            return Err("The same track was listed twice".to_string());
        }
        let file = PathBuf::from(path);
        if !file.is_file() {
            return Err(format!("{path} is not a file"));
        }
        if !is_audio_file(&file) {
            return Err(format!("{path} is not an audio file"));
        }
        let parent = file
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{path} has no folder"))?;
        if let Some(expected) = &folder {
            if expected != &parent {
                return Err("Tracks can only be reordered inside one folder".to_string());
            }
        } else {
            folder = Some(parent);
        }
        files.push(file);
    }
    let folder = folder.ok_or_else(|| "No folder to reorder".to_string())?;
    let width = files.len().to_string().len().max(2);
    let mut taken = HashSet::new();
    let mut planned = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let name = indexed_name(file, index + 1, width, &mut taken)?;
        let destination = folder.join(name);
        let from = file
            .to_str()
            .ok_or_else(|| format!("{} is not a Unicode path", file.display()))?
            .to_string();
        let to = destination
            .to_str()
            .ok_or_else(|| format!("{} is not a Unicode path", destination.display()))?
            .to_string();
        planned.push((from, to));
    }
    let sources: HashSet<&str> = planned.iter().map(|(from, _)| from.as_str()).collect();
    for (_, to) in &planned {
        let destination = Path::new(to);
        if destination.exists() && !sources.contains(to.as_str()) {
            return Err(format!(
                "{} is already in this folder",
                destination
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(to)
            ));
        }
    }
    Ok(planned)
}

fn indexed_name(
    file: &Path,
    index: usize,
    width: usize,
    taken: &mut HashSet<String>,
) -> Result<String, String> {
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} has no file name", file.display()))?;
    let bare = strip_order_prefix(stem);
    let bare = if bare.is_empty() { "track" } else { bare };
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let mut copy = 1usize;
    loop {
        let titled = if copy == 1 {
            bare.to_string()
        } else {
            format!("{bare} ({copy})")
        };
        let name = if extension.is_empty() {
            format!("{index:0width$} - {titled}")
        } else {
            format!("{index:0width$} - {titled}.{extension}")
        };
        if taken.insert(name.to_lowercase()) {
            return Ok(name);
        }
        copy += 1;
        if copy > 1000 {
            return Err(format!("Could not find a free name for {bare}"));
        }
    }
}

/// Drop a leading index we wrote earlier (`01 - `, `26. `, `03)`). Leave real titles alone.
fn strip_order_prefix(stem: &str) -> &str {
    let mut digits = 0usize;
    for character in stem.chars() {
        if character.is_ascii_digit() && digits < 3 {
            digits += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        return stem;
    }
    let rest = stem[digits..].trim_start();
    let Some(marker) = rest.chars().next() else {
        return stem;
    };
    if !matches!(marker, '.' | '-' | ')') {
        return stem;
    }
    let name = rest[marker.len_utf8()..].trim_start();
    if name.is_empty() {
        stem
    } else {
        name
    }
}

fn apply_renames(moves: &[(String, String)]) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut parked = Vec::with_capacity(moves.len());
    for (index, (from, _)) in moves.iter().enumerate() {
        let source = PathBuf::from(from);
        let parent = source
            .parent()
            .ok_or_else(|| format!("{from} has no folder"))?;
        let temp = parent.join(format!(".opendeck-reorder-{stamp}-{index:04}"));
        fs::rename(&source, &temp).map_err(|error| {
            undo_parked(&parked);
            format!("Could not move {from}: {error}")
        })?;
        parked.push((source, temp));
    }
    for (index, (_, to)) in moves.iter().enumerate() {
        let temp = &parked[index].1;
        if let Err(error) = fs::rename(temp, to) {
            for finished in (0..index).rev() {
                let (original, temp) = &parked[finished];
                let _ = fs::rename(&moves[finished].1, temp);
                let _ = fs::rename(temp, original);
            }
            for (original, temp) in parked.iter().skip(index) {
                let _ = fs::rename(temp, original);
            }
            return Err(format!("Could not name {to}: {error}"));
        }
    }
    Ok(())
}

fn undo_parked(parked: &[(PathBuf, PathBuf)]) {
    for (original, temp) in parked.iter().rev() {
        let _ = fs::rename(temp, original);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_prefix_comes_off_and_a_real_title_stays() {
        assert_eq!(strip_order_prefix("26. BOY CRAZY"), "BOY CRAZY");
        assert_eq!(strip_order_prefix("01 - Title"), "Title");
        assert_eq!(strip_order_prefix("03) Cut"), "Cut");
        assert_eq!(strip_order_prefix("99 Problems"), "99 Problems");
        assert_eq!(strip_order_prefix("1979"), "1979");
    }

    #[test]
    fn renaming_a_folder_writes_name_order_and_can_swap() {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-reorder-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("Zebra.mp3");
        let second = dir.join("01 - Alpha.mp3");
        fs::write(&first, b"a").unwrap();
        fs::write(&second, b"b").unwrap();
        let paths = vec![
            second.to_str().unwrap().to_string(),
            first.to_str().unwrap().to_string(),
        ];
        let planned = plan_folder_names(&paths).unwrap();
        assert_eq!(
            Path::new(&planned[0].1)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("01 - Alpha.mp3")
        );
        assert_eq!(
            Path::new(&planned[1].1)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("02 - Zebra.mp3")
        );
        let swapped = plan_folder_names(&[planned[1].0.clone(), planned[0].0.clone()]).unwrap();
        apply_renames(&swapped).unwrap();
        assert!(dir.join("01 - Zebra.mp3").is_file());
        assert!(dir.join("02 - Alpha.mp3").is_file());
        assert!(!dir.join("Zebra.mp3").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_file_is_dropped_and_the_rest_keep_the_requested_order() {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-reorder-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let alpha = dir.join("Alpha.mp3");
        let zebra = dir.join("Zebra.mp3");
        let deleted = dir.join("20 - Music.mp3");
        fs::write(&alpha, b"a").unwrap();
        fs::write(&zebra, b"z").unwrap();
        let database = db::open_in_memory();
        for path in [&alpha, &zebra, &deleted] {
            db::add_track(&database, path.to_str().unwrap(), "Title", "Artist", 1).unwrap();
        }
        let outcome = reorder_folder(
            &database,
            &[
                deleted.to_str().unwrap().to_string(),
                alpha.to_str().unwrap().to_string(),
                zebra.to_str().unwrap().to_string(),
            ],
        )
        .unwrap();
        assert_eq!(outcome.removed, vec![deleted.to_str().unwrap().to_string()]);
        assert!(!db::track_exists(&database, deleted.to_str().unwrap()).unwrap());
        assert!(dir.join("01 - Alpha.mp3").is_file());
        assert!(dir.join("02 - Zebra.mp3").is_file());
        assert!(db::track_exists(&database, dir.join("01 - Alpha.mp3").to_str().unwrap()).unwrap());
        assert!(db::track_exists(&database, dir.join("02 - Zebra.mp3").to_str().unwrap()).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
}
