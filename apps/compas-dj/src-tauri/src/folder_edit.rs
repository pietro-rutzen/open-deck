//! Rename or delete a library folder on disk and keep the database on the new paths.
//! A watched root and any nested watch move with a rename. Delete moves the directory
//! to the Trash, then drops the tracks and watches that lived inside it.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::db;

#[derive(Debug)]
pub struct LibraryFolderEdit {
    pub from: String,
    pub to: Option<String>,
    pub moves: Vec<(String, String)>,
    pub removed: Vec<String>,
    /// Watched paths whose registration changed. `None` means the watch was removed.
    pub watches: Vec<(String, Option<String>)>,
}

pub fn rename_library_folder(
    connection: &Connection,
    folder: &str,
    name: &str,
) -> Result<LibraryFolderEdit, String> {
    let folder = library_directory(connection, folder)?;
    let next_name = folder_name(name)?;
    let current_name = Path::new(&folder)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "This folder can't be renamed".to_string())?;
    if next_name == current_name {
        return Ok(LibraryFolderEdit {
            from: folder.clone(),
            to: Some(folder),
            moves: Vec::new(),
            removed: Vec::new(),
            watches: Vec::new(),
        });
    }
    let destination = destination_path(&folder, next_name)?;
    if directories_collide(Path::new(&folder), Path::new(&destination)) {
        return Err("A file or folder with that name already exists".to_string());
    }
    let watches = db::list_watch_folders(connection).map_err(|error| error.to_string())?;
    let watch_moves = shifted_watches(&watches, &folder, &destination)?;
    let moves = shifted_tracks(connection, &folder, &destination)?;
    for (_, to) in &moves {
        if db::track_exists(connection, to).map_err(|error| error.to_string())? {
            return Err(format!("{to} is already in the library"));
        }
    }
    rename_directory(Path::new(&folder), Path::new(&destination))?;
    if let Err(error) = db::relocate_tracks(connection, &moves) {
        let undo = rename_directory(Path::new(&destination), Path::new(&folder));
        return Err(restore_message(
            "update the library",
            &error.to_string(),
            undo,
        ));
    }
    if let Err(error) = apply_watch_renames(connection, &watch_moves) {
        let tracks_restored = restore_tracks(connection, &moves);
        let folder_restored = rename_directory(Path::new(&destination), Path::new(&folder));
        let undo = match (tracks_restored, folder_restored) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(undo), _) | (_, Err(undo)) => Err(undo),
        };
        return Err(restore_message("update the watched folders", &error, undo));
    }
    Ok(LibraryFolderEdit {
        from: folder,
        to: Some(destination),
        moves,
        removed: Vec::new(),
        watches: watch_moves
            .into_iter()
            .map(|(from, to)| (from, Some(to)))
            .collect(),
    })
}

pub fn delete_library_folder(
    connection: &Connection,
    folder: &str,
) -> Result<LibraryFolderEdit, String> {
    delete_library_folder_with(connection, folder, move_directory_to_trash)
}

fn delete_library_folder_with(
    connection: &Connection,
    folder: &str,
    remove_directory: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<LibraryFolderEdit, String> {
    let folder = library_directory(connection, folder)?;
    let watches = db::list_watch_folders(connection).map_err(|error| error.to_string())?;
    let removed_watches = watches_inside(&watches, &folder);
    let removed = tracks_inside(connection, &folder)?;
    remove_directory(Path::new(&folder))?;
    for path in &removed {
        if let Err(error) = db::remove_track(connection, path) {
            return Err(format!(
                "Moved {folder}, but could not remove {path} from the library: {error}"
            ));
        }
    }
    for path in &removed_watches {
        if let Err(error) = db::remove_watch_folder(connection, path) {
            return Err(format!(
                "Moved {folder}, but could not stop watching {path}: {error}"
            ));
        }
    }
    Ok(LibraryFolderEdit {
        from: folder,
        to: None,
        moves: Vec::new(),
        removed,
        watches: removed_watches
            .into_iter()
            .map(|path| (path, None))
            .collect(),
    })
}

fn library_directory(connection: &Connection, folder: &str) -> Result<String, String> {
    let folder = normalize_stored(folder);
    let path = Path::new(&folder);
    if !path.is_absolute() || path_is_rootish(path) {
        return Err("Choose a folder in the library".to_string());
    }
    if !path.is_dir() {
        return Err("This folder is no longer on disk".to_string());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("This folder is no longer on disk ({error})"))?;
    if path_is_rootish(&canonical) {
        return Err("Choose a folder in the library".to_string());
    }
    let watches = db::list_watch_folders(connection).map_err(|error| error.to_string())?;
    let inside_library = watches.iter().any(|watch| {
        fs::canonicalize(watch)
            .map(|root| canonical == root || canonical.starts_with(&root))
            .unwrap_or(false)
    });
    if !inside_library {
        return Err("Rename and delete are only available for library folders".to_string());
    }
    Ok(folder)
}

fn folder_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a folder name".to_string());
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return Err("Choose a visible folder name".to_string());
    }
    if name.len() > 255 {
        return Err("That folder name is too long".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains(':') || name.contains('\0') {
        return Err("A folder name can't include / : or \\".to_string());
    }
    if name.chars().any(|symbol| symbol.is_control()) {
        return Err("A folder name can't include control characters".to_string());
    }
    Ok(name)
}

fn destination_path(folder: &str, name: &str) -> Result<String, String> {
    let parent = Path::new(folder)
        .parent()
        .ok_or_else(|| "This folder can't be renamed".to_string())?;
    path_text(&parent.join(name))
}

fn directories_collide(from: &Path, to: &Path) -> bool {
    if !to.exists() {
        return false;
    }
    match (fs::canonicalize(from), fs::canonicalize(to)) {
        (Ok(source), Ok(target)) => source != target,
        _ => true,
    }
}

/// A case-only rename on a case-insensitive disk has to leave the old name first.
fn rename_directory(from: &Path, to: &Path) -> Result<(), String> {
    let case_only = matches!(
        (fs::canonicalize(from), to.exists()),
        (Ok(source), true) if fs::canonicalize(to).ok().as_ref() == Some(&source)
    );
    if !case_only {
        return fs::rename(from, to)
            .map_err(|error| format!("Could not rename {}: {error}", from.display()));
    }
    let parent = from
        .parent()
        .ok_or_else(|| "This folder can't be renamed".to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".opendeck-rename-{stamp}"));
    fs::rename(from, &temporary)
        .map_err(|error| format!("Could not rename {}: {error}", from.display()))?;
    if let Err(error) = fs::rename(&temporary, to) {
        let _ = fs::rename(&temporary, from);
        return Err(format!("Could not rename {}: {error}", from.display()));
    }
    Ok(())
}

fn shifted_tracks(
    connection: &Connection,
    folder: &str,
    destination: &str,
) -> Result<Vec<(String, String)>, String> {
    let paths = db::list_track_paths(connection).map_err(|error| error.to_string())?;
    Ok(paths
        .into_iter()
        .filter_map(|path| shift_prefix(&path, folder, destination).map(|next| (path, next)))
        .collect())
}

fn shifted_watches(
    watches: &[String],
    folder: &str,
    destination: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut changes = Vec::new();
    for watch in watches {
        if let Some(next) = shifted_watch(watch, folder, destination)? {
            if next != *watch {
                changes.push((watch.clone(), next));
            }
        }
    }
    Ok(changes)
}

fn shifted_watch(watch: &str, folder: &str, destination: &str) -> Result<Option<String>, String> {
    if let Some(next) = shift_prefix(watch, folder, destination) {
        return Ok(Some(next));
    }
    let Ok(folder_path) = fs::canonicalize(folder) else {
        return Ok(None);
    };
    let Ok(watch_path) = fs::canonicalize(watch) else {
        return Ok(None);
    };
    if watch_path != folder_path && !watch_path.starts_with(&folder_path) {
        return Ok(None);
    }
    let relative = watch_path
        .strip_prefix(&folder_path)
        .map_err(|_| format!("{watch} is outside {folder}"))?;
    path_text(&Path::new(destination).join(relative)).map(Some)
}

fn watches_inside(watches: &[String], folder: &str) -> Vec<String> {
    let folder_path = fs::canonicalize(folder).ok();
    watches
        .iter()
        .filter(|watch| {
            path_inside(folder, watch)
                || folder_path.as_ref().is_some_and(|folder_path| {
                    fs::canonicalize(watch)
                        .map(|watch_path| {
                            watch_path == *folder_path || watch_path.starts_with(folder_path)
                        })
                        .unwrap_or(false)
                })
        })
        .cloned()
        .collect()
}

fn tracks_inside(connection: &Connection, folder: &str) -> Result<Vec<String>, String> {
    let paths = db::list_track_paths(connection).map_err(|error| error.to_string())?;
    Ok(paths
        .into_iter()
        .filter(|path| path_inside(folder, path))
        .collect())
}

fn apply_watch_renames(
    connection: &Connection,
    changes: &[(String, String)],
) -> Result<(), String> {
    let mut applied: Vec<(String, String)> = Vec::new();
    for (from, to) in changes {
        if let Err(error) = db::rename_watch_folder(connection, from, to) {
            for (done_from, done_to) in applied.iter().rev() {
                let _ = db::rename_watch_folder(connection, done_to, done_from);
            }
            let text = error.to_string();
            if text.contains("UNIQUE") || text.contains("constraint") {
                return Err("A watched folder already uses that path".to_string());
            }
            return Err(text);
        }
        applied.push((from.to_string(), to.to_string()));
    }
    Ok(())
}

fn restore_tracks(connection: &Connection, moves: &[(String, String)]) -> Result<(), String> {
    let reverse = moves
        .iter()
        .map(|(from, to)| (to.clone(), from.clone()))
        .collect::<Vec<_>>();
    db::relocate_tracks(connection, &reverse).map_err(|error| error.to_string())
}

fn restore_message(action: &str, error: &str, undo: Result<(), String>) -> String {
    match undo {
        Ok(()) => format!("Could not {action}: {error}"),
        Err(undo) => format!(
            "Renamed the folder but could not {action} ({error}); restoring it also failed ({undo})"
        ),
    }
}

fn shift_prefix(path: &str, folder: &str, destination: &str) -> Option<String> {
    let path = normalize_stored(path);
    let folder = normalize_stored(folder);
    let destination = normalize_stored(destination);
    if path == folder {
        return Some(destination);
    }
    let slash = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    let prefix = format!("{folder}{slash}");
    path.strip_prefix(&prefix)
        .map(|rest| format!("{destination}{slash}{rest}"))
}

fn path_inside(folder: &str, path: &str) -> bool {
    shift_prefix(path, folder, folder).is_some()
}

fn normalize_stored(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn path_is_rootish(path: &Path) -> bool {
    path.components().count() < 3
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not a valid path", path.display()))
}

#[cfg(target_os = "macos")]
fn move_directory_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let text = path_text(path)?;
    let folder = NSString::from_str(&text);
    let url = NSURL::fileURLWithPath_isDirectory(&folder, true);
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| {
            format!(
                "Could not move {} to the Trash: {}",
                path.display(),
                error.localizedDescription()
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn move_directory_to_trash(path: &Path) -> Result<(), String> {
    let _ = path;
    Err("Moving a folder to the Trash is available on macOS".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_library(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn renaming_a_folder_moves_nested_tracks_cues_and_watches() {
        let root = temp_library("rename-folder");
        let house = root.join("House");
        let deep = house.join("Deep");
        fs::create_dir_all(&deep).unwrap();
        let track = deep.join("a.mp3");
        fs::write(&track, b"a").unwrap();
        let outside = root.join("b.mp3");
        fs::write(&outside, b"b").unwrap();
        let database = db::open_in_memory();
        db::add_watch_folder(&database, house.to_str().unwrap()).unwrap();
        db::add_watch_folder(&database, deep.to_str().unwrap()).unwrap();
        db::add_track(&database, track.to_str().unwrap(), "A", "Ar", 1).unwrap();
        db::add_track(&database, outside.to_str().unwrap(), "B", "Br", 1).unwrap();
        db::set_cue(&database, track.to_str().unwrap(), 0, 42.0).unwrap();

        let edit = rename_library_folder(&database, house.to_str().unwrap(), "Techno").unwrap();
        let techno = root.join("Techno");
        let moved = techno.join("Deep").join("a.mp3");
        assert_eq!(edit.to.as_deref(), techno.to_str());
        assert!(moved.is_file());
        assert!(!house.exists());
        assert!(db::track_exists(&database, moved.to_str().unwrap()).unwrap());
        assert!(!db::track_exists(&database, track.to_str().unwrap()).unwrap());
        assert_eq!(
            db::track_state(&database, moved.to_str().unwrap())
                .unwrap()
                .cues[0]
                .frame,
            42.0
        );
        let watches = db::list_watch_folders(&database).unwrap();
        assert!(watches.contains(&text(&techno)));
        assert!(watches.contains(&text(&techno.join("Deep"))));
        assert!(db::track_exists(&database, outside.to_str().unwrap()).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_case_only_rename_keeps_the_new_spelling() {
        let root = temp_library("rename-case");
        let house = root.join("House");
        fs::create_dir_all(&house).unwrap();
        let database = db::open_in_memory();
        db::add_watch_folder(&database, house.to_str().unwrap()).unwrap();
        rename_library_folder(&database, house.to_str().unwrap(), "house").unwrap();
        let names = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["house".to_string()]);
        assert_eq!(
            db::list_watch_folders(&database).unwrap(),
            vec![text(&root.join("house"))]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_slash_or_outside_folder_is_rejected_without_moving_anything() {
        let root = temp_library("rename-reject");
        let house = root.join("House");
        let other = root.join("Other");
        fs::create_dir_all(&house).unwrap();
        fs::create_dir_all(&other).unwrap();
        let database = db::open_in_memory();
        db::add_watch_folder(&database, house.to_str().unwrap()).unwrap();
        let rejected = rename_library_folder(&database, house.to_str().unwrap(), "A/B");
        assert!(rejected.unwrap_err().contains("folder name"));
        assert!(house.is_dir());
        let outside = rename_library_folder(&database, other.to_str().unwrap(), "Elsewhere");
        assert!(outside.unwrap_err().contains("library folders"));
        assert!(other.is_dir());
        let taken = root.join("Taken");
        fs::create_dir_all(&taken).unwrap();
        let collision = rename_library_folder(&database, house.to_str().unwrap(), "Taken");
        assert!(collision.unwrap_err().contains("already exists"));
        assert!(house.is_dir());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_child_folder_drops_its_tracks_and_leaves_the_watched_root() {
        let root = temp_library("delete-child");
        let house = root.join("House");
        let kept = root.join("Kept");
        fs::create_dir_all(&house).unwrap();
        fs::create_dir_all(&kept).unwrap();
        let removed_track = house.join("a.mp3");
        let kept_track = kept.join("b.mp3");
        fs::write(&removed_track, b"a").unwrap();
        fs::write(&kept_track, b"b").unwrap();
        let database = db::open_in_memory();
        db::add_watch_folder(&database, root.to_str().unwrap()).unwrap();
        db::add_watch_folder(&database, house.to_str().unwrap()).unwrap();
        db::add_track(&database, removed_track.to_str().unwrap(), "A", "Ar", 1).unwrap();
        db::add_track(&database, kept_track.to_str().unwrap(), "B", "Br", 1).unwrap();
        let holding = root.join("holding");
        fs::create_dir_all(&holding).unwrap();
        let edit = delete_library_folder_with(&database, house.to_str().unwrap(), |path| {
            fs::rename(path, holding.join("House")).map_err(|error| error.to_string())
        })
        .unwrap();
        assert!(edit.to.is_none());
        assert!(!house.exists());
        assert!(kept_track.is_file());
        assert!(!db::track_exists(&database, removed_track.to_str().unwrap()).unwrap());
        assert!(db::track_exists(&database, kept_track.to_str().unwrap()).unwrap());
        assert_eq!(
            db::list_watch_folders(&database).unwrap(),
            vec![text(&root)]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn deleting_a_watched_folder_moves_it_to_the_trash() {
        let root = temp_library("delete-trash");
        let house = root.join("House");
        fs::create_dir_all(&house).unwrap();
        fs::write(house.join("a.mp3"), b"a").unwrap();
        let database = db::open_in_memory();
        db::add_watch_folder(&database, house.to_str().unwrap()).unwrap();
        db::add_track(
            &database,
            house.join("a.mp3").to_str().unwrap(),
            "A",
            "Ar",
            1,
        )
        .unwrap();
        delete_library_folder(&database, house.to_str().unwrap()).unwrap();
        assert!(!house.exists());
        assert!(db::list_watch_folders(&database).unwrap().is_empty());
        assert!(db::list_track_paths(&database).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
