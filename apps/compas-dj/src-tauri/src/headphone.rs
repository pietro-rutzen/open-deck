//! Native headphone route/session owner. Device work never runs on audio/MIDI callbacks.
use super::{EngineHandle, EngineMsg, CUE_RING_CAPACITY};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const ROUTE_FILE: &str = "headphone-route.json";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
pub(crate) struct CueSession {
    pub enabled: bool,
    pub device: Option<String>,
    pub first_channel: u16,
    pub connected_name: Option<String>,
    pub mix: f32,
    pub volume: f32,
    pub cued: Vec<usize>,
    pub error: Option<String>,
}
impl Default for CueSession {
    fn default() -> Self {
        Self {
            enabled: false,
            device: None,
            first_channel: 0,
            connected_name: None,
            mix: 0.0,
            volume: 0.8,
            cued: Vec::new(),
            error: None,
        }
    }
}
#[derive(Default)]
struct Inner {
    session: CueSession,
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
    generation: u64,
}
pub(crate) struct CueState {
    operation: Mutex<()>,
    inner: Mutex<Inner>,
    controller_tx: Mutex<Option<SyncSender<(String, f64)>>>,
    pub latency: Arc<compas_audio::MonitorLatency>,
}
impl Default for CueState {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            inner: Mutex::new(Inner::default()),
            controller_tx: Mutex::new(None),
            latency: Arc::new(compas_audio::MonitorLatency::default()),
        }
    }
}
impl CueState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}
#[derive(Serialize, Deserialize)]
struct Route {
    device: String,
    first_channel: u16,
}
#[derive(Serialize)]
pub(crate) struct DeviceInfo {
    name: String,
    channels: u16,
    sample_rate: u32,
}

fn snapshot(app: &AppHandle) -> Result<CueSession, String> {
    app.state::<CueState>()
        .inner
        .lock()
        .map(|s| s.session.clone())
        .map_err(|e| e.to_string())
}
fn publish(app: &AppHandle) {
    if let Ok(s) = snapshot(app) {
        let _ = app.emit("cue:session", s);
    }
}
pub(crate) fn report(app: &AppHandle, error: &str) {
    if let Ok(mut inner) = app.state::<CueState>().inner.lock() {
        inner.session.error = Some(error.to_owned());
    }
    publish(app);
}
pub(crate) fn cue_enabled(app: &AppHandle) -> Result<bool, String> {
    snapshot(app).map(|session| session.enabled)
}
fn feedback(app: &AppHandle, deck: usize, active: bool) {
    app.state::<super::controllers::ControllerEngine>().send(
        super::controllers::ControllerMsg::Feedback {
            control: format!("deck.{deck}.headphone_cue"),
            value: if active { 1.0 } else { 0.0 },
        },
    );
}
fn validate_route(device: &str, first: u16) -> Result<(), String> {
    if device.trim().is_empty() || device.len() > 4096 {
        return Err("Choose a valid headphone audio device".into());
    }
    let matches: Vec<_> = compas_audio::output_device_details()
        .into_iter()
        .filter(|d| d.name == device)
        .collect();
    if matches.len() != 1 {
        return Err("Headphone audio device is missing or its name is ambiguous".into());
    }
    if first
        .checked_add(1)
        .is_none_or(|right| right >= matches[0].channels)
    {
        return Err("Headphone stereo pair is outside this device's output channels".into());
    }
    Ok(())
}
fn validate_safe_route(app: &AppHandle, route: &Route) -> Result<(), String> {
    validate_route(&route.device, route.first_channel)?;
    let engine = app.state::<EngineHandle>();
    let master = engine
        .audio
        .master_device
        .lock()
        .map_err(|e| e.to_string())?;
    if route.first_channel < 2 && master.as_deref() == Some(route.device.as_str()) {
        return Err("Headphone channels overlap the active master output; choose a different device or stereo pair".into());
    }
    Ok(())
}
fn load_route(app: &AppHandle) -> Option<Route> {
    let path = app.path().app_data_dir().ok()?.join(ROUTE_FILE);
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 16_384 {
        return None;
    }
    let route: Route = serde_json::from_slice(&bytes).ok()?;
    if route.device.trim().is_empty()
        || route.device.len() > 4096
        || route.first_channel == u16::MAX
    {
        return None;
    }
    Some(route)
}
fn save_route(app: &AppHandle, route: &Route) -> Result<(), String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let temporary = base.join(format!(
        ".headphone-route-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = serde_json::to_vec(route).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, base.join(ROUTE_FILE))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}
fn default_route() -> Option<Route> {
    let matches: Vec<_> = compas_audio::output_device_details()
        .into_iter()
        .filter(|d| d.name.to_lowercase().contains("ddj-flx4") && d.channels >= 4)
        .collect();
    (matches.len() == 1).then(|| Route {
        device: matches[0].name.clone(),
        first_channel: 2,
    })
}
fn selected_route(app: &AppHandle) -> Result<Route, String> {
    let current = snapshot(app)?;
    current
        .device
        .map(|device| Route {
            device,
            first_channel: current.first_channel,
        })
        .or_else(default_route)
        .ok_or_else(|| {
            "Choose the controller's headphone audio device and stereo channels in Settings".into()
        })
}

/// Setup-time preference loading is synchronous, before frontend commands can run.
/// Only the known FLX4 gets a suggested 3/4 pair; generic devices require selection.
pub(crate) fn initialize(app: &AppHandle) {
    if let Some(route) = load_route(app).or_else(default_route) {
        if let Ok(mut inner) = app.state::<CueState>().inner.lock() {
            inner.session.device = Some(route.device);
            inner.session.first_channel = route.first_channel;
        }
    }
    let (tx, rx) = sync_channel::<(String, f64)>(64);
    if let Ok(mut sender) = app.state::<CueState>().controller_tx.lock() {
        *sender = Some(tx);
    }
    let controller_app = app.clone();
    std::thread::spawn(move || {
        while let Ok((control, value)) = rx.recv() {
            let mut batch = vec![(control, value)];
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
            }
            if let Err(error) = apply_controller_batch(&controller_app, &rx, batch) {
                report(&controller_app, &error);
            }
        }
    });
    arm_saved_headphones(app);
}
fn arm_saved_headphones(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        for _ in 0..40 {
            if !app
                .state::<EngineHandle>()
                .audio
                .online
                .load(Ordering::Relaxed)
            {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            let cue = app.state::<CueState>();
            let Ok(_operation) = cue.operation.lock() else {
                return;
            };
            if snapshot(&app)
                .map(|session| session.enabled)
                .unwrap_or(true)
            {
                return;
            }
            let Ok(route) = selected_route(&app) else {
                return;
            };
            match start_locked(&app, &route) {
                Ok(_) => publish(&app),
                Err(error) => report(&app, &error),
            }
            return;
        }
    });
}
fn cue_deck(control: &str) -> Option<usize> {
    control
        .strip_prefix("deck.")?
        .strip_suffix(".headphone_cue")?
        .parse::<usize>()
        .ok()
        .filter(|deck| *deck < 4)
}
/// Called on the existing controller worker. Bounded enqueue avoids blocking it on CPAL.
pub(crate) fn controller_update(app: &AppHandle, control: &str, value: f64) -> bool {
    if control != "mixer.cue_mix" && control != "mixer.cue_volume" && cue_deck(control).is_none() {
        return false;
    }
    if !value.is_finite() {
        report(app, "Invalid headphone controller value");
        return true;
    }
    let result = app
        .state::<CueState>()
        .controller_tx
        .lock()
        .map_err(|e| e.to_string())
        .and_then(|tx| {
            tx.as_ref()
                .ok_or_else(|| "Headphone controller worker is unavailable".to_string())?
                .try_send((control.to_owned(), value))
                .map_err(|e| e.to_string())
        });
    if let Err(error) = result {
        report(app, &error);
    }
    true
}

// Caller owns operation mutex. The previous monitor thread is joined before return so a new
// stream cannot open while the old one still holds the device.
fn stop_locked(app: &AppHandle, leds: bool) -> Result<(), String> {
    let (stop, worker, old_cued) = {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
        inner.generation = inner.generation.wrapping_add(1);
        let old_cued = std::mem::take(&mut inner.session.cued);
        inner.session.enabled = false;
        inner.session.connected_name = None;
        (inner.stop.take(), inner.worker.take(), old_cued)
    };
    app.state::<EngineHandle>()
        .audio
        .embed_cue_live
        .store(false, Ordering::Relaxed);
    if let Some(stop) = stop {
        let _ = stop.send(());
    }
    if let Some(worker) = worker {
        if worker.thread().id() != std::thread::current().id() {
            let _ = worker.join();
        }
    }
    app.state::<EngineHandle>().send(EngineMsg::StopCueOutput)?;
    for deck in old_cued {
        app.state::<EngineHandle>().send(EngineMsg::DeckCue {
            deck,
            active: false,
        })?;
        if leds {
            feedback(app, deck, false);
        }
    }
    Ok(())
}
fn release_monitor_worker(app: &AppHandle) -> Result<(), String> {
    let (stop, worker) = {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|error| error.to_string())?;
        inner.generation = inner.generation.wrapping_add(1);
        (inner.stop.take(), inner.worker.take())
    };
    if let Some(stop) = stop {
        let _ = stop.send(());
    }
    if let Some(worker) = worker {
        if worker.thread().id() != std::thread::current().id() {
            let _ = worker.join();
        }
    }
    app.state::<EngineHandle>().send(EngineMsg::StopCueOutput)
}
fn stream_embeds(app: &AppHandle, route: &Route) -> bool {
    let audio = &app.state::<EngineHandle>().audio;
    if !audio.online.load(Ordering::Relaxed) || !audio.embedded_active.load(Ordering::Relaxed) {
        return false;
    }
    if audio.embedded_cue_channel.load(Ordering::Relaxed) != route.first_channel {
        return false;
    }
    audio
        .master_device
        .lock()
        .ok()
        .and_then(|name| name.clone())
        .as_deref()
        == Some(route.device.as_str())
}
fn enable_embedded(app: &AppHandle, route: &Route) -> Result<String, String> {
    validate_safe_route(app, route)?;
    let desired = snapshot(app)?;
    release_monitor_worker(app)?;
    if let Err(error) = replay_monitor_state(app, &desired) {
        let _ = stop_locked(app, true);
        return Err(error);
    }
    app.state::<EngineHandle>()
        .audio
        .embed_cue_live
        .store(true, Ordering::Relaxed);
    {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|error| error.to_string())?;
        inner.session.device = Some(route.device.clone());
        inner.session.first_channel = route.first_channel;
        inner.session.enabled = true;
        inner.session.connected_name = Some(route.device.clone());
        inner.session.error = None;
    }
    if let Err(error) = save_route(app, route) {
        report(
            app,
            &format!("Headphones active; could not remember routing: {error}"),
        );
    }
    Ok(route.device.clone())
}
// Reapply the native session after a fresh mixer is created. Replay false PFL
// states too so a reused mixer cannot retain a cue absent from the session.
fn send_monitor_state(
    desired: &CueSession,
    mut send: impl FnMut(EngineMsg) -> Result<(), String>,
) -> Result<(), String> {
    send(EngineMsg::CueMix(desired.mix))?;
    send(EngineMsg::CueVolume(desired.volume))?;
    for (deck, active) in remember_cued(desired).into_iter().enumerate() {
        send(EngineMsg::DeckCue { deck, active })?;
    }
    Ok(())
}
fn restore_monitor_session(app: &AppHandle, desired: &CueSession) -> Result<(), String> {
    let cued = remember_cued(desired);
    {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|error| error.to_string())?;
        inner.session.mix = desired.mix;
        inner.session.volume = desired.volume;
        inner.session.cued = cued
            .iter()
            .enumerate()
            .filter_map(|(deck, active)| active.then_some(deck))
            .collect();
    }
    for (deck, active) in cued.into_iter().enumerate() {
        feedback(app, deck, active);
    }
    Ok(())
}
fn replay_monitor_state(app: &AppHandle, desired: &CueSession) -> Result<(), String> {
    send_monitor_state(desired, |message| app.state::<EngineHandle>().send(message))?;
    restore_monitor_session(app, desired)
}
pub(crate) fn rearm_after_master_change(
    app: &AppHandle,
    headphones_were_enabled: bool,
) -> Result<(), String> {
    let cue = app.state::<CueState>();
    let _operation = cue.operation.lock().map_err(|error| error.to_string())?;
    if !headphones_were_enabled {
        app.state::<EngineHandle>()
            .audio
            .embed_cue_live
            .store(false, Ordering::Relaxed);
        publish(app);
        return Ok(());
    }
    let route = selected_route(app)?;
    start_locked(app, &route)?;
    publish(app);
    Ok(())
}
fn start_locked(app: &AppHandle, route: &Route) -> Result<String, String> {
    validate_safe_route(app, route)?;
    if stream_embeds(app, route) {
        return enable_embedded(app, route);
    }
    let engine = app.state::<EngineHandle>();
    if !engine.audio.online.load(Ordering::Relaxed) {
        return Err("Master audio engine is offline; headphones could not start".into());
    }
    let source_rate = engine.audio.sample_rate.load(Ordering::Relaxed);
    let source_generation = engine.audio.restarts.load(Ordering::Relaxed);
    if source_rate == 0 {
        return Err("Master audio sample rate is unavailable".into());
    }
    let desired = snapshot(app)?;
    stop_locked(app, true)?;
    let cue = app.state::<CueState>();
    let generation = cue.inner.lock().map_err(|e| e.to_string())?.generation;
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(CUE_RING_CAPACITY);
    engine.send(EngineMsg::StartCueOutput { sink: producer })?;
    send_monitor_state(&desired, |message| engine.send(message))?;
    let (stop_tx, stop_rx) = channel();
    let (ready_tx, ready_rx) = channel();
    let worker_app = app.clone();
    let device = route.device.clone();
    let first = route.first_channel;
    let latency = cue.latency.clone();
    let source_route = engine.audio.route_generation.load(Ordering::Relaxed);
    let spawn = std::thread::Builder::new()
        .name("opendeck-headphones".into())
        .spawn(move || {
            let active_rate =
                crate::device_rate::active_sample_rate(crate::device_rate::AudioEndpoint::Output, Some(&device));
            match compas_audio::open_cue_output_route_with_latency(
                &device,
                first,
                source_rate,
                active_rate,
                consumer,
                latency,
            ) {
                Ok(output) => {
                    let _ = ready_tx.send(Ok(output.device_name.clone()));
                    let failure = loop {
                        match stop_rx.recv_timeout(Duration::from_millis(200)) {
                            Ok(()) | Err(RecvTimeoutError::Disconnected) => break None,
                            Err(RecvTimeoutError::Timeout) => {
                                let engine = worker_app.state::<EngineHandle>();
                                if engine.audio.route_generation.load(Ordering::Relaxed) != source_route {
                                    break None;
                                }
                                if !engine.audio.online.load(Ordering::Relaxed)
                                    || engine.audio.sample_rate.load(Ordering::Relaxed) != source_rate
                                    || engine.audio.restarts.load(Ordering::Relaxed) != source_generation
                                {
                                    break Some("Master audio changed or restarted; press headphone Cue to reconnect the monitor bus".into());
                                }
                                if output.failed() {
                                    break Some(output.last_error().unwrap_or_else(|| {
                                        "Headphone output stream failed".into()
                                    }));
                                }
                            }
                        }
                    };
                    drop(output);
                    if let Some(error) = failure {
                        let cue = worker_app.state::<CueState>();
                        if let Ok(_operation) = cue.operation.try_lock() {
                            let current = cue
                                .inner
                                .lock()
                                .map(|inner| inner.generation == generation)
                                .unwrap_or(false);
                            if current {
                                let _ = stop_locked(&worker_app, true);
                                report(&worker_app, &error);
                            }
                        };
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                }
            }
        });
    let worker = match spawn {
        Ok(worker) => worker,
        Err(error) => {
            let _ = stop_locked(app, true);
            return Err(format!("Could not create headphone worker: {error}"));
        }
    };
    {
        let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
        inner.worker = Some(worker);
    }
    match ready_rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(name)) => {
            // The worker is live; reconcile the session and every PFL LED with the
            // state already sent before its stream started.
            restore_monitor_session(app, &desired)?;
            {
                let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
                inner.stop = Some(stop_tx);
                inner.session.device = Some(route.device.clone());
                inner.session.first_channel = first;
                inner.session.enabled = true;
                inner.session.connected_name = Some(name.clone());
                inner.session.error = None;
            }
            if let Err(error) = save_route(app, route) {
                report(
                    app,
                    &format!("Headphones active; could not remember routing: {error}"),
                );
            }
            Ok(name)
        }
        Ok(Err(error)) => {
            let _ = stop_locked(app, true);
            Err(error)
        }
        Err(error) => {
            let _ = stop_locked(app, true);
            Err(format!("Headphone output did not become ready: {error}"))
        }
    }
}
fn with_error<T>(app: &AppHandle, result: Result<T, String>) -> Result<T, String> {
    if let Err(error) = &result {
        report(app, error);
    }
    result
}

#[tauri::command]
pub(crate) fn cue_session(app: AppHandle) -> Result<CueSession, String> {
    snapshot(&app)
}
#[tauri::command]
pub(crate) fn list_output_device_details() -> Vec<DeviceInfo> {
    compas_audio::output_device_details()
        .into_iter()
        .map(|d| DeviceInfo {
            name: d.name,
            channels: d.channels,
            sample_rate: d.sample_rate,
        })
        .collect()
}
#[tauri::command]
pub(crate) fn configure_cue_output(
    app: AppHandle,
    device: Option<String>,
    first_channel: u16,
) -> Result<CueSession, String> {
    let result = (|| {
        let cue = app.state::<CueState>();
        let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
        let route = Route {
            device: device.ok_or("Choose a headphone audio device")?,
            first_channel,
        };
        validate_safe_route(&app, &route)?;
        if snapshot(&app)?.enabled {
            start_locked(&app, &route)?;
        } else {
            save_route(&app, &route)?;
            let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
            inner.session.device = Some(route.device);
            inner.session.first_channel = first_channel;
            inner.session.error = None;
        }
        publish(&app);
        snapshot(&app)
    })();
    with_error(&app, result)
}
#[tauri::command]
pub(crate) fn start_cue_output(app: AppHandle, device: Option<String>) -> Result<String, String> {
    let result = (|| {
        let cue = app.state::<CueState>();
        let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
        let route = if let Some(device) = device {
            let session = snapshot(&app)?;
            let first_channel = if session.device.as_ref() == Some(&device) {
                session.first_channel
            } else {
                default_route()
                    .filter(|route| route.device == device)
                    .map(|route| route.first_channel)
                    .unwrap_or(0)
            };
            Route {
                device,
                first_channel,
            }
        } else {
            selected_route(&app)?
        };
        let name = start_locked(&app, &route)?;
        publish(&app);
        Ok(name)
    })();
    with_error(&app, result)
}
#[tauri::command]
pub(crate) fn stop_cue_output(app: AppHandle) -> Result<(), String> {
    let result = (|| {
        let cue = app.state::<CueState>();
        let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
        stop_locked(&app, true)?;
        cue.inner.lock().map_err(|e| e.to_string())?.session.error = None;
        publish(&app);
        Ok(())
    })();
    with_error(&app, result)
}
fn set_deck_locked(app: &AppHandle, deck: usize, active: bool) -> Result<(), String> {
    if deck >= 4 {
        return Err("Invalid headphone deck".into());
    }
    if active && !snapshot(app)?.enabled {
        start_locked(app, &selected_route(app)?)?;
    }
    app.state::<EngineHandle>()
        .send(EngineMsg::DeckCue { deck, active })?;
    {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
        inner.session.cued.retain(|d| *d != deck);
        if active {
            inner.session.cued.push(deck);
            inner.session.cued.sort_unstable();
        }
        inner.session.error = None;
    }
    feedback(app, deck, active);
    publish(app);
    Ok(())
}
fn fold_cue_press(mut cued: [bool; 4], deck: usize) -> [bool; 4] {
    if let Some(slot) = cued.get_mut(deck) {
        *slot = !*slot;
    }
    cued
}
fn remember_cued(session: &CueSession) -> [bool; 4] {
    let mut cued = [false; 4];
    for deck in &session.cued {
        if let Some(slot) = cued.get_mut(*deck) {
            *slot = true;
        }
    }
    cued
}
fn apply_level_locked(app: &AppHandle, value: f32, volume: bool) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err("Headphone control must be a finite value between 0 and 1".into());
    }
    app.state::<EngineHandle>().send(if volume {
        EngineMsg::CueVolume(value)
    } else {
        EngineMsg::CueMix(value)
    })?;
    {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
        if volume {
            inner.session.volume = value;
        } else {
            inner.session.mix = value;
        }
    }
    Ok(())
}
fn fold_batch(
    mut desired: [bool; 4],
    batch: &[(String, f64)],
    app: &AppHandle,
) -> Result<[bool; 4], String> {
    for (control, value) in batch {
        if control == "mixer.cue_mix" {
            apply_level_locked(app, *value as f32, false)?;
        } else if control == "mixer.cue_volume" {
            apply_level_locked(app, *value as f32, true)?;
        } else if let Some(deck) = cue_deck(control).filter(|_| *value >= 0.5) {
            desired = fold_cue_press(desired, deck);
        }
    }
    Ok(desired)
}
/// One headphone press is a toggle. Presses that arrive while the route is opening are counted
/// before any deck is put on the cue bus, so a second press cancels instead of leaving pre-fader
/// audio on the wrong output.
fn apply_controller_batch(
    app: &AppHandle,
    rx: &Receiver<(String, f64)>,
    batch: Vec<(String, f64)>,
) -> Result<(), String> {
    let cue = app.state::<CueState>();
    let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
    let mut desired = fold_batch(remember_cued(&snapshot(app)?), &batch, app)?;
    let opened_for_this_batch = desired.iter().any(|on| *on) && !snapshot(app)?.enabled;
    if opened_for_this_batch {
        start_locked(app, &selected_route(app)?)?;
        let mut late = Vec::new();
        while let Ok(message) = rx.try_recv() {
            late.push(message);
        }
        desired = fold_batch(desired, &late, app)?;
    }
    if opened_for_this_batch && desired.iter().all(|on| !*on) {
        stop_locked(app, true)?;
        for deck in 0..4 {
            feedback(app, deck, false);
        }
        publish(app);
        return Ok(());
    }
    commit_cued(app, desired)
}
fn commit_cued(app: &AppHandle, desired: [bool; 4]) -> Result<(), String> {
    let current = snapshot(app)?;
    for (deck, on) in desired.into_iter().enumerate() {
        if current.cued.contains(&deck) != on {
            app.state::<EngineHandle>()
                .send(EngineMsg::DeckCue { deck, active: on })?;
            let cue = app.state::<CueState>();
            let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
            inner.session.cued.retain(|candidate| *candidate != deck);
            if on {
                inner.session.cued.push(deck);
            }
            inner.session.error = None;
        }
        feedback(app, deck, on);
    }
    {
        let cue = app.state::<CueState>();
        let mut inner = cue.inner.lock().map_err(|e| e.to_string())?;
        inner.session.cued.sort_unstable();
    }
    publish(app);
    Ok(())
}
#[tauri::command]
pub(crate) fn set_deck_cue(app: AppHandle, deck: usize, active: bool) -> Result<(), String> {
    let result = (|| {
        let cue = app.state::<CueState>();
        let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
        set_deck_locked(&app, deck, active)
    })();
    with_error(&app, result)
}
fn set_parameter(app: &AppHandle, value: f32, volume: bool) -> Result<(), String> {
    let cue = app.state::<CueState>();
    let _operation = cue.operation.lock().map_err(|e| e.to_string())?;
    apply_level_locked(app, value, volume)?;
    publish(app);
    Ok(())
}
#[tauri::command]
pub(crate) fn set_cue_mix(app: AppHandle, value: f32) -> Result<(), String> {
    with_error(&app, set_parameter(&app, value, false))
}
#[tauri::command]
pub(crate) fn set_cue_volume(app: AppHandle, value: f32) -> Result<(), String> {
    with_error(&app, set_parameter(&app, value, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headphone_control_paths_validate_decks() {
        assert_eq!(cue_deck("deck.0.headphone_cue"), Some(0));
        assert_eq!(cue_deck("deck.3.headphone_cue"), Some(3));
        assert_eq!(cue_deck("deck.4.headphone_cue"), None);
        assert_eq!(cue_deck("deck.0.play"), None);
    }
    #[test]
    fn a_second_press_before_the_route_opens_cancels_the_first() {
        assert!(!fold_cue_press(fold_cue_press([false; 4], 0), 0)[0]);
        assert!(fold_cue_press([false; 4], 1)[1]);
        assert!(!fold_cue_press([true, false, false, false], 0)[0]);
    }
    #[test]
    fn session_defaults_are_disabled() {
        let session = CueSession::default();
        assert!(!session.enabled);
        assert!(session.cued.is_empty());
        assert_eq!(session.mix, 0.0);
    }
    #[test]
    fn route_replay_preserves_levels_and_all_pfl_states() {
        let mut desired = CueSession::default();
        desired.mix = 0.25;
        desired.volume = 0.15;
        desired.cued = vec![0, 2];
        let mut messages = Vec::new();
        send_monitor_state(&desired, |message| {
            messages.push(message);
            Ok(())
        })
        .unwrap();
        assert_eq!(messages.len(), 6);
        assert!(matches!(messages[0], EngineMsg::CueMix(v) if v == 0.25));
        assert!(matches!(messages[1], EngineMsg::CueVolume(v) if v == 0.15));
        for (index, message) in messages[2..].iter().enumerate() {
            assert!(matches!(message, EngineMsg::DeckCue { deck, active }
                if *deck == index && *active == (index == 0 || index == 2)));
        }
    }
    #[test]
    fn route_replay_propagates_queue_failure_without_sending_later_controls() {
        let mut sent = 0;
        let result = send_monitor_state(&CueSession::default(), |_| {
            sent += 1;
            if sent == 2 {
                Err("offline".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err("offline".into()));
        assert_eq!(sent, 2);
    }
}
