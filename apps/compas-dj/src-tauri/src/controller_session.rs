//! Control-thread controller preference persistence and conservative port matching.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use compas_core::ControllerProfile;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "controller-session.json";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionPreference {
    pub profile: Option<ControllerProfile>,
    pub port: Option<String>,
}

pub(crate) fn load_preference(base: &Path) -> SessionPreference {
    let Ok(file) = std::fs::File::open(base.join(FILE_NAME)) else {
        return SessionPreference::default();
    };
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES {
        return SessionPreference::default();
    }
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub(crate) fn save_preference(base: &Path, preference: &SessionPreference) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(preference).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Controller preference exceeds 2 MiB".into());
    }
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temporary = base.join(format!(
        ".controller-session-{}-{nonce}.tmp",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, base.join(FILE_NAME))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}

fn input_hint(profile: &ControllerProfile) -> Option<String> {
    profile
        .ports
        .input
        .as_deref()
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(str::to_lowercase)
}

fn matching_ports<'a>(ports: &'a [&str], hint: &str) -> Vec<&'a str> {
    ports
        .iter()
        .copied()
        .filter(|port| port.to_lowercase().contains(hint))
        .collect()
}

/// Never guess between devices. A saved snapshot outranks on-disk profile edits.
pub(crate) fn choose_profile(
    profiles: &[ControllerProfile],
    ports: &[String],
    preference: &SessionPreference,
) -> Option<(ControllerProfile, String)> {
    let mut ports: Vec<&str> = ports.iter().map(String::as_str).collect();
    let mut counts = std::collections::BTreeMap::new();
    for port in &ports {
        *counts.entry(*port).or_insert(0usize) += 1;
    }
    // Names are the only identity available here: duplicates cannot be resolved safely.
    ports.retain(|port| counts.get(port) == Some(&1));
    ports.sort_unstable();
    if let Some(profile) = preference.profile.as_ref() {
        let hint = input_hint(profile);
        if let Some(port) = preference
            .port
            .as_deref()
            .filter(|port| ports.contains(port))
        {
            if hint
                .as_ref()
                .is_none_or(|hint| port.to_lowercase().contains(hint))
            {
                return Some((profile.clone(), port.to_owned()));
            }
        }
        if let Some(hint) = hint {
            let matching = matching_ports(&ports, &hint);
            if matching.len() == 1 {
                return Some((profile.clone(), matching[0].to_owned()));
            }
            // Multiple matches remain ambiguous; global discovery must not override
            // an unresolved remembered profile with a shorter generic hint.
            if matching.len() > 1 {
                return None;
            }
        }
    }
    let mut candidates = Vec::new();
    for port in ports {
        let lower_port = port.to_lowercase();
        let matches: Vec<_> = profiles
            .iter()
            .filter_map(|profile| {
                input_hint(profile)
                    .filter(|hint| lower_port.contains(hint))
                    .map(|hint| (profile, hint.len()))
            })
            .collect();
        let Some(longest) = matches.iter().map(|(_, length)| *length).max() else {
            continue;
        };
        for (profile, _) in matches.into_iter().filter(|(_, length)| *length == longest) {
            candidates.push((profile, port));
        }
    }
    if candidates.len() == 1 {
        let (profile, port) = candidates[0];
        Some((profile.clone(), port.to_owned()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compas_core::PortHints;

    fn profile(id: &str, hint: Option<&str>) -> ControllerProfile {
        ControllerProfile {
            id: id.into(),
            name: id.into(),
            ports: PortHints {
                input: hint.map(str::to_owned),
                output: None,
            },
            ..Default::default()
        }
    }
    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn bundled() -> ControllerProfile {
        serde_json::from_str(include_str!(
            "../resources/controllers/pioneer-ddj-flx4.json"
        ))
        .unwrap()
    }
    #[test]
    fn identical_port_names_remain_ambiguous_even_when_remembered() {
        let preference = SessionPreference {
            profile: Some(bundled()),
            port: Some("DDJ-FLX4".into()),
        };
        assert!(
            choose_profile(&[bundled()], &ports(&["DDJ-FLX4", "DDJ-FLX4"]), &preference).is_none()
        );
    }
    #[test]
    fn bundled_flx4_does_not_choose_launchkey() {
        let found = choose_profile(
            &[bundled()],
            &ports(&["Launchkey MK4 MIDI", "DDJ-FLX4"]),
            &SessionPreference::default(),
        )
        .unwrap();
        assert_eq!(found.1, "DDJ-FLX4");
        assert!(choose_profile(
            &[bundled()],
            &ports(&["Launchkey MK4 MIDI"]),
            &SessionPreference::default()
        )
        .is_none());
    }
    #[test]
    fn remembered_snapshot_resolves_ambiguity_and_keeps_edits() {
        let mut snapshot = bundled();
        snapshot.name = "Unsaved custom mapping".into();
        let preference = SessionPreference {
            profile: Some(snapshot),
            port: Some("DDJ-FLX4 #2".into()),
        };
        let found = choose_profile(
            &[bundled()],
            &ports(&["DDJ-FLX4 #1", "DDJ-FLX4 #2"]),
            &preference,
        )
        .unwrap();
        assert_eq!(found.0.name, "Unsaved custom mapping");
        assert_eq!(found.1, "DDJ-FLX4 #2");
    }
    #[test]
    fn ambiguous_devices_or_profiles_do_not_connect() {
        assert!(choose_profile(
            &[bundled()],
            &ports(&["DDJ-FLX4 #1", "DDJ-FLX4 #2"]),
            &SessionPreference::default()
        )
        .is_none());
        assert!(choose_profile(
            &[profile("a", Some("DJ")), profile("b", Some("DJ"))],
            &ports(&["DJ"]),
            &SessionPreference::default()
        )
        .is_none());
        assert!(choose_profile(
            &[bundled(), profile("other", Some("Other"))],
            &ports(&["DDJ-FLX4", "Other"]),
            &SessionPreference::default()
        )
        .is_none());
    }
    #[test]
    fn specific_hint_wins_on_same_port() {
        let found = choose_profile(
            &[profile("generic", Some("DDJ")), bundled()],
            &ports(&["DDJ-FLX4"]),
            &SessionPreference::default(),
        )
        .unwrap();
        assert_eq!(found.0.id, bundled().id);
    }
    #[test]
    fn remembered_hint_follows_renamed_port_but_never_incompatible_exact_port() {
        let pref = SessionPreference {
            profile: Some(bundled()),
            port: Some("Launchkey".into()),
        };
        let found =
            choose_profile(&[], &ports(&["Launchkey", "USB ddj-flx4 MIDI"]), &pref).unwrap();
        assert_eq!(found.1, "USB ddj-flx4 MIDI");
        assert!(choose_profile(&[], &ports(&["Launchkey"]), &pref).is_none());
    }
    #[test]
    fn disconnected_remembered_device_allows_unambiguous_known_device() {
        let pref = SessionPreference {
            profile: Some(profile("old", Some("Missing"))),
            port: Some("Missing".into()),
        };
        assert_eq!(
            choose_profile(&[bundled()], &ports(&["DDJ-FLX4"]), &pref)
                .unwrap()
                .1,
            "DDJ-FLX4"
        );
    }
    #[test]
    fn empty_hints_require_remembered_exact_port() {
        let p = profile("custom", Some("  "));
        assert!(choose_profile(
            &[p.clone()],
            &ports(&["Keyboard"]),
            &SessionPreference::default()
        )
        .is_none());
        let pref = SessionPreference {
            profile: Some(p),
            port: Some("Keyboard".into()),
        };
        assert!(choose_profile(&[], &ports(&["Keyboard"]), &pref).is_some());
        assert!(choose_profile(&[], &ports(&["Other"]), &pref).is_none());
    }
    #[test]
    fn preference_round_trip_and_malformed_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "opendeck-session-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(load_preference(&dir).profile.is_none());
        let pref = SessionPreference {
            profile: Some(bundled()),
            port: Some("DDJ-FLX4".into()),
        };
        save_preference(&dir, &pref).unwrap();
        assert_eq!(
            serde_json::to_value(load_preference(&dir)).unwrap(),
            serde_json::to_value(pref).unwrap()
        );
        std::fs::write(dir.join(FILE_NAME), b"{broken").unwrap();
        assert!(load_preference(&dir).profile.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
