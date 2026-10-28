//! Move audio files dropped from outside the app into the folder that is open.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::is_audio_file;

const MAX_DROPPED_FILES: usize = 200;
const MAX_WALK_DEPTH: u32 = 6;

#[derive(Debug)]
pub struct PlannedMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[cfg(test)]
#[derive(Debug)]
pub struct FolderDrop {
    pub moves: Vec<(String, String)>,
    pub failures: Vec<String>,
}

/// Choose a destination name for every dropped audio file. Nothing is renamed yet.
pub fn plan_audio_moves(folder: &Path, sources: &[String]) -> Result<Vec<PlannedMove>, String> {
    let destination = fs::canonicalize(folder)
        .map_err(|error| format!("Open a folder before dropping files into it ({error})"))?;
    if !destination.is_dir() {
        return Err("Open a folder before dropping files into it".to_string());
    }
    let mut audio = Vec::new();
    for source in sources {
        collect_audio(Path::new(source), &destination, &mut audio, 0)?;
    }
    if audio.is_empty() {
        return Err("Drop audio files to move them into this folder".to_string());
    }
    let mut reserved = HashSet::new();
    let mut planned = Vec::new();
    for source in audio {
        if let Some(target) = destination_for(&source, &destination, &reserved)? {
            reserved.insert(target.clone());
            planned.push(PlannedMove {
                from: source,
                to: target,
            });
        }
    }
    Ok(planned)
}

pub fn commit_move(from: &Path, to: &Path) -> Result<(), String> {
    move_file(from, to)
}

pub const REMOVED_FROM_PLAYLISTS: &str = "removed from playlists";

pub fn removed_playlists_dir(music_dir: &Path) -> PathBuf {
    music_dir.join(REMOVED_FROM_PLAYLISTS)
}

/// Choose where `source` lands inside the user's Music folder. Creates that folder when it is missing.
pub fn plan_remove_from_folder(source: &Path, music_dir: &Path) -> Result<PlannedMove, String> {
    if !source.is_file() {
        return Err(format!("{} is no longer on disk", source.display()));
    }
    let destination = removed_playlists_dir(music_dir);
    if parent_is_inside(source, &destination) {
        return Err("This track is already in removed from playlists".to_string());
    }
    fs::create_dir_all(&destination).map_err(|error| {
        format!(
            "Could not create {}: {error}",
            destination.display()
        )
    })?;
    let planned = plan_audio_moves(&destination, &[path_text(source)?])?;
    planned.into_iter().next().ok_or_else(|| {
        "This track is already in removed from playlists".to_string()
    })
}

fn parent_is_inside(source: &Path, destination: &Path) -> bool {
    let Some(parent) = source.parent() else {
        return false;
    };
    if !destination.exists() {
        return false;
    }
    match (fs::canonicalize(parent), fs::canonicalize(destination)) {
        (Ok(parent), Ok(destination)) => parent.starts_with(destination),
        _ => false,
    }
}

/// Move dropped audio into `folder`. A dropped directory contributes the audio
/// files inside it. A name that is already taken gets a numeric suffix.
#[cfg(test)]
pub fn move_audio_into(folder: &Path, sources: &[String]) -> Result<FolderDrop, String> {
    let planned = plan_audio_moves(folder, sources)?;
    let mut moves = Vec::new();
    let mut failures = Vec::new();
    for item in planned {
        match commit_move(&item.from, &item.to) {
            Ok(()) => match (path_text(&item.from), path_text(&item.to)) {
                (Ok(from), Ok(to)) => moves.push((from, to)),
                (Err(error), _) | (_, Err(error)) => failures.push(error),
            },
            Err(error) => failures.push(error),
        }
    }
    if moves.is_empty() && !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    Ok(FolderDrop { moves, failures })
}

fn collect_audio(
    path: &Path,
    destination: &Path,
    audio: &mut Vec<PathBuf>,
    depth: u32,
) -> Result<(), String> {
    if audio.len() >= MAX_DROPPED_FILES {
        return Err(format!(
            "Drop at most {MAX_DROPPED_FILES} audio files at once"
        ));
    }
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }
    if path.is_file() {
        if is_audio_file(path) {
            audio.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !path.is_dir() || same_place(path, destination) {
        return Ok(());
    }
    let entries = fs::read_dir(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let hidden = name.to_str().is_some_and(|name| name.starts_with('.'));
        if hidden {
            continue;
        }
        collect_audio(&entry.path(), destination, audio, depth + 1)?;
    }
    Ok(())
}

fn destination_for(
    source: &Path,
    destination: &Path,
    reserved: &HashSet<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let from = path_text(source)?;
    let parent = source
        .parent()
        .ok_or_else(|| format!("{from} has no folder"))?;
    if same_place(parent, destination) {
        return Ok(None);
    }
    let file_name = source
        .file_name()
        .ok_or_else(|| format!("{from} has no file name"))?;
    Ok(Some(available_name(destination, file_name, reserved)?))
}

fn available_name(
    folder: &Path,
    file_name: &std::ffi::OsStr,
    reserved: &HashSet<PathBuf>,
) -> Result<PathBuf, String> {
    let direct = folder.join(file_name);
    if !taken(&direct, reserved) {
        return Ok(direct);
    }
    let parsed = Path::new(file_name);
    let stem = parsed
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} needs a text file name", direct.display()))?;
    let extension = parsed.extension().and_then(|extension| extension.to_str());
    for copy in 2..1000 {
        let name = match extension {
            Some(extension) => format!("{stem} {copy}.{extension}"),
            None => format!("{stem} {copy}"),
        };
        let candidate = folder.join(name);
        if !taken(&candidate, reserved) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not find a free name for {}",
        direct.display()
    ))
}

fn taken(path: &Path, reserved: &HashSet<PathBuf>) -> bool {
    path.exists() || reserved.contains(path)
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            fs::copy(from, to).map_err(|error| {
                format!(
                    "Could not copy {} into this folder: {error}",
                    from.display()
                )
            })?;
            fs::remove_file(from).map_err(|error| {
                format!(
                    "Copied {} but could not remove the original: {error}",
                    from.display()
                )
            })?;
            Ok(())
        }
        Err(error) => Err(format!(
            "Could not move {} into this folder: {error}",
            from.display()
        )),
    }
}

fn same_place(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not a valid path", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "opendeck-drop-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        fs::create_dir_all(root.join("open")).unwrap();
        root
    }

    #[test]
    fn an_audio_file_moves_into_the_open_folder() {
        let root = scratch("move");
        let source = root.join("elsewhere").join("Song.mp3");
        fs::write(&source, b"audio").unwrap();
        let dropped =
            move_audio_into(&root.join("open"), &[source.to_str().unwrap().to_string()]).unwrap();
        assert_eq!(dropped.moves.len(), 1);
        assert!(!source.exists());
        assert!(root.join("open").join("Song.mp3").is_file());
        assert!(dropped.failures.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_taken_name_keeps_the_file_already_in_the_folder() {
        let root = scratch("clash");
        fs::write(root.join("open").join("Song.mp3"), b"kept").unwrap();
        let source = root.join("elsewhere").join("Song.mp3");
        fs::write(&source, b"incoming").unwrap();
        let dropped =
            move_audio_into(&root.join("open"), &[source.to_str().unwrap().to_string()]).unwrap();
        assert_eq!(
            fs::read(root.join("open").join("Song.mp3")).unwrap(),
            b"kept"
        );
        assert_eq!(
            fs::read(root.join("open").join("Song 2.mp3")).unwrap(),
            b"incoming"
        );
        assert_eq!(dropped.moves.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn two_dropped_files_with_the_same_name_both_land() {
        let root = scratch("pair");
        let first_dir = root.join("elsewhere").join("a");
        let second_dir = root.join("elsewhere").join("b");
        fs::create_dir_all(&first_dir).unwrap();
        fs::create_dir_all(&second_dir).unwrap();
        fs::write(first_dir.join("Song.mp3"), b"one").unwrap();
        fs::write(second_dir.join("Song.mp3"), b"two").unwrap();
        let dropped = move_audio_into(
            &root.join("open"),
            &[
                first_dir.join("Song.mp3").to_str().unwrap().to_string(),
                second_dir.join("Song.mp3").to_str().unwrap().to_string(),
            ],
        )
        .unwrap();
        let open = root.join("open");
        let landed = [
            fs::read(open.join("Song.mp3")).unwrap(),
            fs::read(open.join("Song 2.mp3")).unwrap(),
        ];
        assert!(landed.contains(&b"one".to_vec()));
        assert!(landed.contains(&b"two".to_vec()));
        assert_eq!(dropped.moves.len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_from_folder_lands_in_removed_from_playlists() {
        let root = scratch("remove-folder");
        let music = root.join("Music");
        let playlist = music.join("Friday");
        fs::create_dir_all(&playlist).unwrap();
        let source = playlist.join("Song.mp3");
        fs::write(&source, b"audio").unwrap();
        let planned = plan_remove_from_folder(&source, &music).unwrap();
        commit_move(&planned.from, &planned.to).unwrap();
        let landed = removed_playlists_dir(&music).join("Song.mp3");
        assert_eq!(
            fs::canonicalize(&planned.to).unwrap(),
            fs::canonicalize(&landed).unwrap()
        );
        assert!(landed.is_file());
        assert!(!source.exists());
        let again = plan_remove_from_folder(&landed, &music).unwrap_err();
        assert!(again.contains("already in removed from playlists"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_from_folder_keeps_a_file_already_in_the_holding_folder() {
        let root = scratch("remove-clash");
        let music = root.join("Music");
        let holding = removed_playlists_dir(&music);
        fs::create_dir_all(&holding).unwrap();
        fs::write(holding.join("Song.mp3"), b"kept").unwrap();
        let source = music.join("Friday").join("Song.mp3");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"incoming").unwrap();
        let planned = plan_remove_from_folder(&source, &music).unwrap();
        commit_move(&planned.from, &planned.to).unwrap();
        assert_eq!(fs::read(holding.join("Song.mp3")).unwrap(), b"kept");
        assert_eq!(fs::read(holding.join("Song 2.mp3")).unwrap(), b"incoming");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_already_in_the_open_folder_stays_put() {
        let root = scratch("same");
        let source = root.join("open").join("Song.mp3");
        fs::write(&source, b"audio").unwrap();
        let dropped =
            move_audio_into(&root.join("open"), &[source.to_str().unwrap().to_string()]).unwrap();
        assert!(dropped.moves.is_empty());
        assert!(source.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audio_inside_a_dropped_directory_moves_into_the_open_folder() {
        let root = scratch("dir");
        let nested = root.join("elsewhere").join("album");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("Song.flac"), b"audio").unwrap();
        fs::write(nested.join("notes.txt"), b"skip").unwrap();
        let dropped =
            move_audio_into(&root.join("open"), &[nested.to_str().unwrap().to_string()]).unwrap();
        assert!(root.join("open").join("Song.flac").is_file());
        assert!(nested.join("notes.txt").is_file());
        assert_eq!(dropped.moves.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_drop_without_audio_is_refused() {
        let root = scratch("text");
        let source = root.join("elsewhere").join("notes.txt");
        fs::write(&source, b"text").unwrap();
        let error = move_audio_into(&root.join("open"), &[source.to_str().unwrap().to_string()])
            .unwrap_err();
        assert!(error.contains("audio"));
        assert!(source.is_file());
        let _ = fs::remove_dir_all(root);
    }
}
