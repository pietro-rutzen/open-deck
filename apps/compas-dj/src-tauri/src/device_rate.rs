//! The sample rate an audio device is already running, without changing it.

#[cfg(target_os = "macos")]
use std::mem::size_of;

#[derive(Clone, Copy)]
pub enum AudioEndpoint {
    Output,
    Input,
}

/// Rate the named device, or the system default for `endpoint`, is already using.
/// `None` means the host could not report one; callers then keep cpal's default format.
pub fn active_sample_rate(endpoint: AudioEndpoint, device_name: Option<&str>) -> Option<u32> {
    active_sample_rate_impl(endpoint, device_name)
}

#[cfg(not(target_os = "macos"))]
fn active_sample_rate_impl(_endpoint: AudioEndpoint, _device_name: Option<&str>) -> Option<u32> {
    None
}

#[cfg(target_os = "macos")]
fn active_sample_rate_impl(endpoint: AudioEndpoint, device_name: Option<&str>) -> Option<u32> {
    let device_id = match device_name {
        Some(name) => named_device(name)?,
        None => default_device(endpoint)?,
    };
    nominal_sample_rate(device_id)
}

#[cfg(target_os = "macos")]
fn default_device(endpoint: AudioEndpoint) -> Option<coreaudio::sys::AudioDeviceID> {
    use coreaudio::sys::{
        kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectSystemObject, AudioDeviceID, AudioObjectGetPropertyData,
        AudioObjectPropertyAddress,
    };

    let selector = match endpoint {
        AudioEndpoint::Output => kAudioHardwarePropertyDefaultOutputDevice,
        AudioEndpoint::Input => kAudioHardwarePropertyDefaultInputDevice,
    };
    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };
    let mut device_id: AudioDeviceID = 0;
    let mut size = size_of::<AudioDeviceID>() as u32;
    let status = unsafe {
        // SAFETY: `address` describes the default input or output device. `size` is the
        // byte length of `device_id`, and the call writes one device id into that slot.
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut device_id as *mut AudioDeviceID as *mut _,
        )
    };
    if coreaudio::Error::from_os_status(status).is_err() || device_id == 0 {
        None
    } else {
        Some(device_id)
    }
}

#[cfg(target_os = "macos")]
fn named_device(name: &str) -> Option<coreaudio::sys::AudioDeviceID> {
    let mut matches = device_ids()
        .into_iter()
        .filter(|device_id| device_name(*device_id).as_deref() == Some(name));
    let device_id = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(device_id)
    }
}

#[cfg(target_os = "macos")]
fn device_ids() -> Vec<coreaudio::sys::AudioDeviceID> {
    use coreaudio::sys::{
        kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMaster,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, AudioDeviceID,
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectPropertyAddress,
    };

    let address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };
    let mut size = 0u32;
    let status = unsafe {
        // SAFETY: `address` describes the hardware device list. The size query writes one u32.
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject,
            &address,
            0,
            std::ptr::null(),
            &mut size,
        )
    };
    if coreaudio::Error::from_os_status(status).is_err() || size == 0 {
        return Vec::new();
    }
    let count = size as usize / size_of::<AudioDeviceID>();
    let mut ids = vec![0; count];
    let status = unsafe {
        // SAFETY: `ids` has `count` slots and `size` is that allocation in bytes. The call
        // writes device ids into `ids` and does not retain the pointer.
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut _,
        )
    };
    if coreaudio::Error::from_os_status(status).is_err() {
        Vec::new()
    } else {
        ids
    }
}

#[cfg(target_os = "macos")]
fn device_name(device_id: coreaudio::sys::AudioDeviceID) -> Option<String> {
    use coreaudio::sys::{
        kAudioDevicePropertyDeviceName, kAudioDevicePropertyScopeOutput,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeInput, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
        AudioObjectPropertyAddress,
    };

    for scope in [
        kAudioDevicePropertyScopeOutput,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeInput,
    ] {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyDeviceName,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let mut size = 0u32;
        let status = unsafe {
            // SAFETY: `address` describes this device's name. The size query writes one u32.
            AudioObjectGetPropertyDataSize(device_id, &address, 0, std::ptr::null(), &mut size)
        };
        if coreaudio::Error::from_os_status(status).is_err() || size == 0 || size > 1024 {
            continue;
        }
        let mut bytes = vec![0u8; size as usize];
        let status = unsafe {
            // SAFETY: `bytes` is `size` bytes long. The call writes a NUL-terminated device
            // name into that buffer and does not retain the pointer.
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                bytes.as_mut_ptr() as *mut _,
            )
        };
        if coreaudio::Error::from_os_status(status).is_err() {
            continue;
        }
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        if let Ok(name) = String::from_utf8(bytes[..end].to_vec()) {
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn nominal_sample_rate(device_id: coreaudio::sys::AudioDeviceID) -> Option<u32> {
    use coreaudio::sys::{
        kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyElementMaster,
        kAudioObjectPropertyScopeGlobal, AudioObjectGetPropertyData, AudioObjectPropertyAddress,
    };

    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyNominalSampleRate,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };
    let mut rate = 0f64;
    let mut size = size_of::<f64>() as u32;
    let status = unsafe {
        // SAFETY: `address` describes the device's current nominal sample rate. `size` is the
        // byte length of `rate`, and the call writes one f64 into that slot.
        AudioObjectGetPropertyData(
            device_id,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut rate as *mut f64 as *mut _,
        )
    };
    if coreaudio::Error::from_os_status(status).is_err() || !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    Some(rate.round() as u32)
}
