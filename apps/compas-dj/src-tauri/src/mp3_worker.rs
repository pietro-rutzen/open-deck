//! One background pass over a watched folder. The audio callback never calls this.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::mp3_encode;
use crate::mp3_mirror::{self, Manifest, ManifestFile, OrphanPolicy, MIRROR_DIR};

pub(crate) struct AnalysisHint {
    pub(crate) bpm: Option<f64>,
    pub(crate) musical_key: Option<String>,
}

pub(crate) fn lower_thread_priority() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        const QOS_CLASS_BACKGROUND: u32 = 0x09;
        // SAFETY: this changes only the calling thread's QoS. 0x09 is
        // QOS_CLASS_BACKGROUND from Apple's qos.h, and 0 is that class's default.
        let _ = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0) };
    }
}

pub(crate) fn load_per_cpu() -> f64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
        }
        let mut load = [0.0];
        // SAFETY: getloadavg writes at most one f64 into `load`.
        let wrote = unsafe { getloadavg(load.as_mut_ptr(), 1) };
        if wrote < 1 || !load[0].is_finite() || load[0] < 0.0 {
            return 0.0;
        }
        let cpus = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .max(1);
        return load[0] / cpus as f64;
    }
    #[cfg(not(target_os = "macos"))]
    0.0
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyncProgress {
    pub(crate) converted: u32,
    pub(crate) convertible: u32,
    pub(crate) already_mp3: u32,
    pub(crate) failures: Vec<String>,
    pub(crate) active: bool,
}

pub(crate) fn sync_folder(
    root: &Path,
    mut enabled: impl FnMut() -> bool,
    mut between_files: impl FnMut(),
    mut during_encode: impl FnMut(),
    names: &HashMap<String, (String, String)>,
    mut hint: impl FnMut(&Path) -> AnalysisHint,
    mut progress: impl FnMut(SyncProgress),
) -> Result<Vec<String>, String> {
    if !root.is_dir() {
        progress(SyncProgress {
            converted: 0,
            convertible: 0,
            already_mp3: 0,
            failures: Vec::new(),
            active: false,
        });
        return Ok(Vec::new());
    }
    if !enabled() {
        report_progress(root, false, &[], &mut progress);
        return Ok(Vec::new());
    }
    let mut sources = mp3_mirror::source_stamps(root);
    for source in &mut sources {
        let path = root.join(&source.rel);
        let Some(path) = path.to_str() else {
            continue;
        };
        let Some((artist, title)) = names.get(path) else {
            continue;
        };
        source.artist.clone_from(artist);
        source.title.clone_from(title);
    }
    let (manifest, policy) = mp3_mirror::load_manifest(root);
    let plan = mp3_mirror::plan_mirror(&sources, &manifest, policy, |dest| {
        mirrored_path(root, dest).is_some_and(|path| path.is_file())
    });
    let keeps_differ = plan.keep.iter().any(|job| {
        job.previous_key != job.identity || manifest.files.get(&job.identity) != Some(&job.entry)
    });
    let mut manifest = manifest;
    remember_keeps(&mut manifest, &plan.keep);
    if plan.encode.is_empty() && plan.rename.is_empty() && plan.delete.is_empty() {
        if keeps_differ {
            if let Err(error) = store_manifest(root, &manifest, policy) {
                return Err(error.to_string());
            }
        }
        report_progress(root, false, &[], &mut progress);
        return Ok(Vec::new());
    }
    report_progress(root, true, &[], &mut progress);
    let mut errors = Vec::new();
    if !plan.rename.is_empty() {
        between_files();
        if !enabled() {
            report_progress(root, false, &errors, &mut progress);
            return Ok(errors);
        }
        match rename_mirrors(root, &plan.rename) {
            Ok(()) => {
                for job in &plan.rename {
                    manifest.files.remove(&job.previous_key);
                    manifest
                        .files
                        .insert(job.identity.clone(), job.entry.clone());
                }
                if let Err(error) = mp3_mirror::save_manifest(root, &manifest) {
                    errors.push(format!("manifest: {error}"));
                }
                report_progress(root, true, &errors, &mut progress);
            }
            Err(error) => errors.push(error),
        }
    }
    for job in &plan.encode {
        if !enabled() {
            report_progress(root, false, &errors, &mut progress);
            return Ok(errors);
        }
        between_files();
        if !enabled() {
            report_progress(root, false, &errors, &mut progress);
            return Ok(errors);
        }
        let Some(source_path) = safe_joined(root, &job.source_rel) else {
            errors.push(format!(
                "{} is not a path inside the folder",
                job.source_rel
            ));
            continue;
        };
        let Some(dest_path) = mirrored_path(root, &job.dest_rel) else {
            errors.push(format!("{} is not a mirror path", job.dest_rel));
            continue;
        };
        let stamp = match file_stamp(&source_path) {
            Ok(Some(stamp)) => stamp,
            Ok(None) => continue,
            Err(error) => {
                errors.push(format!("{}: {error}", job.source_rel));
                continue;
            }
        };
        let analysis = hint(&source_path);
        if let Err(error) = mp3_encode::encode_mirror(
            &source_path,
            &dest_path,
            analysis.bpm,
            analysis.musical_key.as_deref(),
            &mut during_encode,
        ) {
            errors.push(format!("{}: {error}", job.source_rel));
            report_progress(root, true, &errors, &mut progress);
            continue;
        }
        let (source_len, source_mtime_ns) = stamp;
        manifest.files.insert(
            job.identity.clone(),
            ManifestFile {
                source: job.source_rel.clone(),
                source_len,
                source_mtime_ns,
                dest: job.dest_rel.clone(),
                source_dev: job.dev,
                source_ino: job.ino,
            },
        );
        if let Err(error) = mp3_mirror::save_manifest(root, &manifest) {
            manifest.files.remove(&job.identity);
            errors.push(format!("manifest: {error}"));
            report_progress(root, true, &errors, &mut progress);
            continue;
        }
        report_progress(root, true, &errors, &mut progress);
    }
    if !enabled() {
        report_progress(root, false, &errors, &mut progress);
        return Ok(errors);
    }
    for job in &plan.delete {
        if !source_is_confirmed_missing(root, &job.source_rel) {
            continue;
        }
        if let Some(path) = mirrored_path(root, &job.dest_rel) {
            if path.is_file() && std::fs::remove_file(&path).is_err() {
                errors.push(format!("could not remove {}", job.dest_rel));
                continue;
            }
            remove_empty_parents(root, &path);
        }
        manifest.files.remove(&job.identity);
        report_progress(root, true, &errors, &mut progress);
    }
    if let Err(error) = store_manifest(root, &manifest, policy) {
        errors.push(format!("manifest: {error}"));
    }
    report_progress(root, false, &errors, &mut progress);
    Ok(errors)
}

fn report_progress(
    root: &Path,
    active: bool,
    failures: &[String],
    progress: &mut impl FnMut(SyncProgress),
) {
    let status = mp3_mirror::conversion_status(root, "");
    progress(SyncProgress {
        converted: status.converted,
        convertible: status.convertible,
        already_mp3: status.already_mp3,
        failures: failures.to_vec(),
        active,
    });
}

fn file_stamp(path: &Path) -> Result<Option<(u64, u64)>, String> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .and_then(|elapsed| u64::try_from(elapsed.as_nanos()).ok())
                .ok_or_else(|| format!("{} has no modification time", path.display()))?;
            Ok(Some((meta.len(), mtime_ns)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn source_is_confirmed_missing(root: &Path, source_rel: &str) -> bool {
    let Some(path) = safe_joined(root, source_rel) else {
        return false;
    };
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        _ => false,
    }
}

fn safe_joined(root: &Path, rel: &str) -> Option<PathBuf> {
    if !mp3_mirror::safe_source_rel(rel) {
        return None;
    }
    let path = root.join(rel);
    path.starts_with(root).then_some(path)
}

fn mirrored_path(root: &Path, rel: &str) -> Option<PathBuf> {
    if !mp3_mirror::safe_mirror_rel(rel) {
        return None;
    }
    let path = root.join(rel);
    path.starts_with(root.join(MIRROR_DIR)).then_some(path)
}

fn remove_empty_parents(root: &Path, file: &Path) {
    let mirror = root.join(MIRROR_DIR);
    let mut directory = file.parent().map(Path::to_path_buf);
    while let Some(current) = directory {
        if current == mirror || !current.starts_with(&mirror) {
            break;
        }
        if std::fs::remove_dir(&current).is_err() {
            break;
        }
        directory = current.parent().map(Path::to_path_buf);
    }
}

fn remember_keeps(manifest: &mut Manifest, keeps: &[mp3_mirror::KeepJob]) {
    for job in keeps {
        if job.previous_key != job.identity {
            manifest.files.remove(&job.previous_key);
        }
        manifest
            .files
            .insert(job.identity.clone(), job.entry.clone());
    }
}

fn rename_mirrors(root: &Path, jobs: &[mp3_mirror::RenameJob]) -> Result<(), String> {
    let mut parked = Vec::with_capacity(jobs.len());
    for (index, job) in jobs.iter().enumerate() {
        let Some(from) = mirrored_path(root, &job.from_rel) else {
            undo_parked(&parked);
            return Err(format!("{} is not a mirror path", job.from_rel));
        };
        let Some(parent) = from.parent() else {
            undo_parked(&parked);
            return Err(format!("{} has no folder", job.from_rel));
        };
        let temp = parent.join(format!(".opendeck-order-{index:04}"));
        if let Err(error) = std::fs::rename(&from, &temp) {
            undo_parked(&parked);
            return Err(format!("{}: {error}", job.from_rel));
        }
        parked.push((temp, from, job.to_rel.clone()));
    }
    parked.sort_by(|left, right| left.2.cmp(&right.2));
    for (temp, _from, to_rel) in &parked {
        let Some(to) = mirrored_path(root, to_rel) else {
            return Err(format!("{to_rel} is not a mirror path"));
        };
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::rename(temp, to) {
            return Err(format!("{to_rel}: {error}"));
        }
    }
    Ok(())
}

fn undo_parked(parked: &[(PathBuf, PathBuf, String)]) {
    for (temp, original, _) in parked.iter().rev() {
        let _ = std::fs::rename(temp, original);
    }
}

fn store_manifest(root: &Path, manifest: &Manifest, policy: OrphanPolicy) -> std::io::Result<()> {
    if manifest.files.is_empty() {
        if policy == OrphanPolicy::DeleteTracked || mp3_mirror::manifest_path(root).is_file() {
            let _ = std::fs::remove_file(mp3_mirror::manifest_path(root));
        }
        let _ = std::fs::remove_dir(root.join(MIRROR_DIR));
        return Ok(());
    }
    mp3_mirror::save_manifest(root, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp3_encode::{test_encode_lock, write_stereo_pcm_wav};
    use crate::mp3_mirror::{identity_key, save_manifest, Manifest, ManifestFile};
    use id3::TagLike;
    use std::os::unix::fs::MetadataExt;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opendeck-mp3-sync-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn silence(frames: usize) -> Vec<i16> {
        vec![0; frames * 2]
    }

    #[test]
    fn sync_skips_mp3_sources_reencodes_changes_and_deletes_only_its_files() {
        let _guard = test_encode_lock();
        let root = scratch("sync");
        std::fs::create_dir(root.join("Artist")).unwrap();
        write_stereo_pcm_wav(&root.join("Artist").join("a.wav"), 44_100, &silence(4_000));
        write_stereo_pcm_wav(&root.join("already.mp3"), 44_100, &silence(1_000));
        std::fs::create_dir(root.join("mp3")).unwrap();
        std::fs::write(root.join("mp3").join("keep-user.mp3"), b"user-file").unwrap();
        let hint = |_path: &Path| AnalysisHint {
            bpm: Some(128.0),
            musical_key: Some("Am".to_string()),
        };
        let names = HashMap::new();
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let mirrored = root.join("mp3").join("Artist").join("a.mp3");
        assert!(mirrored.is_file());
        assert!(!root.join("mp3").join("already.mp3").exists());
        assert_eq!(
            std::fs::read(root.join("mp3").join("keep-user.mp3")).unwrap(),
            b"user-file"
        );
        let first = std::fs::read(&mirrored).unwrap();
        let modified = mirrored.metadata().unwrap().modified().unwrap();
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(std::fs::read(&mirrored).unwrap(), first);
        assert_eq!(mirrored.metadata().unwrap().modified().unwrap(), modified);
        write_stereo_pcm_wav(&root.join("Artist").join("a.wav"), 44_100, &silence(8_000));
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_ne!(std::fs::read(&mirrored).unwrap(), first);
        let tag = id3::Tag::read_from_path(&mirrored).unwrap();
        assert_eq!(
            tag.get("TBPM").and_then(|frame| frame.content().text()),
            Some("128")
        );
        assert_eq!(
            tag.get("TKEY").and_then(|frame| frame.content().text()),
            Some("Am")
        );
        std::fs::remove_file(root.join("Artist").join("a.wav")).unwrap();
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!mirrored.exists());
        assert_eq!(
            std::fs::read(root.join("mp3").join("keep-user.mp3")).unwrap(),
            b"user-file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reorder_renames_the_mp3_without_reencoding_and_keeps_orphan_cleanup() {
        let _guard = test_encode_lock();
        let root = scratch("reorder");
        write_stereo_pcm_wav(&root.join("a.wav"), 44_100, &silence(4_000));
        std::fs::create_dir(root.join("mp3")).unwrap();
        std::fs::write(root.join("mp3").join("keep-user.mp3"), b"user-file").unwrap();
        let source = root.join("a.wav");
        let source_path = source.to_str().unwrap().to_string();
        let names = HashMap::from([(source_path, ("Ada".to_string(), "Song".to_string()))]);
        let hint = |_path: &Path| AnalysisHint {
            bpm: None,
            musical_key: None,
        };
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let mirrored = root.join("mp3").join("a.mp3");
        let first = std::fs::read(&mirrored).unwrap();
        let inode = std::fs::metadata(&mirrored).unwrap().ino();
        let tag = id3::Tag::read_from_path(&mirrored).ok();
        let title = tag.as_ref().and_then(|tag| tag.title());
        assert_eq!(title, None);

        let ordered = root.join("01 - a.wav");
        std::fs::rename(&source, &ordered).unwrap();
        let ordered_path = ordered.to_str().unwrap().to_string();
        let names = HashMap::from([(ordered_path, ("Ada".to_string(), "Song".to_string()))]);
        let errors =
            sync_folder(&root, || true, || {}, || {}, &names, hint, |_progress| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let renamed = root.join("mp3").join("1 - Ada - Song.mp3");
        assert!(renamed.is_file());
        assert!(!mirrored.exists());
        assert_eq!(std::fs::read(&renamed).unwrap(), first);
        assert_eq!(std::fs::metadata(&renamed).unwrap().ino(), inode);
        let tag = id3::Tag::read_from_path(&renamed).ok();
        assert_eq!(tag.as_ref().and_then(|tag| tag.title()), None);

        let manifest =
            std::fs::read_to_string(root.join("mp3").join(".opendeck-mp3-manifest.json")).unwrap();
        assert!(manifest.contains("01 - a.wav"));
        assert!(manifest.contains("1 - Ada - Song.mp3"));
        assert!(manifest.contains("source_ino"));

        std::fs::remove_file(&ordered).unwrap();
        let errors = sync_folder(
            &root,
            || true,
            || {},
            || {},
            &HashMap::new(),
            hint,
            |_progress| {},
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!renamed.exists());
        assert_eq!(
            std::fs::read(root.join("mp3").join("keep-user.mp3")).unwrap(),
            b"user-file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn idle_folder_reports_converted_sources_not_a_cleared_counter() {
        let root = scratch("progress");
        std::fs::create_dir_all(root.join("mp3")).unwrap();
        for index in 1..=23 {
            std::fs::write(root.join(format!("track-{index:02}.mp3")), b"mp3").unwrap();
        }
        let mut manifest = Manifest {
            version: 2,
            files: std::collections::BTreeMap::new(),
        };
        for (source, dest) in [("one.flac", "mp3/one.mp3"), ("two.flac", "mp3/two.mp3")] {
            std::fs::write(root.join(source), b"flac").unwrap();
            std::fs::write(root.join(dest), b"copy").unwrap();
            let meta = std::fs::metadata(root.join(source)).unwrap();
            let mtime_ns = meta
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            manifest.files.insert(
                identity_key(meta.dev(), meta.ino(), source),
                ManifestFile {
                    source: source.to_string(),
                    source_len: meta.len(),
                    source_mtime_ns: u64::try_from(mtime_ns).unwrap(),
                    dest: dest.to_string(),
                    source_dev: meta.dev(),
                    source_ino: meta.ino(),
                },
            );
        }
        save_manifest(&root, &manifest).unwrap();
        let mut seen = Vec::new();
        let errors = sync_folder(
            &root,
            || true,
            || {},
            || {},
            &HashMap::new(),
            |_path| AnalysisHint {
                bpm: None,
                musical_key: None,
            },
            |progress| seen.push(progress),
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let last = seen
            .last()
            .expect("a finished folder still reports its sources");
        assert_eq!(last.converted, 2);
        assert_eq!(last.convertible, 2);
        assert_eq!(last.already_mp3, 23);
        assert!(last.failures.is_empty());
        assert!(!last.active);
        assert!(seen
            .iter()
            .all(|progress| !(progress.converted == 0 && progress.convertible > 0)));
        let _ = std::fs::remove_dir_all(&root);
    }
}
