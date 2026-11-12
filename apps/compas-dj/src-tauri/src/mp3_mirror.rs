//! Path and manifest rules for a watched folder's `mp3/` mirror.
//! Encoding lives in `mp3_encode`. This module never opens an audio device.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) const MIRROR_DIR: &str = "mp3";
const MANIFEST_VERSION: u32 = 1;
const MANIFEST_FILE: &str = ".opendeck-mp3-manifest.json";
/// One-minute load average divided by the CPU count. Above this, the worker
/// yields between files. At or below it, a file may start.
pub(crate) const CPU_YIELD_LOAD_PER_CPU: f64 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrphanPolicy {
    /// The manifest was read and lists every file this app created.
    DeleteTracked,
    /// No usable manifest. Do not delete anything.
    KeepAll,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ManifestFile {
    pub(crate) source: String,
    pub(crate) source_len: u64,
    pub(crate) source_mtime_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Manifest {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) files: BTreeMap<String, ManifestFile>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 0,
            files: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceStamp {
    pub(crate) rel: String,
    pub(crate) len: u64,
    pub(crate) mtime_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EncodeJob {
    pub(crate) source_rel: String,
    pub(crate) dest_rel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MirrorPlan {
    pub(crate) encode: Vec<EncodeJob>,
    pub(crate) skip: Vec<String>,
    pub(crate) delete: Vec<String>,
}

pub(crate) fn machine_is_busy(load_per_cpu: f64) -> bool {
    load_per_cpu.is_finite() && load_per_cpu > CPU_YIELD_LOAD_PER_CPU
}

pub(crate) fn playback_should_pause(any_deck_playing: bool) -> bool {
    any_deck_playing
}

/// `mp3/Artist/Album/song.mp3` for `Artist/Album/song.flac`.
/// `None` for an existing MP3, a path inside the reserved mirror, or a non-audio file.
pub(crate) fn mirror_dest_rel(source_rel: &str) -> Option<String> {
    if source_rel.is_empty() || source_rel.contains('\\') {
        return None;
    }
    let mut parts = Vec::new();
    for component in Path::new(source_rel).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str()?;
                parts.push(part);
            }
            _ => return None,
        }
    }
    let file_name = *parts.first()?;
    if file_name.eq_ignore_ascii_case(MIRROR_DIR) {
        return None;
    }
    let file_name = *parts.last()?;
    let (stem, ext) = file_name.rsplit_once('.')?;
    if stem.is_empty() || !is_convertible_extension(ext) {
        return None;
    }
    let mut dest = String::from(MIRROR_DIR);
    for part in &parts[..parts.len() - 1] {
        dest.push('/');
        dest.push_str(part);
    }
    dest.push('/');
    dest.push_str(stem);
    dest.push_str(".mp3");
    Some(dest)
}

fn is_convertible_extension(ext: &str) -> bool {
    if ext.eq_ignore_ascii_case("mp3") {
        return false;
    }
    crate::is_audio_file(Path::new(&format!("track.{ext}")))
}

pub(crate) fn safe_source_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(crate) fn safe_mirror_rel(rel: &str) -> bool {
    rel.starts_with("mp3/")
        && !rel.ends_with('/')
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(crate) fn is_under_watch_mirror(roots: &[String], path: &str) -> bool {
    let file = Path::new(path);
    roots.iter().any(|root| {
        let root = root.trim_end_matches(['/', '\\']);
        if root.is_empty() {
            return false;
        }
        let Ok(rel) = file.strip_prefix(root) else {
            return false;
        };
        rel.components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(MIRROR_DIR))
    })
}

pub(crate) fn source_stamps(root: &Path) -> Vec<SourceStamp> {
    let mut stamps = Vec::new();
    for path in crate::library_scan::collect_audio_files(root) {
        let Some(rel) = path.strip_prefix(root).ok().and_then(relative_string) else {
            continue;
        };
        if mirror_dest_rel(&rel).is_none() {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let Some(mtime_ns) = meta.modified().ok().and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_nanos()).ok())
        }) else {
            continue;
        };
        stamps.push(SourceStamp {
            rel,
            len: meta.len(),
            mtime_ns,
        });
    }
    stamps.sort_by(|left, right| left.rel.cmp(&right.rel));
    stamps
}

fn relative_string(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

pub(crate) fn plan_mirror(
    sources: &[SourceStamp],
    manifest: &Manifest,
    policy: OrphanPolicy,
    dest_exists: impl Fn(&str) -> bool,
) -> MirrorPlan {
    let mut sources = sources.to_vec();
    sources.sort_by(|left, right| left.rel.cmp(&right.rel));
    let mut encode = Vec::new();
    let mut skip = Vec::new();
    let mut claimed = BTreeSet::new();
    for source in sources.iter() {
        let Some(dest) = mirror_dest_rel(&source.rel) else {
            continue;
        };
        if !safe_mirror_rel(&dest) || !claimed.insert(dest.clone()) {
            continue;
        }
        let unchanged = manifest.files.get(&dest).is_some_and(|entry| {
            entry.source == source.rel
                && entry.source_len == source.len
                && entry.source_mtime_ns == source.mtime_ns
                && dest_exists(&dest)
        });
        if unchanged {
            skip.push(dest);
        } else {
            encode.push(EncodeJob {
                source_rel: source.rel.clone(),
                dest_rel: dest,
            });
        }
    }
    let delete = match policy {
        OrphanPolicy::KeepAll => Vec::new(),
        OrphanPolicy::DeleteTracked => manifest
            .files
            .iter()
            .filter(|(dest, entry)| {
                safe_mirror_rel(dest) && !sources.iter().any(|source| source.rel == entry.source)
            })
            .map(|(dest, _)| dest.clone())
            .collect(),
    };
    MirrorPlan {
        encode,
        skip,
        delete,
    }
}

pub(crate) fn manifest_path(root: &Path) -> PathBuf {
    root.join(MIRROR_DIR).join(MANIFEST_FILE)
}

pub(crate) fn load_manifest(root: &Path) -> (Manifest, OrphanPolicy) {
    let bytes = match std::fs::read(manifest_path(root)) {
        Ok(bytes) => bytes,
        Err(_) => return (Manifest::default(), OrphanPolicy::KeepAll),
    };
    match serde_json::from_slice::<Manifest>(&bytes) {
        Ok(manifest) if manifest.version == MANIFEST_VERSION => {
            (manifest, OrphanPolicy::DeleteTracked)
        }
        _ => (Manifest::default(), OrphanPolicy::KeepAll),
    }
}

pub(crate) fn save_manifest(root: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let stored = Manifest {
        version: MANIFEST_VERSION,
        files: manifest.files.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_atomic(&manifest_path(root), &bytes)
}

pub(crate) fn partial_path(dest: &Path) -> Option<PathBuf> {
    let name = dest.file_name()?.to_str()?;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(dest.with_file_name(format!("{name}.partial")))
}

/// Write `bytes` to a sibling temp file, sync it, then rename it onto `dest`.
pub(crate) fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_replace(dest, |tmp| {
        let mut file = File::create(tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })
}

pub(crate) fn atomic_replace(
    dest: &Path,
    write_temp: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let tmp = partial_path(dest).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the mirror path has no file name",
        )
    })?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Err(error) = write_temp(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    std::fs::rename(&tmp, dest)?;
    if let Some(parent) = dest.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-mp3-mirror-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn stamp(rel: &str, len: u64, mtime_ns: u64) -> SourceStamp {
        SourceStamp {
            rel: rel.to_string(),
            len,
            mtime_ns,
        }
    }

    #[test]
    fn mirror_paths_keep_the_tree_and_skip_mp3_sources() {
        assert_eq!(
            mirror_dest_rel("Artist/Album/song.flac").as_deref(),
            Some("mp3/Artist/Album/song.mp3")
        );
        assert_eq!(
            mirror_dest_rel("nested/mp3/song.aiff").as_deref(),
            Some("mp3/nested/mp3/song.mp3")
        );
        assert_eq!(mirror_dest_rel("take.wav").as_deref(), Some("mp3/take.mp3"));
        assert_eq!(mirror_dest_rel("take.aif").as_deref(), Some("mp3/take.mp3"));
        assert_eq!(mirror_dest_rel("take.m4a").as_deref(), Some("mp3/take.mp3"));
        assert_eq!(mirror_dest_rel("song.mp3"), None);
        assert_eq!(mirror_dest_rel("song.MP3"), None);
        assert_eq!(mirror_dest_rel("mp3/song.flac"), None);
        assert_eq!(mirror_dest_rel("MP3/song.wav"), None);
        assert_eq!(mirror_dest_rel("../song.flac"), None);
        assert_eq!(mirror_dest_rel("/tmp/song.flac"), None);
        assert!(!safe_mirror_rel("mp3/../../song.mp3"));
        assert!(!safe_mirror_rel("song.mp3"));
        assert!(safe_mirror_rel("mp3/Artist/song.mp3"));
    }

    #[test]
    fn library_hides_only_the_reserved_mirror() {
        let roots = vec!["/music/house".to_string()];
        assert!(is_under_watch_mirror(
            &roots,
            "/music/house/mp3/Artist/song.mp3"
        ));
        assert!(is_under_watch_mirror(&roots, "/music/house/MP3/song.mp3"));
        assert!(!is_under_watch_mirror(
            &roots,
            "/music/house/Artist/song.flac"
        ));
        assert!(!is_under_watch_mirror(&roots, "/music/other/mp3/song.mp3"));
        assert!(!is_under_watch_mirror(
            &["/music/house/mp3".to_string()],
            "/music/house/mp3/song.flac"
        ));
    }

    #[test]
    fn plan_skips_unchanged_reencodes_and_deletes_only_tracked_orphans() {
        let sources = vec![
            stamp("song.flac", 10, 100),
            stamp("song.wav", 11, 100),
            stamp("new.aiff", 4, 7),
            stamp("already.mp3", 9, 9),
        ];
        let manifest = Manifest {
            version: 1,
            files: BTreeMap::from([
                (
                    "mp3/song.mp3".to_string(),
                    ManifestFile {
                        source: "song.flac".to_string(),
                        source_len: 10,
                        source_mtime_ns: 100,
                    },
                ),
                (
                    "mp3/gone.mp3".to_string(),
                    ManifestFile {
                        source: "gone.flac".to_string(),
                        source_len: 3,
                        source_mtime_ns: 3,
                    },
                ),
                (
                    "mp3/changed.mp3".to_string(),
                    ManifestFile {
                        source: "new.aiff".to_string(),
                        source_len: 1,
                        source_mtime_ns: 1,
                    },
                ),
            ]),
        };
        let present = ["mp3/song.mp3", "mp3/gone.mp3", "mp3/notes.mp3"];
        let plan = plan_mirror(&sources, &manifest, OrphanPolicy::DeleteTracked, |dest| {
            present.contains(&dest)
        });
        assert_eq!(
            plan.skip,
            vec!["mp3/song.mp3".to_string()],
            "the flac owns the shared name and is unchanged"
        );
        assert!(plan.encode.iter().all(|job| job.source_rel != "song.wav"));
        assert!(plan
            .encode
            .iter()
            .all(|job| job.source_rel != "already.mp3"));
        assert!(plan
            .encode
            .iter()
            .any(|job| job.source_rel == "new.aiff" && job.dest_rel == "mp3/new.mp3"));
        assert_eq!(plan.delete, vec!["mp3/gone.mp3".to_string()]);
        assert!(!plan.delete.iter().any(|dest| dest == "mp3/notes.mp3"));

        let missing_dest = plan_mirror(&sources, &manifest, OrphanPolicy::DeleteTracked, |_| false);
        assert!(missing_dest
            .encode
            .iter()
            .any(|job| job.dest_rel == "mp3/song.mp3"));

        let changed = vec![stamp("song.flac", 10, 101)];
        let again = plan_mirror(&changed, &manifest, OrphanPolicy::DeleteTracked, |dest| {
            dest == "mp3/song.mp3"
        });
        assert_eq!(again.encode.len(), 1);
        assert!(again.skip.is_empty());
    }

    #[test]
    fn missing_or_corrupt_manifest_does_not_delete() {
        let root = scratch("manifest");
        let (manifest, policy) = load_manifest(&root);
        assert!(manifest.files.is_empty());
        assert_eq!(policy, OrphanPolicy::KeepAll);
        let sources = vec![stamp("gone.flac", 1, 1)];
        let plan = plan_mirror(&[], &manifest, policy, |_| true);
        assert!(plan.delete.is_empty());
        let _ = sources;

        save_manifest(
            &root,
            &Manifest {
                version: 0,
                files: BTreeMap::from([(
                    "mp3/a.mp3".to_string(),
                    ManifestFile {
                        source: "a.flac".to_string(),
                        source_len: 2,
                        source_mtime_ns: 2,
                    },
                )]),
            },
        )
        .unwrap();
        let (loaded, policy) = load_manifest(&root);
        assert_eq!(policy, OrphanPolicy::DeleteTracked);
        assert_eq!(loaded.files["mp3/a.mp3"].source, "a.flac");

        std::fs::write(manifest_path(&root), b"{").unwrap();
        let (loaded, policy) = load_manifest(&root);
        assert!(loaded.files.is_empty());
        assert_eq!(policy, OrphanPolicy::KeepAll);
        let plan = plan_mirror(&[], &loaded, policy, |_| true);
        assert!(plan.delete.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn source_stamps_ignore_mp3_sources_and_the_mirror_tree() {
        let root = scratch("stamps");
        std::fs::create_dir(root.join("Artist")).unwrap();
        std::fs::write(root.join("Artist").join("a.flac"), b"fLaC").unwrap();
        std::fs::write(root.join("already.mp3"), b"ID3").unwrap();
        std::fs::create_dir(root.join("mp3")).unwrap();
        std::fs::write(root.join("mp3").join("hidden.wav"), b"RIFF").unwrap();
        let stamps = source_stamps(&root);
        assert_eq!(stamps.len(), 1);
        assert_eq!(stamps[0].rel, "Artist/a.flac");
        assert_eq!(stamps[0].len, 4);
        assert!(stamps[0].mtime_ns > 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn atomic_write_replaces_the_destination_and_a_failure_keeps_it() {
        let root = scratch("atomic");
        let dest = root.join("mp3").join("song.mp3");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"old").unwrap();
        let error = atomic_replace(&dest, |_| {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "full"))
        });
        assert!(error.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert!(!partial_path(&dest).unwrap().exists());

        write_atomic(&dest, b"new-bytes").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-bytes");
        assert!(!partial_path(&dest).unwrap().exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mirror_pauses_for_playback_and_yields_when_the_machine_is_busy() {
        assert!(playback_should_pause(true));
        assert!(!playback_should_pause(false));
        assert!(!machine_is_busy(0.2));
        assert!(!machine_is_busy(CPU_YIELD_LOAD_PER_CPU));
        assert!(machine_is_busy(CPU_YIELD_LOAD_PER_CPU + 0.01));
        assert!(!machine_is_busy(f64::NAN));
    }
}
