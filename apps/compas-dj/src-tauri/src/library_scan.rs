//! Startup folder walk. It runs off the audio thread and off the UI thread.
//! Symlinks are not followed, so a link back to a parent cannot loop. Depth is capped.
//! A directory that cannot be listed is skipped; that is also what a failed network
//! listing looks like. A listing that blocks inside the operating system still blocks
//! only this walk.

use std::path::{Path, PathBuf};

pub(crate) const MAX_SCAN_DEPTH: u32 = 16;

pub(crate) fn collect_audio_files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(folder, 0, &mut found);
    found
}

/// `true` only when the path is confirmed absent. Permission errors and other
/// failures keep the library row, because a cue must survive a volume that
/// cannot be stated.
pub(crate) fn file_is_confirmed_missing(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(_) => false,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn walk(folder: &Path, depth: u32, found: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            let reserved_mirror = depth == 0
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(crate::mp3_mirror::MIRROR_DIR));
            if !reserved_mirror {
                walk(&path, depth + 1, found);
            }
        } else if kind.is_file() && crate::is_audio_file(&path) {
            found.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-scan-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn startup_scan_stops_at_a_symlink_loop_and_skips_linked_audio() {
        let root = scratch("loop");
        std::fs::write(root.join("kept.wav"), b"RIFF").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&root, root.join("again")).unwrap();
            std::os::unix::fs::symlink(root.join("kept.wav"), root.join("linked.wav")).unwrap();
        }
        let found = collect_audio_files(&root);
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert_eq!(names, vec!["kept.wav"]);
        assert!(!file_is_confirmed_missing(&root.join("kept.wav")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_scan_stops_at_the_depth_bound() {
        let root = scratch("depth");
        let mut nested = root.clone();
        for _ in 0..=MAX_SCAN_DEPTH + 2 {
            nested.push("down");
        }
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("too-deep.wav"), b"RIFF").unwrap();
        let mut visible = root.clone();
        for _ in 0..MAX_SCAN_DEPTH {
            visible.push("down");
        }
        std::fs::write(visible.join("inside.wav"), b"RIFF").unwrap();
        let found = collect_audio_files(&root);
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert_eq!(names, vec!["inside.wav"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_scan_skips_the_reserved_mp3_mirror_only() {
        let root = scratch("mirror");
        std::fs::write(root.join("kept.flac"), b"fLaC").unwrap();
        let mirror = root.join("mp3");
        std::fs::create_dir(&mirror).unwrap();
        std::fs::write(mirror.join("hidden.wav"), b"RIFF").unwrap();
        let nested = root.join("Artist").join("mp3");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("still.wav"), b"RIFF").unwrap();
        let found = collect_audio_files(&root);
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert!(names.contains(&"kept.flac"));
        assert!(names.contains(&"still.wav"));
        assert!(!names.contains(&"hidden.wav"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_scan_skips_unreadable_directories_and_non_audio_files() {
        let root = scratch("wide");
        for index in 0..300 {
            std::fs::write(root.join(format!("note-{index}.txt")), b"text").unwrap();
        }
        std::fs::write(root.join("song.flac"), b"fLaC").unwrap();
        let blocked = root.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("hidden.mp3"), b"ID3").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&blocked).unwrap().permissions();
            permissions.set_mode(0);
            std::fs::set_permissions(&blocked, permissions).unwrap();
        }
        let found = collect_audio_files(&root);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&blocked).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&blocked, permissions).unwrap();
        }
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        #[cfg(unix)]
        assert_eq!(names, vec!["song.flac"]);
        #[cfg(not(unix))]
        {
            assert!(names.contains(&"song.flac"));
            assert!(names.contains(&"hidden.mp3"));
        }
        assert!(!names.iter().any(|name| name.ends_with(".txt")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
