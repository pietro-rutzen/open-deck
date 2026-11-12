//! Path and manifest rules for a watched folder's `mp3/` mirror.
//! Encoding lives in `mp3_encode`. This module never opens an audio device.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) const MIRROR_DIR: &str = "mp3";

/// Frames a decoder plays before the first source sample of an MP3 that OpenDeck encoded:
/// LAME's 576-frame encoder delay plus the 529-frame Layer III decoder delay. The mirror has
/// no LAME info frame, so no decoder trims it. `mp3_encode` measures this in a test.
pub(crate) const ENCODED_COPY_DELAY_FRAMES: u32 = 1105;

/// Seconds an encoded copy plays behind its source. A source that is already an MP3 is
/// copied byte for byte, so its copy has no shift.
pub(crate) fn encoded_copy_delay_sec(source: &Path) -> f64 {
    let copied_bytes = source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
    if copied_bytes {
        0.0
    } else {
        f64::from(ENCODED_COPY_DELAY_FRAMES) / 44_100.0
    }
}
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
    /// Existing MP3 sources. The bytes are cloned or copied, never re-encoded.
    pub(crate) copy: Vec<EncodeJob>,
    pub(crate) rename: Vec<RenameJob>,
    pub(crate) keep: Vec<KeepJob>,
    pub(crate) skip: Vec<String>,
    pub(crate) delete: Vec<DeleteJob>,
}

pub(crate) fn machine_is_busy(load_per_cpu: f64) -> bool {
    load_per_cpu.is_finite() && load_per_cpu > CPU_YIELD_LOAD_PER_CPU
}

/// True only while a deck's play-head is moving. A loaded deck that is paused reports rate 0.
pub(crate) fn playback_should_pause(playhead_rates: impl IntoIterator<Item = f64>) -> bool {
    playhead_rates
        .into_iter()
        .any(|rate| rate.is_finite() && rate.abs() > f64::EPSILON)
}

/// `mp3/Artist/Album/song.mp3` for `Artist/Album/song.flac` and for an existing `song.mp3`.
/// `None` for a path inside the reserved mirror, or for a non-audio file.
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
    if stem.is_empty() || !is_mirror_source_extension(ext) {
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

pub(crate) fn source_is_mp3(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("mp3"))
}

fn is_mirror_source_extension(ext: &str) -> bool {
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
                let mut copy = 1usize;
                let candidate = loop {
                    let suffix = if copy == 1 {
                        String::new()
                    } else {
                        format!(" ({copy})")
                    };
                    let candidate = limit_file_name(&format!("{stem}{suffix}.mp3"));
                    if taken.insert(candidate.to_lowercase()) {
                        break candidate;
                    }
                    copy += 1;
                    if copy > 1000 {
                        break limit_file_name(&format!("{stem} ({copy}).mp3"));
                    }
                };
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
    let mut copy = Vec::new();
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
            _ => {
                let job = EncodeJob {
                    source_rel: source.rel.clone(),
                    dest_rel: dest.clone(),
                    identity,
                    dev: source.dev,
                    ino: source.ino,
                };
                if source_is_mp3(&source.rel) {
                    copy.push(job);
                } else {
                    encode.push(job);
                }
            }
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
        copy,
        rename,
        keep,
        skip,
        delete,
    }
}

/// Mirrored files over every audio source in the folder. An existing MP3 is part
/// of both numbers once its copy is in place. `already_mp3` is how many sources
/// were already MP3, which is included in `convertible` rather than a remainder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConversionStatus {
    pub(crate) converted: u32,
    pub(crate) convertible: u32,
    pub(crate) already_mp3: u32,
}

pub(crate) fn conversion_status(root: &Path, scope_prefix: &str) -> ConversionStatus {
    let prefix = scope_prefix.trim_matches('/');
    let in_scope =
        |rel: &str| prefix.is_empty() || rel == prefix || rel.starts_with(&format!("{prefix}/"));
    let sources: Vec<SourceStamp> = source_stamps(root)
        .into_iter()
        .filter(|source| in_scope(&source.rel))
        .collect();
    let (manifest, policy) = load_manifest(root);
    let plan = plan_mirror(&sources, &manifest, policy, |dest| {
        root.join(dest).is_file()
    });
    let done = plan.keep.len().saturating_add(plan.rename.len());
    let pending = plan.encode.len().saturating_add(plan.copy.len());
    let converted = u32::try_from(done).unwrap_or(u32::MAX);
    let convertible = converted.saturating_add(u32::try_from(pending).unwrap_or(u32::MAX));
    ConversionStatus {
        converted,
        convertible,
        already_mp3: count_mp3_sources(root, prefix),
    }
}

pub(crate) fn conversion_status_for_open(roots: &[String], open: &Path) -> ConversionStatus {
    let Some(root) = single_watch_root(roots, open) else {
        return ConversionStatus::default();
    };
    let open = trim_path(open);
    let mirror = root.join(MIRROR_DIR);
    let scope = if open == root || open == mirror {
        String::new()
    } else if let Ok(relative) = open.strip_prefix(&mirror) {
        relative_string(relative).unwrap_or_default()
    } else if let Ok(relative) = open.strip_prefix(&root) {
        relative_string(relative).unwrap_or_default()
    } else {
        String::new()
    };
    conversion_status(&root, &scope)
}

fn count_mp3_sources(root: &Path, prefix: &str) -> u32 {
    let in_scope =
        |rel: &str| prefix.is_empty() || rel == prefix || rel.starts_with(&format!("{prefix}/"));
    let mut count = 0u32;
    for path in crate::library_scan::collect_audio_files(root) {
        let Some(rel) = path.strip_prefix(root).ok().and_then(relative_string) else {
            continue;
        };
        if !in_scope(&rel) {
            continue;
        }
        let mp3 = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"));
        if mp3 {
            count = count.saturating_add(1);
        }
    }
    count
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

/// Place an existing MP3 into the mirror without re-encoding it.
///
/// On the same APFS volume this is a `clonefile` clone: a separate file that
/// shares extents until either side is written. Reorder can rename that file,
/// and replacing it cannot rewrite the source. A USB volume cannot hold the
/// clone, so the bytes are copied instead. A hard link is not used: it is the
/// same inode, and a later write or delete of the mirror would change the source.
pub(crate) fn clone_or_copy(source: &Path, dest: &Path) -> std::io::Result<()> {
    atomic_replace(dest, |tmp| {
        if clone_same_volume(source, tmp).is_ok() {
            return Ok(());
        }
        let mut input = File::open(source)?;
        let mut output = File::create(tmp)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        Ok(())
    })
}

#[cfg(target_os = "macos")]
fn clone_same_volume(source: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source_path = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the source path contains a null",
        )
    })?;
    let dest_path = std::ffi::CString::new(dest.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the mirror path contains a null",
        )
    })?;
    unsafe extern "C" {
        fn clonefile(src: *const std::ffi::c_char, dst: *const std::ffi::c_char, flags: u32)
            -> i32;
    }
    // SAFETY: both pointers come from CStrings that outlive this call. clonefile
    // creates `dest` and does not retain the pointers. flags 0 follows the source.
    let copied = unsafe { clonefile(source_path.as_ptr(), dest_path.as_ptr(), 0) };
    if copied == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn clone_same_volume(_source: &Path, _dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "clonefile is a macOS APFS call",
    ))
}

const MIRROR_WALK_LIMIT: u32 = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CopySource {
    NotMirror,
    Unmapped,
    Mapped(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MirrorAnalysisChoice {
    Measure,
    UseStored,
    LeaveUnanalyzed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MirrorFolderRow {
    pub path: PathBuf,
    pub name: String,
    pub track_count: u64,
    pub source_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MirrorFileRow {
    pub path: PathBuf,
    pub name: String,
    pub source_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MirrorBrowse {
    pub mirror_path: Option<PathBuf>,
    pub inside: bool,
    pub file_count: u64,
    pub source_paths: Vec<PathBuf>,
    pub folders: Vec<MirrorFolderRow>,
    pub files: Vec<MirrorFileRow>,
}

struct WalkedMp3 {
    source_path: Option<PathBuf>,
}

impl MirrorBrowse {
    fn empty() -> Self {
        Self {
            mirror_path: None,
            inside: false,
            file_count: 0,
            source_paths: Vec::new(),
            folders: Vec::new(),
            files: Vec::new(),
        }
    }
}

pub(crate) fn mirror_analysis_choice(
    source: &CopySource,
    source_has_tempo: bool,
) -> MirrorAnalysisChoice {
    match source {
        CopySource::NotMirror => MirrorAnalysisChoice::Measure,
        CopySource::Mapped(_) if source_has_tempo => MirrorAnalysisChoice::UseStored,
        CopySource::Mapped(_) | CopySource::Unmapped => MirrorAnalysisChoice::LeaveUnanalyzed,
    }
}

pub(crate) fn recorded_play_path(
    source: CopySource,
    played: &Path,
    source_in_library: bool,
) -> Option<PathBuf> {
    match source {
        CopySource::NotMirror => Some(played.to_path_buf()),
        CopySource::Mapped(path) if source_in_library => Some(path),
        CopySource::Mapped(_) | CopySource::Unmapped => None,
    }
}

pub(crate) fn copy_display_title(stem: &str) -> &str {
    let stripped = strip_order_prefix(stem);
    if stripped.is_empty() {
        stem
    } else {
        stripped
    }
}

pub(crate) fn copy_source(roots: &[String], file: &Path) -> CopySource {
    if !is_under_watch_mirror(roots, &file.to_string_lossy()) {
        return CopySource::NotMirror;
    }
    let Some(root) = single_watch_root(roots, file) else {
        return CopySource::Unmapped;
    };
    let Ok(stripped) = file.strip_prefix(&root) else {
        return CopySource::Unmapped;
    };
    let Some(relative) = relative_string(stripped) else {
        return CopySource::Unmapped;
    };
    let (manifest, _) = load_manifest(&root);
    match source_by_dest(&manifest).get(&relative) {
        Some(source) => CopySource::Mapped(root.join(source)),
        None => CopySource::Unmapped,
    }
}

/// The mirror directory for `open`, or its contents when `open` is already inside it.
/// Reads the directory and the manifest. It does not create files or library rows.
pub(crate) fn browse_mirror(roots: &[String], open: &Path) -> MirrorBrowse {
    let Some(root) = single_watch_root(roots, open) else {
        return MirrorBrowse::empty();
    };
    let open = trim_path(open);
    let inside = is_path_inside_mirror(&root, &open);
    let mirror_dir = if inside {
        open
    } else {
        mirror_root_dir(&root).join(open.strip_prefix(&root).unwrap_or(Path::new("")))
    };
    if !mirror_dir.is_dir() {
        return MirrorBrowse {
            mirror_path: Some(mirror_dir),
            inside,
            ..MirrorBrowse::empty()
        };
    }
    let (manifest, _) = load_manifest(&root);
    let sources = source_by_dest(&manifest);
    if inside {
        let (folders, files) = list_mirror_dir(&mirror_dir, &root, &sources);
        let file_count =
            files.len() as u64 + folders.iter().map(|folder| folder.track_count).sum::<u64>();
        let mut source_paths = files
            .iter()
            .filter_map(|file| file.source_path.clone())
            .collect::<Vec<_>>();
        for folder in &folders {
            source_paths.extend(folder.source_paths.iter().cloned());
        }
        return MirrorBrowse {
            mirror_path: Some(mirror_dir),
            inside: true,
            file_count,
            source_paths,
            folders,
            files,
        };
    }
    let walked = walk_files(&mirror_dir, &root, &sources, 0);
    let source_paths = walked
        .iter()
        .filter_map(|file| file.source_path.clone())
        .collect();
    MirrorBrowse {
        mirror_path: Some(mirror_dir),
        inside: false,
        file_count: walked.len() as u64,
        source_paths,
        folders: Vec::new(),
        files: Vec::new(),
    }
}

fn source_by_dest(manifest: &Manifest) -> BTreeMap<String, String> {
    let mut sources = BTreeMap::new();
    for entry in manifest.files.values() {
        if entry.dest.is_empty() || !safe_source_rel(&entry.source) {
            continue;
        }
        sources
            .entry(entry.dest.clone())
            .or_insert_with(|| entry.source.clone());
    }
    sources
}

fn single_watch_root(roots: &[String], open: &Path) -> Option<PathBuf> {
    let open = trim_path(open);
    let mut found = None;
    for root in roots {
        let trimmed = root.trim_end_matches(['/', '\\']);
        if trimmed.is_empty() {
            continue;
        }
        let root_path = PathBuf::from(trimmed);
        if open == root_path || open.starts_with(&root_path) {
            if found.is_some() {
                return None;
            }
            found = Some(root_path);
        }
    }
    found
}

fn trim_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        PathBuf::from(text.as_ref())
    } else {
        PathBuf::from(trimmed)
    }
}

fn is_path_inside_mirror(root: &Path, open: &Path) -> bool {
    let Ok(relative) = open.strip_prefix(root) else {
        return false;
    };
    relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(MIRROR_DIR))
}

fn mirror_root_dir(root: &Path) -> PathBuf {
    let fallback = root.join(MIRROR_DIR);
    let Ok(entries) = std::fs::read_dir(root) else {
        return fallback;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if entry.file_name().eq_ignore_ascii_case(MIRROR_DIR) {
            return entry.path();
        }
    }
    fallback
}

fn list_mirror_dir(
    dir: &Path,
    root: &Path,
    sources: &BTreeMap<String, String>,
) -> (Vec<MirrorFolderRow>, Vec<MirrorFileRow>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if hidden_mirror_entry(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            let nested = walk_files(&path, root, sources, 1);
            if nested.is_empty() {
                continue;
            }
            let source_paths = nested
                .iter()
                .filter_map(|file| file.source_path.clone())
                .collect();
            folders.push(MirrorFolderRow {
                path,
                name,
                track_count: nested.len() as u64,
                source_paths,
            });
        } else if file_type.is_file() && is_mp3_name(&name) {
            files.push(MirrorFileRow {
                source_path: mapped_source(root, &path, sources),
                path,
                name,
            });
        }
    }
    folders.sort_by(|left, right| cmp_numeric_names(&left.name, &right.name));
    files.sort_by(|left, right| cmp_numeric_names(&left.name, &right.name));
    (folders, files)
}

fn walk_files(
    dir: &Path,
    root: &Path,
    sources: &BTreeMap<String, String>,
    depth: u32,
) -> Vec<WalkedMp3> {
    if depth > MIRROR_WALK_LIMIT {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if hidden_mirror_entry(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            files.extend(walk_files(&path, root, sources, depth + 1));
        } else if file_type.is_file() && is_mp3_name(&name) {
            files.push(WalkedMp3 {
                source_path: mapped_source(root, &path, sources),
            });
        }
    }
    files
}

fn mapped_source(root: &Path, file: &Path, sources: &BTreeMap<String, String>) -> Option<PathBuf> {
    let relative = relative_string(file.strip_prefix(root).ok()?)?;
    sources.get(&relative).map(|source| root.join(source))
}

fn hidden_mirror_entry(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('.')
        || name.ends_with(".partial")
        || name.contains(".opendeck-order-")
}

fn is_mp3_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("mp3"))
}

fn cmp_numeric_names(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char))
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() =>
            {
                let left_digits = take_digits(&mut left_chars);
                let right_digits = take_digits(&mut right_chars);
                let order = cmp_digit_runs(&left_digits, &right_digits);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(left_char), Some(right_char)) => {
                let order = left_char
                    .to_ascii_lowercase()
                    .cmp(&right_char.to_ascii_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left_chars.next();
                right_chars.next();
            }
        }
    }
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while chars
        .peek()
        .is_some_and(|character| character.is_ascii_digit())
    {
        if let Some(character) = chars.next() {
            digits.push(character);
        }
    }
    digits
}

fn cmp_digit_runs(left: &str, right: &str) -> Ordering {
    let left_trimmed = left.trim_start_matches('0');
    let right_trimmed = right.trim_start_matches('0');
    let left_value = if left_trimmed.is_empty() {
        "0"
    } else {
        left_trimmed
    };
    let right_value = if right_trimmed.is_empty() {
        "0"
    } else {
        right_trimmed
    };
    left_value
        .len()
        .cmp(&right_value.len())
        .then_with(|| left_value.cmp(right_value))
        .then_with(|| left.len().cmp(&right.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_encoded_copy_is_shifted_behind_its_source() {
        let encoded = encoded_copy_delay_sec(Path::new("/music/set/song.m4a"));
        assert!((encoded - 1105.0 / 44_100.0).abs() < 1e-12);
        assert_eq!(
            encoded_copy_delay_sec(Path::new("/music/set/song.MP3")),
            0.0
        );
    }

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
    fn mirror_paths_keep_the_tree_and_include_mp3_sources() {
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
        assert_eq!(mirror_dest_rel("song.mp3").as_deref(), Some("mp3/song.mp3"));
        assert_eq!(mirror_dest_rel("song.MP3").as_deref(), Some("mp3/song.mp3"));
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
        assert!(plan
            .encode
            .iter()
            .all(|job| job.source_rel != "already.mp3"));
        assert!(plan
            .copy
            .iter()
            .any(|job| job.source_rel == "already.mp3" && job.dest_rel == "mp3/already.mp3"));
        assert!(plan
            .encode
            .iter()
            .any(|job| job.source_rel == "song.wav" && job.dest_rel == "mp3/song (2).mp3"));
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
    fn source_stamps_include_mp3_sources_and_skip_the_mirror_tree() {
        let root = scratch("stamps");
        std::fs::create_dir(root.join("Artist")).unwrap();
        std::fs::write(root.join("Artist").join("a.flac"), b"fLaC").unwrap();
        std::fs::write(root.join("already.mp3"), b"ID3").unwrap();
        std::fs::create_dir(root.join("mp3")).unwrap();
        std::fs::write(root.join("mp3").join("hidden.wav"), b"RIFF").unwrap();
        let stamps = source_stamps(&root);
        assert_eq!(stamps.len(), 2);
        assert_eq!(stamps[0].rel, "Artist/a.flac");
        assert_eq!(stamps[1].rel, "already.mp3");
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
    fn ordered_folder_numbers_mp3_copies_with_the_lossless_files() {
        let sources = vec![
            named("01 - a.flac", 4, 4, 1, 1, "Ada", "First"),
            named("02 - b.mp3", 4, 4, 1, 2, "Bea", "Second"),
            named("03 - c.wav", 4, 4, 1, 3, "Cy", "Third"),
        ];
        let plan = plan_mirror(
            &sources,
            &Manifest::default(),
            OrphanPolicy::KeepAll,
            |_| false,
        );
        assert_eq!(plan.copy.len(), 1);
        assert_eq!(plan.copy[0].source_rel, "02 - b.mp3");
        assert_eq!(plan.copy[0].dest_rel, "mp3/2 - Bea - Second.mp3");
        assert_eq!(plan.encode.len(), 2);
        assert_eq!(plan.encode[0].dest_rel, "mp3/1 - Ada - First.mp3");
        assert_eq!(plan.encode[1].dest_rel, "mp3/3 - Cy - Third.mp3");
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
        assert!(playback_should_pause([0.0, 44_100.0]));
        assert!(!playback_should_pause([0.0, 0.0]));
        assert!(!playback_should_pause([f64::NAN]));
        assert!(!machine_is_busy(0.2));
        assert!(!machine_is_busy(CPU_YIELD_LOAD_PER_CPU));
        assert!(machine_is_busy(CPU_YIELD_LOAD_PER_CPU + 0.01));
        assert!(!machine_is_busy(f64::NAN));
    }

    #[test]
    fn mirror_listing_follows_filename_order_and_maps_the_source() {
        let root = scratch("browse");
        let mirror = root.join("mp3");
        std::fs::create_dir(&mirror).unwrap();
        std::fs::write(mirror.join("2 - Ada - Second.mp3"), b"b").unwrap();
        std::fs::write(mirror.join("10 - Ada - Tenth.mp3"), b"t").unwrap();
        std::fs::write(mirror.join("1 - Ada - First.mp3"), b"a").unwrap();
        std::fs::write(mirror.join("song.mp3.partial"), b"p").unwrap();
        std::fs::write(mirror.join(".opendeck-order-0001"), b"o").unwrap();
        std::fs::write(mirror.join("notes.txt"), b"n").unwrap();
        let nested = mirror.join("Album");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("3 - Ada - Nested.mp3"), b"n").unwrap();
        let manifest = Manifest {
            version: 2,
            files: BTreeMap::from([
                (
                    "dev:1:ino:1".to_string(),
                    ManifestFile {
                        source: "01 - first.flac".to_string(),
                        source_len: 1,
                        source_mtime_ns: 1,
                        dest: "mp3/1 - Ada - First.mp3".to_string(),
                        source_dev: 1,
                        source_ino: 1,
                    },
                ),
                (
                    "dev:1:ino:2".to_string(),
                    ManifestFile {
                        source: "02 - second.flac".to_string(),
                        source_len: 1,
                        source_mtime_ns: 1,
                        dest: "mp3/2 - Ada - Second.mp3".to_string(),
                        source_dev: 1,
                        source_ino: 2,
                    },
                ),
            ]),
        };
        save_manifest(&root, &manifest).unwrap();
        let names_before = dir_names(&mirror);
        let roots = vec![root.to_string_lossy().into_owned()];
        let outside = browse_mirror(&roots, &root);
        assert!(!outside.inside);
        assert_eq!(outside.file_count, 4);
        assert!(outside.files.is_empty());
        assert_eq!(outside.mirror_path.as_deref(), Some(mirror.as_path()));
        let inside = browse_mirror(&roots, &mirror);
        assert!(inside.inside);
        assert_eq!(
            inside
                .files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "1 - Ada - First.mp3",
                "2 - Ada - Second.mp3",
                "10 - Ada - Tenth.mp3",
            ]
        );
        assert_eq!(inside.folders.len(), 1);
        assert_eq!(inside.folders[0].name, "Album");
        assert_eq!(inside.folders[0].track_count, 1);
        assert_eq!(
            inside.files[0].source_path.as_deref(),
            Some(root.join("01 - first.flac").as_path())
        );
        assert!(inside.files[2].source_path.is_none());
        assert_eq!(dir_names(&mirror), names_before);
        let nested_view = browse_mirror(&roots, &nested);
        assert!(nested_view.inside);
        assert_eq!(nested_view.files.len(), 1);
        assert!(nested_view.folders.is_empty());
        let copy = mirror.join("1 - Ada - First.mp3");
        assert_eq!(
            copy_source(&roots, &copy),
            CopySource::Mapped(root.join("01 - first.flac"))
        );
        assert_eq!(
            copy_source(&roots, &mirror.join("10 - Ada - Tenth.mp3")),
            CopySource::Unmapped
        );
        assert_eq!(
            mirror_analysis_choice(&CopySource::Mapped(root.join("01 - first.flac")), true),
            MirrorAnalysisChoice::UseStored
        );
        assert_eq!(
            mirror_analysis_choice(&CopySource::Mapped(root.join("01 - first.flac")), false),
            MirrorAnalysisChoice::LeaveUnanalyzed
        );
        assert_eq!(
            mirror_analysis_choice(&CopySource::Unmapped, false),
            MirrorAnalysisChoice::LeaveUnanalyzed
        );
        assert_eq!(
            mirror_analysis_choice(&CopySource::NotMirror, false),
            MirrorAnalysisChoice::Measure
        );
        assert_eq!(
            recorded_play_path(
                CopySource::Mapped(root.join("01 - first.flac")),
                &copy,
                true
            ),
            Some(root.join("01 - first.flac"))
        );
        assert_eq!(recorded_play_path(CopySource::Unmapped, &copy, false), None);
        let empty = scratch("missing");
        let missing = browse_mirror(&[empty.to_string_lossy().into_owned()], &empty);
        assert_eq!(missing.file_count, 0);
        assert!(!empty.join("mp3").exists());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn finished_folder_counts_converted_sources_not_mirror_entries() {
        let root = scratch("status");
        std::fs::create_dir_all(root.join("mp3")).unwrap();
        for index in 1..=23 {
            std::fs::write(root.join(format!("track-{index:02}.mp3")), b"mp3").unwrap();
        }
        std::fs::write(root.join("one.flac"), b"flac-one").unwrap();
        std::fs::write(root.join("two.flac"), b"flac-two").unwrap();
        let mut manifest = Manifest {
            version: 2,
            files: BTreeMap::new(),
        };
        for (source, dest) in [("one.flac", "mp3/one.mp3"), ("two.flac", "mp3/two.mp3")] {
            let meta = std::fs::metadata(root.join(source)).unwrap();
            let (dev, ino) = file_identity(&meta);
            let mtime_ns = meta
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            std::fs::write(root.join(dest), b"copy").unwrap();
            manifest.files.insert(
                identity_key(dev, ino, source),
                ManifestFile {
                    source: source.to_string(),
                    source_len: meta.len(),
                    source_mtime_ns: u64::try_from(mtime_ns).unwrap(),
                    dest: dest.to_string(),
                    source_dev: dev,
                    source_ino: ino,
                },
            );
        }
        save_manifest(&root, &manifest).unwrap();
        let finished = conversion_status(&root, "");
        assert_eq!(finished.converted, 2);
        assert_eq!(finished.convertible, 25);
        assert_eq!(finished.already_mp3, 23);
        std::fs::remove_file(root.join("mp3/two.mp3")).unwrap();
        let partial = conversion_status(&root, "");
        assert_eq!(partial.converted, 1);
        assert_eq!(partial.convertible, 25);
        assert_eq!(partial.already_mp3, 23);
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}
