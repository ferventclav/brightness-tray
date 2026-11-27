use windows::Devices::Sensors::LightSensor;
use windows::Win32::Media::MediaFoundation::{
    MFEnumDeviceSources, MFCreateAttributes, MFShutdown, MFStartup,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_VERSION, MFSTARTUP_NOSOCKET,
};
use windows::Win32::System::Com::CoTaskMemFree;

use crate::camera;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmbientCapability {
    LightSensor,
    Webcam,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmbientResult {
    Brightness(u32),
    CameraInUse,
    CameraDenied,
    Unavailable,
}

/// Detects the best available ambient-light source on the current hardware:
/// 1. Dedicated hardware Ambient Light Sensor (ALS) via WinRT `LightSensor`
/// 2. Video capture webcam via Media Foundation
/// 3. None (e.g. desktop PC without ALS or webcam)
pub fn detect_capability() -> AmbientCapability {
    if read_als_lux().is_some() {
        return AmbientCapability::LightSensor;
    }
    if has_any_webcam() {
        return AmbientCapability::Webcam;
    }
    AmbientCapability::None
}

/// Measures ambient room light and returns a perceptual target brightness (12..=100%)
/// or a specific reason if unavailable.
pub fn measure_ambient_brightness() -> AmbientResult {
    if let Some(lux) = read_als_lux() {
        return AmbientResult::Brightness(lux_to_brightness(lux));
    }
    camera::measure_room_brightness()
}

fn read_als_lux() -> Option<f32> {
    let sensor = LightSensor::GetDefault().ok()?;
    let reading = sensor.GetCurrentReading().ok()?;
    let lux = reading.IlluminanceInLux().ok()?;
    if lux.is_finite() && lux >= 0.0 {
        Some(lux)
    } else {
        None
    }
}

fn has_any_webcam() -> bool {
    unsafe {
        if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
            return false;
        }
        let mut found = false;
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_ok() {
            if let Some(attrs) = attrs {
                if attrs
                    .SetGUID(
                        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                    )
                    .is_ok()
                {
                    let mut devices_ptr = std::ptr::null_mut();
                    let mut count = 0u32;
                    if MFEnumDeviceSources(&attrs, &mut devices_ptr, &mut count).is_ok() {
                        if !devices_ptr.is_null() {
                            let devices =
                                std::slice::from_raw_parts_mut(devices_ptr, count as usize);
                            found = devices.iter().any(|opt| {
                                opt.as_ref()
                                    .map(|act| !camera::is_virtual_camera_name(&camera::activate_friendly_name(act)))
                                    .unwrap_or(false)
                            });
                            for dev in devices.iter_mut() {
                                *dev = None;
                            }
                            CoTaskMemFree(Some(devices_ptr as *const _));
                        }
                    }
                }
            }
        }
        let _ = MFShutdown();
        found
    }
}

/// Maps hardware Ambient Light Sensor lux (0..50,000+) to perceptual brightness (12..=100%)
/// using Weber-Fechner log compression + Stevens' Power Law exponent.
pub fn lux_to_brightness(lux: f32) -> u32 {
    let lux = lux.max(0.0);
    let max_lux = 15_000.0_f32;
    let log_norm = ((lux + 1.0).log10() / (max_lux + 1.0).log10()).clamp(0.0, 1.0);
    let perceptual = log_norm.powf(0.75);
    (12.0 + perceptual * 88.0).round().clamp(12.0, 100.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lux_curve() {
        assert_eq!(lux_to_brightness(0.0), 12);
        let dim = lux_to_brightness(10.0);
        let office = lux_to_brightness(400.0);
        let daylight = lux_to_brightness(15_000.0);
        println!("ALS curve: 10lux={dim}%, 400lux={office}%, 15000lux={daylight}%");
        assert!(dim >= 25 && dim <= 45);
        assert!(office >= 60 && office <= 80);
        assert_eq!(daylight, 100);
    }

    #[test]
    fn test_capability_detection() {
        let cap = detect_capability();
        println!("Detected ambient capability: {:?}", cap);
    }
}
