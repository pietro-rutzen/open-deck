//! Path and manifest rules for a watched folder's `mp3/` mirror.
//! Encoding lives in `mp3_encode`. This module never opens an audio device.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) const MIRROR_DIR: &str = "mp3";
const MANIFEST_VERSION: u32 = 2;
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
    #[serde(default)]
    pub(crate) dest: String,
    #[serde(default)]
    pub(crate) source_dev: u64,
    #[serde(default)]
    pub(crate) source_ino: u64,
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
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) artist: String,
    pub(crate) title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EncodeJob {
    pub(crate) source_rel: String,
    pub(crate) dest_rel: String,
    pub(crate) identity: String,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenameJob {
    pub(crate) previous_key: String,
    pub(crate) identity: String,
    pub(crate) from_rel: String,
    pub(crate) to_rel: String,
    pub(crate) entry: ManifestFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeepJob {
    pub(crate) previous_key: String,
    pub(crate) identity: String,
    pub(crate) entry: ManifestFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteJob {
    pub(crate) identity: String,
    pub(crate) dest_rel: String,
    pub(crate) source_rel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MirrorPlan {
    pub(crate) encode: Vec<EncodeJob>,
    pub(crate) rename: Vec<RenameJob>,
    pub(crate) keep: Vec<KeepJob>,
    pub(crate) skip: Vec<String>,
    pub(crate) delete: Vec<DeleteJob>,
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
        let (dev, ino) = file_identity(&meta);
        stamps.push(SourceStamp {
            rel,
            len: meta.len(),
            mtime_ns,
            dev,
            ino,
            artist: String::new(),
            title: String::new(),
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

fn file_identity(meta: &std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return (meta.dev(), meta.ino());
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        (0, 0)
    }
}

pub(crate) fn identity_key(dev: u64, ino: u64, source: &str) -> String {
    if ino != 0 {
        format!("dev:{dev}:ino:{ino}")
    } else {
        format!("path:{source}")
    }
}

fn normalized_manifest(manifest: &Manifest) -> Manifest {
    if manifest.version >= MANIFEST_VERSION {
        return manifest.clone();
    }
    let mut files = BTreeMap::new();
    for (key, entry) in &manifest.files {
        let mut entry = entry.clone();
        if entry.dest.is_empty() {
            entry.dest = key.clone();
        }
        let id = identity_key(entry.source_dev, entry.source_ino, &entry.source);
        files.insert(id, entry);
    }
    Manifest {
        version: MANIFEST_VERSION,
        files,
    }
}

fn parent_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(index) => &rel[..index],
        None => "",
    }
}

fn file_name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn order_index(file_name: &str) -> Option<u32> {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let digits = stem
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let rest = stem[digits..].trim_start();
    let marker = rest.chars().next()?;
    if !matches!(marker, '.' | '-' | ')') {
        return None;
    }
    let name = rest[marker.len_utf8()..].trim_start();
    if name.is_empty() {
        return None;
    }
    stem[..digits].parse().ok()
}

fn strip_order_prefix(stem: &str) -> &str {
    let digits = stem
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
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

fn clean_label(value: &str) -> String {
    let mut cleaned = String::new();
    let mut spaced = false;
    for character in value.chars() {
        let blocked = character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            );
        if blocked || character.is_whitespace() {
            if !cleaned.is_empty() && !spaced {
                cleaned.push(' ');
                spaced = true;
            }
            continue;
        }
        cleaned.push(character);
        spaced = false;
    }
    cleaned.trim().trim_matches('.').trim().to_string()
}

fn display_label(artist: &str, title: &str, fallback_stem: &str) -> String {
    let artist = clean_label(artist);
    let title = clean_label(title);
    let label = match (artist.is_empty(), title.is_empty()) {
        (false, false) => format!("{artist} - {title}"),
        (true, false) => title,
        (false, true) => artist,
        (true, true) => clean_label(strip_order_prefix(fallback_stem)),
    };
    if label.is_empty() {
        "track".to_string()
    } else {
        label
    }
}

fn limit_file_name(name: &str) -> String {
    let mut limited = name.to_string();
    while limited.len() > 240 {
        limited.pop();
    }
    let trimmed = limited.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "track.mp3".to_string()
    } else {
        trimmed.to_string()
    }
}

fn assigned_dests(sources: &[SourceStamp]) -> BTreeMap<String, String> {
    let mut groups: BTreeMap<String, Vec<&SourceStamp>> = BTreeMap::new();
    for source in sources {
        if mirror_dest_rel(&source.rel).is_none() {
            continue;
        }
        groups
            .entry(parent_of(&source.rel).to_string())
            .or_default()
            .push(source);
    }
    let mut dests = BTreeMap::new();
    for (parent, mut group) in groups {
        let ordered = group
            .iter()
            .all(|source| order_index(file_name_of(&source.rel)).is_some());
        if ordered {
            group.sort_by(|left, right| {
                order_index(file_name_of(&left.rel))
                    .cmp(&order_index(file_name_of(&right.rel)))
                    .then_with(|| left.rel.cmp(&right.rel))
            });
        } else {
            group.sort_by(|left, right| left.rel.cmp(&right.rel));
        }
        let width = group.len().to_string().len();
        let mut taken = BTreeSet::new();
        for (position, source) in group.iter().enumerate() {
            let stem = file_name_of(&source.rel)
                .rsplit_once('.')
                .map_or(file_name_of(&source.rel), |(stem, _)| stem);
            let name = if ordered {
                let mut copy = 1usize;
                loop {
                    let label = display_label(&source.artist, &source.title, stem);
                    let suffix = if copy == 1 {
                        String::new()
                    } else {
                        format!(" ({copy})")
                    };
                    let candidate =
                        limit_file_name(&format!("{:0width$} - {label}{suffix}.mp3", position + 1));
                    if taken.insert(candidate.to_lowercase()) {
                        break candidate;
                    }
                    copy += 1;
                    if copy > 1000 {
                        break limit_file_name(&format!(
                            "{:0width$} - track ({copy}).mp3",
                            position + 1
                        ));
                    }
                }
            } else {
                let candidate = limit_file_name(&format!("{stem}.mp3"));
                if !taken.insert(candidate.to_lowercase()) {
                    continue;
                }
                candidate
            };
            let dest = if parent.is_empty() {
                format!("mp3/{name}")
            } else {
                format!("mp3/{parent}/{name}")
            };
            if safe_mirror_rel(&dest) {
                dests.insert(source.rel.clone(), dest);
            }
        }
    }
    dests
}

pub(crate) fn plan_mirror(
    sources: &[SourceStamp],
    manifest: &Manifest,
    policy: OrphanPolicy,
    dest_exists: impl Fn(&str) -> bool,
) -> MirrorPlan {
    let manifest = normalized_manifest(manifest);
    let dests = assigned_dests(sources);
    let mut by_source: BTreeMap<&str, (&str, &ManifestFile)> = BTreeMap::new();
    for (key, entry) in &manifest.files {
        by_source.insert(entry.source.as_str(), (key.as_str(), entry));
    }
    let mut encode = Vec::new();
    let mut rename = Vec::new();
    let mut keep = Vec::new();
    let mut skip = Vec::new();
    let mut claimed_sources = BTreeSet::new();
    let mut ordered: Vec<&SourceStamp> = sources
        .iter()
        .filter(|source| dests.contains_key(&source.rel))
        .collect();
    ordered.sort_by(|left, right| dests[&left.rel].cmp(&dests[&right.rel]));
    for source in ordered {
        let Some(dest) = dests.get(&source.rel) else {
            continue;
        };
        if !claimed_sources.insert(source.rel.clone()) {
            continue;
        }
        let identity = identity_key(source.dev, source.ino, &source.rel);
        let matched = manifest
            .files
            .get(&identity)
            .map(|entry| (identity.as_str(), entry))
            .or_else(|| {
                if source.ino != 0 {
                    manifest
                        .files
                        .iter()
                        .find(|(_, entry)| {
                            entry.source_dev == source.dev && entry.source_ino == source.ino
                        })
                        .map(|(key, entry)| (key.as_str(), entry))
                } else {
                    None
                }
            })
            .or_else(|| by_source.get(source.rel.as_str()).copied());
        let entry = ManifestFile {
            source: source.rel.clone(),
            source_len: source.len,
            source_mtime_ns: source.mtime_ns,
            dest: dest.clone(),
            source_dev: source.dev,
            source_ino: source.ino,
        };
        match matched {
            Some((key, previous))
                if previous.source_len == source.len
                    && previous.source_mtime_ns == source.mtime_ns
                    && dest_exists(&previous.dest) =>
            {
                if previous.dest == *dest {
                    skip.push(dest.clone());
                    keep.push(KeepJob {
                        previous_key: key.to_string(),
                        identity: identity.clone(),
                        entry,
                    });
                } else {
                    rename.push(RenameJob {
                        previous_key: key.to_string(),
                        identity,
                        from_rel: previous.dest.clone(),
                        to_rel: dest.clone(),
                        entry,
                    });
                }
            }
            _ => encode.push(EncodeJob {
                source_rel: source.rel.clone(),
                dest_rel: dest.clone(),
                identity,
                dev: source.dev,
                ino: source.ino,
            }),
        }
    }
    let delete = match policy {
        OrphanPolicy::KeepAll => Vec::new(),
        OrphanPolicy::DeleteTracked => manifest
            .files
            .iter()
            .filter(|(key, entry)| {
                if !safe_mirror_rel(&entry.dest) && !safe_mirror_rel(key) {
                    return false;
                }
                let source_known = sources.iter().any(|source| source.rel == entry.source);
                let inode_known = entry.source_ino != 0
                    && sources.iter().any(|source| {
                        source.dev == entry.source_dev && source.ino == entry.source_ino
                    });
                !source_known && !inode_known
            })
            .map(|(key, entry)| DeleteJob {
                identity: key.clone(),
                dest_rel: if entry.dest.is_empty() {
                    key.clone()
                } else {
                    entry.dest.clone()
                },
                source_rel: entry.source.clone(),
            })
            .collect(),
    };
    MirrorPlan {
        encode,
        rename,
        keep,
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
        Ok(manifest) if manifest.version == 1 || manifest.version == MANIFEST_VERSION => {
            (normalized_manifest(&manifest), OrphanPolicy::DeleteTracked)
        }
        _ => (Manifest::default(), OrphanPolicy::KeepAll),
    }
}

pub(crate) fn save_manifest(root: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let mut stored = normalized_manifest(manifest);
    stored.version = MANIFEST_VERSION;
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
        named(rel, len, mtime_ns, 0, 0, "", "")
    }

    fn named(
        rel: &str,
        len: u64,
        mtime_ns: u64,
        dev: u64,
        ino: u64,
        artist: &str,
        title: &str,
    ) -> SourceStamp {
        SourceStamp {
            rel: rel.to_string(),
            len,
            mtime_ns,
            dev,
            ino,
            artist: artist.to_string(),
            title: title.to_string(),
        }
    }

    fn tracked(source: &str, len: u64, mtime_ns: u64) -> ManifestFile {
        ManifestFile {
            source: source.to_string(),
            source_len: len,
            source_mtime_ns: mtime_ns,
            dest: String::new(),
            source_dev: 0,
            source_ino: 0,
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
                ("mp3/song.mp3".to_string(), tracked("song.flac", 10, 100)),
                ("mp3/gone.mp3".to_string(), tracked("gone.flac", 3, 3)),
                ("mp3/changed.mp3".to_string(), tracked("new.aiff", 1, 1)),
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
        assert_eq!(plan.delete.len(), 1);
        assert_eq!(plan.delete[0].dest_rel, "mp3/gone.mp3");
        assert!(!plan
            .delete
            .iter()
            .any(|job| job.dest_rel == "mp3/notes.mp3"));

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
                files: BTreeMap::from([("mp3/a.mp3".to_string(), tracked("a.flac", 2, 2))]),
            },
        )
        .unwrap();
        let (loaded, policy) = load_manifest(&root);
        assert_eq!(policy, OrphanPolicy::DeleteTracked);
        let saved = loaded.files.get("path:a.flac").expect("source identity");
        assert_eq!(saved.source, "a.flac");
        assert_eq!(saved.dest, "mp3/a.mp3");

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
    fn ordered_folders_use_padded_artist_title_names_and_plain_folders_do_not() {
        let plain = plan_mirror(
            &[named("Album/song.flac", 4, 4, 1, 2, "Ada", "Song")],
            &Manifest::default(),
            OrphanPolicy::KeepAll,
            |_| false,
        );
        assert_eq!(plain.encode[0].dest_rel, "mp3/Album/song.mp3");

        let mut many = Vec::new();
        for index in 1..=100 {
            many.push(named(
                &format!("Album/{index:03} - take.flac"),
                4,
                4,
                1,
                index,
                "Ada",
                "Song",
            ));
        }
        let planned = plan_mirror(&many, &Manifest::default(), OrphanPolicy::KeepAll, |_| {
            false
        });
        assert_eq!(planned.encode.len(), 100);
        assert_eq!(planned.encode[0].dest_rel, "mp3/Album/001 - Ada - Song.mp3");
        assert_eq!(
            planned.encode[99].dest_rel,
            "mp3/Album/100 - Ada - Song.mp3"
        );
        assert!(planned
            .encode
            .windows(2)
            .all(|pair| pair[0].dest_rel <= pair[1].dest_rel));
    }

    #[test]
    fn reorder_renames_by_source_identity_and_leaves_orphans_deletable() {
        let sources = vec![
            named("01 - a.flac", 10, 100, 7, 8, "Ada", "First"),
            named("02 - b.flac", 10, 100, 7, 9, "Ada", "Second"),
        ];
        let manifest = Manifest {
            version: 2,
            files: BTreeMap::from([
                (
                    identity_key(7, 8, "a.flac"),
                    ManifestFile {
                        source: "a.flac".to_string(),
                        source_len: 10,
                        source_mtime_ns: 100,
                        dest: "mp3/a.mp3".to_string(),
                        source_dev: 7,
                        source_ino: 8,
                    },
                ),
                (
                    identity_key(7, 9, "b.flac"),
                    ManifestFile {
                        source: "b.flac".to_string(),
                        source_len: 10,
                        source_mtime_ns: 100,
                        dest: "mp3/b.mp3".to_string(),
                        source_dev: 7,
                        source_ino: 9,
                    },
                ),
                (
                    identity_key(7, 4, "gone.flac"),
                    ManifestFile {
                        source: "gone.flac".to_string(),
                        source_len: 1,
                        source_mtime_ns: 1,
                        dest: "mp3/gone.mp3".to_string(),
                        source_dev: 7,
                        source_ino: 4,
                    },
                ),
            ]),
        };
        let plan = plan_mirror(&sources, &manifest, OrphanPolicy::DeleteTracked, |dest| {
            dest == "mp3/a.mp3" || dest == "mp3/b.mp3" || dest == "mp3/gone.mp3"
        });
        assert!(plan.encode.is_empty());
        assert_eq!(plan.rename.len(), 2);
        assert!(plan.rename.iter().any(|job| {
            job.from_rel == "mp3/a.mp3"
                && job.to_rel == "mp3/1 - Ada - First.mp3"
                && job.entry.source == "01 - a.flac"
        }));
        assert!(plan
            .rename
            .iter()
            .any(|job| job.to_rel == "mp3/2 - Ada - Second.mp3"));
        assert_eq!(plan.delete.len(), 1);
        assert_eq!(plan.delete[0].dest_rel, "mp3/gone.mp3");
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
