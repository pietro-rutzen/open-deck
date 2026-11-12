//! One background pass over a watched folder. The audio callback never calls this.

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

pub(crate) fn sync_folder(
    root: &Path,
    mut enabled: impl FnMut() -> bool,
    mut between_files: impl FnMut(),
    mut during_encode: impl FnMut(),
    mut hint: impl FnMut(&Path) -> AnalysisHint,
    mut progress: impl FnMut(u32, u32, bool),
) -> Result<Vec<String>, String> {
    if !root.is_dir() {
        progress(0, 0, false);
        return Ok(Vec::new());
    }
    if !enabled() {
        progress(0, 0, false);
        return Ok(Vec::new());
    }
    let sources = mp3_mirror::source_stamps(root);
    let (manifest, policy) = mp3_mirror::load_manifest(root);
    let plan = mp3_mirror::plan_mirror(&sources, &manifest, policy, |dest| {
        mirrored_path(root, dest).is_some_and(|path| path.is_file())
    });
    let total =
        u32::try_from(plan.encode.len() + plan.skip.len() + plan.delete.len()).unwrap_or(u32::MAX);
    let mut done = u32::try_from(plan.skip.len()).unwrap_or(0).min(total);
    if plan.encode.is_empty() && plan.delete.is_empty() {
        progress(0, 0, false);
        return Ok(Vec::new());
    }
    progress(done, total, true);
    let mut manifest = manifest;
    let mut errors = Vec::new();
    for job in &plan.encode {
        if !enabled() {
            progress(0, 0, false);
            return Ok(errors);
        }
        between_files();
        if !enabled() {
            progress(0, 0, false);
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
            continue;
        }
        let (source_len, source_mtime_ns) = stamp;
        manifest.files.insert(
            job.dest_rel.clone(),
            ManifestFile {
                source: job.source_rel.clone(),
                source_len,
                source_mtime_ns,
            },
        );
        if let Err(error) = mp3_mirror::save_manifest(root, &manifest) {
            manifest.files.remove(&job.dest_rel);
            errors.push(format!("manifest: {error}"));
            continue;
        }
        done = done.saturating_add(1).min(total);
        progress(done, total, true);
    }
    if !enabled() {
        progress(0, 0, false);
        return Ok(errors);
    }
    for dest in &plan.delete {
        let Some(entry) = manifest.files.get(dest).cloned() else {
            continue;
        };
        if !source_is_confirmed_missing(root, &entry.source) {
            continue;
        }
        if let Some(path) = mirrored_path(root, dest) {
            if path.is_file() && std::fs::remove_file(&path).is_err() {
                errors.push(format!("could not remove {dest}"));
                continue;
            }
            remove_empty_parents(root, &path);
        }
        manifest.files.remove(dest);
        done = done.saturating_add(1).min(total);
        progress(done, total, true);
    }
    if let Err(error) = store_manifest(root, &manifest, policy) {
        errors.push(format!("manifest: {error}"));
    }
    progress(0, 0, false);
    Ok(errors)
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
    use id3::TagLike;

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
        let errors = sync_folder(&root, || true, || {}, || {}, hint, |_, _, _| {}).unwrap();
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
        let errors = sync_folder(&root, || true, || {}, || {}, hint, |_, _, _| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(std::fs::read(&mirrored).unwrap(), first);
        assert_eq!(mirrored.metadata().unwrap().modified().unwrap(), modified);
        write_stereo_pcm_wav(&root.join("Artist").join("a.wav"), 44_100, &silence(8_000));
        let errors = sync_folder(&root, || true, || {}, || {}, hint, |_, _, _| {}).unwrap();
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
        let errors = sync_folder(&root, || true, || {}, || {}, hint, |_, _, _| {}).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!mirrored.exists());
        assert_eq!(
            std::fs::read(root.join("mp3").join("keep-user.mp3")).unwrap(),
            b"user-file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
