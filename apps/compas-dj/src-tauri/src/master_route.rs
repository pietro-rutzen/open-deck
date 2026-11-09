//! Speaker output choice. The audio thread opens the device; this module only decides
//! which one, and remembers it. Headphone audio on a DDJ-FLX4 shares that stream.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::headphone;
use crate::AudioRuntimeStatus;

const ROUTE_FILE: &str = "master-route.json";
const APP_SUPPORT_DIR: &str = "Library/Application Support/org.opendeck.prototype";
pub(crate) const NO_EMBEDDED_CUE: u16 = u16::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputChoice {
    pub name: String,
    pub channels: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SavedMaster {
    Unset,
    System,
    Device(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MasterPlan {
    pub open_device: Option<String>,
    pub preferred_device: Option<String>,
    pub embedded_cue_channel: Option<u16>,
    pub fallback_from: Option<String>,
    pub remember: Option<SavedMaster>,
}

#[derive(Clone, Serialize)]
pub(crate) struct MasterSession {
    pub requested_device: Option<String>,
    pub connected_device: Option<String>,
    pub embedded_headphones: bool,
    pub fallback: bool,
    pub online: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct MasterRouteFile {
    device: Option<String>,
}

pub(crate) fn is_flx4(name: &str) -> bool {
    name.to_ascii_lowercase().contains("ddj-flx4")
}

pub(crate) fn plan_master(saved: SavedMaster, devices: &[OutputChoice]) -> MasterPlan {
    match saved {
        SavedMaster::System => MasterPlan {
            open_device: None,
            preferred_device: None,
            embedded_cue_channel: None,
            fallback_from: None,
            remember: None,
        },
        SavedMaster::Device(name) => match unique_device(devices, &name) {
            Some(device) if device.channels >= 1 => open_choice(device),
            _ => MasterPlan {
                open_device: None,
                preferred_device: Some(name.clone()),
                embedded_cue_channel: None,
                fallback_from: Some(name),
                remember: None,
            },
        },
        SavedMaster::Unset => MasterPlan {
            open_device: None,
            preferred_device: None,
            embedded_cue_channel: None,
            fallback_from: None,
            remember: None,
        },
    }
}

fn open_choice(device: &OutputChoice) -> MasterPlan {
    MasterPlan {
        open_device: Some(device.name.clone()),
        preferred_device: Some(device.name.clone()),
        embedded_cue_channel: (is_flx4(&device.name) && device.channels >= 4).then_some(2),
        fallback_from: None,
        remember: None,
    }
}

fn unique_device<'a>(devices: &'a [OutputChoice], name: &str) -> Option<&'a OutputChoice> {
    let mut matches = devices.iter().filter(|device| device.name == name);
    match (matches.next(), matches.next()) {
        (Some(device), None) => Some(device),
        _ => None,
    }
}

pub(crate) fn install_boot(audio: &AudioRuntimeStatus) {
    let plan = boot_plan();
    store_plan(audio, &plan);
    if plan.embedded_cue_channel.is_some() {
        audio.embed_cue_live.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn store_plan(audio: &AudioRuntimeStatus, plan: &MasterPlan) {
    if let Ok(mut requested) = audio.requested_device.lock() {
        *requested = plan.open_device.clone();
    }
    if let Ok(mut preferred) = audio.preferred_device.lock() {
        *preferred = plan.preferred_device.clone();
    }
    if let Ok(mut fallback) = audio.fallback_from.lock() {
        *fallback = plan.fallback_from.clone();
    }
    audio.embedded_cue_channel.store(
        plan.embedded_cue_channel.unwrap_or(NO_EMBEDDED_CUE),
        Ordering::Relaxed,
    );
}

pub(crate) fn session(audio: &AudioRuntimeStatus) -> MasterSession {
    let requested_device = audio
        .preferred_device
        .lock()
        .ok()
        .and_then(|device| device.clone());
    let connected_device = audio
        .master_device
        .lock()
        .ok()
        .and_then(|device| device.clone());
    let fallback = audio
        .fallback_from
        .lock()
        .ok()
        .and_then(|device| device.clone())
        .is_some();
    let online = audio.online.load(Ordering::Relaxed);
    MasterSession {
        requested_device,
        connected_device,
        embedded_headphones: online
            && audio.embedded_active.load(Ordering::Relaxed)
            && audio.embed_cue_live.load(Ordering::Relaxed),
        fallback,
        online,
        error: audio.last_error(),
    }
}

pub(crate) fn output_already_matches(audio: &AudioRuntimeStatus, plan: &MasterPlan) -> bool {
    if !audio.online.load(Ordering::Relaxed) {
        return false;
    }
    let fell_back = audio
        .fallback_from
        .lock()
        .ok()
        .and_then(|device| device.clone())
        .is_some();
    if fell_back {
        return false;
    }
    let requested = audio
        .requested_device
        .lock()
        .ok()
        .and_then(|device| device.clone());
    let embed = audio.embedded_cue_channel.load(Ordering::Relaxed);
    let plan_embed = plan.embedded_cue_channel.unwrap_or(NO_EMBEDDED_CUE);
    requested == plan.open_device && embed == plan_embed
}

pub(crate) fn wait_until_applied(audio: &AudioRuntimeStatus, generation: u64) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(4) {
        let applied = audio.applied_route_generation.load(Ordering::Acquire) >= generation;
        let settled = !audio.restarting.load(Ordering::Relaxed);
        if applied && settled {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn boot_plan() -> MasterPlan {
    let saved = route_path()
        .map(|path| read_saved(&path))
        .unwrap_or(SavedMaster::Unset);
    let plan = plan_master(saved, &listed_outputs());
    if let Some(remember) = &plan.remember {
        if let Some(path) = route_path() {
            if let Err(error) = write_saved(&path, remember) {
                tracing::warn!("could not remember speaker output: {error}");
            }
        }
    }
    plan
}

fn listed_outputs() -> Vec<OutputChoice> {
    compas_audio::output_device_details()
        .into_iter()
        .map(|device| OutputChoice {
            name: device.name,
            channels: device.channels,
        })
        .collect()
}

fn route_path() -> Option<PathBuf> {
    Some(app_data_dir()?.join(ROUTE_FILE))
}

fn app_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(APP_SUPPORT_DIR))
}

fn read_saved(path: &std::path::Path) -> SavedMaster {
    let Ok(file) = std::fs::File::open(path) else {
        return SavedMaster::Unset;
    };
    let mut bytes = Vec::new();
    let Ok(_) = file.take(16_385).read_to_end(&mut bytes) else {
        return SavedMaster::Unset;
    };
    if bytes.len() > 16_384 {
        return SavedMaster::Unset;
    }
    parse_saved(&bytes)
}

fn parse_saved(bytes: &[u8]) -> SavedMaster {
    let Ok(route) = serde_json::from_slice::<MasterRouteFile>(bytes) else {
        return SavedMaster::Unset;
    };
    match route.device {
        None => SavedMaster::System,
        Some(name) if name.trim().is_empty() || name.len() > 4096 => SavedMaster::Unset,
        Some(name) => SavedMaster::Device(name),
    }
}

fn write_saved(path: &std::path::Path, saved: &SavedMaster) -> Result<(), String> {
    let device = match saved {
        SavedMaster::Unset => return Ok(()),
        SavedMaster::System => None,
        SavedMaster::Device(name) => Some(name.clone()),
    };
    let parent = path.parent().ok_or("Speaker route path has no directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".master-route-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes =
        serde_json::to_vec(&MasterRouteFile { device }).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error: std::io::Error| error.to_string())
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn choice_for_request(
    device: Option<String>,
    devices: &[OutputChoice],
) -> Result<SavedMaster, String> {
    let Some(name) = device else {
        return Ok(SavedMaster::System);
    };
    if name.trim().is_empty() || name.len() > 4096 {
        return Err("Choose a speaker output".into());
    }
    let Some(output) = unique_device(devices, &name) else {
        return Err("That speaker output is missing or its name is ambiguous".into());
    };
    if output.channels < 1 {
        return Err("That speaker output has no channels".into());
    }
    Ok(SavedMaster::Device(output.name.clone()))
}

#[tauri::command]
pub(crate) fn master_session(state: State<'_, crate::EngineHandle>) -> MasterSession {
    session(&state.audio)
}

#[tauri::command]
pub(crate) fn set_master_output(
    app: AppHandle,
    device: Option<String>,
) -> Result<MasterSession, String> {
    let audio = Arc::clone(&app.state::<crate::EngineHandle>().audio);
    let devices = listed_outputs();
    let saved = choice_for_request(device, &devices)?;
    let plan = plan_master(saved.clone(), &devices);
    let path = route_path().ok_or("Speaker route directory is unavailable")?;
    write_saved(&path, &saved)?;
    let headphones_were_enabled = headphone::cue_enabled(&app).unwrap_or(false);
    if output_already_matches(&audio, &plan) {
        store_plan(&audio, &plan);
        return Ok(session(&audio));
    }
    store_plan(&audio, &plan);
    let generation = audio.route_generation.fetch_add(1, Ordering::Release) + 1;
    if !wait_until_applied(&audio, generation) {
        return Err("Speaker output did not finish switching".into());
    }
    if let Some(loads) = app.try_state::<crate::deck_loads::DeckLoads>() {
        let _ = loads.invalidate_all();
    }
    if let Some(performance) = app.try_state::<crate::native_controls::PerformanceState>() {
        for deck in 0..compas_audio::NUM_DECKS {
            performance.clear_deck(deck);
        }
    }
    let _ = app.emit("audio:output-reset", ());
    if let Err(error) = headphone::rearm_after_master_change(&app, headphones_were_enabled) {
        tracing::warn!("speakers changed; headphones were not rearmed: {error}");
        headphone::report(&app, &error);
    }
    Ok(session(&audio))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flx4() -> OutputChoice {
        OutputChoice {
            name: "DDJ-FLX4".into(),
            channels: 4,
        }
    }

    fn laptop() -> OutputChoice {
        OutputChoice {
            name: "MacBook Pro Speakers".into(),
            channels: 2,
        }
    }

    fn interface() -> OutputChoice {
        OutputChoice {
            name: "Audient iD4".into(),
            channels: 4,
        }
    }

    #[test]
    fn a_new_setup_follows_the_mac_output_while_the_controller_is_connected() {
        let plan = plan_master(SavedMaster::Unset, &[laptop(), flx4()]);
        assert_eq!(plan.open_device, None);
        assert_eq!(plan.preferred_device, None);
        assert_eq!(plan.embedded_cue_channel, None);
        assert_eq!(plan.remember, None);
    }

    #[test]
    fn saved_choices_keep_laptop_system_and_external_outputs_distinct() {
        let devices = [laptop(), flx4(), interface()];
        let laptop_plan = plan_master(SavedMaster::Device(laptop().name), &devices);
        assert_eq!(
            laptop_plan.open_device.as_deref(),
            Some("MacBook Pro Speakers")
        );
        assert_eq!(laptop_plan.embedded_cue_channel, None);

        let system_plan = plan_master(SavedMaster::System, &devices);
        assert_eq!(system_plan.open_device, None);
        assert_eq!(system_plan.preferred_device, None);
        assert_eq!(system_plan.embedded_cue_channel, None);

        let interface_plan = plan_master(SavedMaster::Device(interface().name), &devices);
        assert_eq!(interface_plan.open_device.as_deref(), Some("Audient iD4"));
        assert_eq!(interface_plan.embedded_cue_channel, None);
    }

    #[test]
    fn startup_without_audio_devices_or_flx4_uses_the_system_output() {
        let fresh = plan_master(SavedMaster::Unset, &[]);
        assert_eq!(fresh.open_device, None);
        assert_eq!(fresh.fallback_from, None);
        assert_eq!(fresh.embedded_cue_channel, None);
        let remembered = plan_master(SavedMaster::Device("DDJ-FLX4".into()), &[]);
        assert_eq!(remembered.open_device, None);
        assert_eq!(remembered.preferred_device.as_deref(), Some("DDJ-FLX4"));
        assert_eq!(remembered.fallback_from.as_deref(), Some("DDJ-FLX4"));
        assert_eq!(remembered.embedded_cue_channel, None);
        assert_eq!(remembered.remember, None);
    }

    #[test]
    fn a_missing_saved_controller_falls_back_without_forgetting_it() {
        let plan = plan_master(SavedMaster::Device("DDJ-FLX4".into()), &[laptop()]);
        assert_eq!(plan.open_device, None);
        assert_eq!(plan.preferred_device.as_deref(), Some("DDJ-FLX4"));
        assert_eq!(plan.fallback_from.as_deref(), Some("DDJ-FLX4"));
        assert_eq!(plan.embedded_cue_channel, None);
    }

    #[test]
    fn ambiguous_or_absent_controllers_stay_on_the_system_output() {
        let first = OutputChoice {
            name: "DDJ-FLX4".into(),
            channels: 4,
        };
        let second = OutputChoice {
            name: "DDJ-FLX4".into(),
            channels: 4,
        };
        let plan = plan_master(SavedMaster::Unset, &[first, second, laptop()]);
        assert_eq!(plan.open_device, None);
        assert_eq!(plan.remember, None);
    }

    #[test]
    fn route_file_distinguishes_system_device_and_garbage() {
        assert_eq!(parse_saved(b"{\"device\":null}"), SavedMaster::System);
        assert_eq!(
            parse_saved(b"{\"device\":\"DDJ-FLX4\"}"),
            SavedMaster::Device("DDJ-FLX4".into())
        );
        assert_eq!(parse_saved(b"nope"), SavedMaster::Unset);
        assert_eq!(parse_saved(b"{\"device\":\"\"}"), SavedMaster::Unset);
    }
}
