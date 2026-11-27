use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{w, Interface};
use windows::Win32::Media::DirectShow::{CameraControl_Exposure, IAMCameraControl};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaBuffer, IMFMediaSource, IMFSample, IMFSourceReader, MFCreateAttributes,
    MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFShutdown,
    MFStartup, MFVideoFormat_RGB32, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
    MFMediaType_Video, MFSTARTUP_NOSOCKET,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_READ, REG_QWORD, REG_SZ,
};

use crate::ambient::AmbientResult;

/// Checks Windows CapabilityAccessManager ConsentStore to see if any app currently
/// has the webcam open (`LastUsedTimeStop == 0`). If so, we must not touch the camera.
fn is_camera_in_use_by_other_app() -> bool {
    unsafe { check_consent_subkey(w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam")) }
}

/// Checks if Windows camera privacy access is explicitly denied for desktop apps.
fn is_camera_access_denied() -> bool {
    unsafe {
        if check_deny_value(w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam")) {
            return true;
        }
        check_deny_value(w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam\NonPackaged"))
    }
}

unsafe fn check_deny_value(path: windows::core::PCWSTR) -> bool {
    let mut hkey = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, path, 0, KEY_READ, &mut hkey).is_err() {
        return false;
    }
    let mut buf = [0u16; 32];
    let mut size = (buf.len() * std::mem::size_of::<u16>()) as u32;
    let mut reg_type = REG_SZ;
    let denied = RegQueryValueExW(
        hkey,
        w!("Value"),
        None,
        Some(&mut reg_type),
        Some(buf.as_mut_ptr() as *mut u8),
        Some(&mut size),
    )
    .is_ok()
        && {
            let n = (size as usize / 2).min(buf.len());
            let s = String::from_utf16_lossy(&buf[..n]);
            s.trim_end_matches('\0').eq_ignore_ascii_case("Deny")
        };
    let _ = RegCloseKey(hkey);
    denied
}

unsafe fn check_consent_subkey(path: windows::core::PCWSTR) -> bool {
    let mut hkey = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, path, 0, KEY_READ, &mut hkey).is_err() {
        return false;
    }
    let in_use = check_hkey_recursive(hkey, 0);
    let _ = RegCloseKey(hkey);
    in_use
}

unsafe fn check_hkey_recursive(hkey: HKEY, depth: u32) -> bool {
    if depth > 2 {
        return false;
    }
    let mut stop_val: u64 = 1;
    let mut size = std::mem::size_of::<u64>() as u32;
    let mut reg_type = REG_QWORD;
    if RegQueryValueExW(
        hkey,
        w!("LastUsedTimeStop"),
        None,
        Some(&mut reg_type),
        Some(&mut stop_val as *mut u64 as *mut u8),
        Some(&mut size),
    )
    .is_ok()
        && stop_val == 0
    {
        return true;
    }

    let mut idx = 0u32;
    let mut name_buf = [0u16; 256];
    loop {
        let mut name_len = name_buf.len() as u32;
        if RegEnumKeyExW(
            hkey,
            idx,
            windows::core::PWSTR(name_buf.as_mut_ptr()),
            &mut name_len,
            None,
            windows::core::PWSTR::null(),
            None,
            None,
        )
        .is_err()
        {
            break;
        }
        idx += 1;
        let sub_name: Vec<u16> = name_buf[..name_len as usize]
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .collect();
        let mut sub_hkey = HKEY::default();
        if RegOpenKeyExW(
            hkey,
            windows::core::PCWSTR(sub_name.as_ptr()),
            0,
            KEY_READ,
            &mut sub_hkey,
        )
        .is_ok()
        {
            let found = check_hkey_recursive(sub_hkey, depth + 1);
            let _ = RegCloseKey(sub_hkey);
            if found {
                return true;
            }
        }
    }
    false
}

/// Measures room ambient light using the default physical webcam in read-only mode
/// (never taking exclusive control or changing camera settings).
/// Immediately skips if another app is already using the camera or camera access is denied,
/// and shuts down within ~2.8s max.
pub fn measure_room_brightness() -> AmbientResult {
    if is_camera_access_denied() {
        return AmbientResult::CameraDenied;
    }
    if is_camera_in_use_by_other_app() {
        return AmbientResult::CameraInUse;
    }
    unsafe {
        if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
            return AmbientResult::Unavailable;
        }
        let result = measure_inner();
        let _ = MFShutdown();
        match result {
            Some(b) => AmbientResult::Brightness(b),
            None => AmbientResult::Unavailable,
        }
    }
}

pub fn is_virtual_camera_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("virtual")
        || lower.contains("obs ")
        || lower.contains("obs-")
        || lower.contains("nvidia broadcast")
        || lower.contains("snap camera")
        || lower.contains("droidcam")
        || lower.contains("epoccam")
        || lower.contains("iriun")
        || lower.contains("manycam")
}

pub unsafe fn activate_friendly_name(act: &IMFActivate) -> String {
    let mut buf = [0u16; 256];
    let mut len = 0u32;
    if act
        .GetString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut buf, Some(&mut len))
        .is_ok()
    {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(len as usize).min(buf.len());
        String::from_utf16_lossy(&buf[..end])
    } else {
        String::new()
    }
}

unsafe fn measure_inner() -> Option<u32> {
    let mut attrs = None;
    MFCreateAttributes(&mut attrs, 1).ok()?;
    let attrs = attrs?;
    attrs
        .SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )
        .ok()?;

    let mut devices_ptr = std::ptr::null_mut();
    let mut count = 0u32;
    MFEnumDeviceSources(&attrs, &mut devices_ptr, &mut count).ok()?;
    if count == 0 || devices_ptr.is_null() {
        return None;
    }

    let devices = std::slice::from_raw_parts_mut(devices_ptr, count as usize);
    let preferred_idx = devices
        .iter()
        .position(|opt| {
            opt.as_ref()
                .map(|act| !is_virtual_camera_name(&activate_friendly_name(act)))
                .unwrap_or(false)
        })
        .unwrap_or(0);
    let source: Option<IMFMediaSource> = devices[preferred_idx]
        .as_ref()
        .and_then(|act| act.ActivateObject().ok());

    for dev in devices.iter_mut() {
        *dev = None;
    }
    CoTaskMemFree(Some(devices_ptr as *const _));

    let source = source?;

    // Watchdog: if ReadSample stalls for >2.8s, force-shutdown the source so it unblocks and releases immediately
    let finished = Arc::new(AtomicBool::new(false));
    let finished_flag = finished.clone();
    let source_raw = source.as_raw() as usize;
    std::thread::spawn(move || {
        for _ in 0..56 {
            if finished_flag.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if !finished_flag.load(Ordering::Acquire) {
            let raw_ptr = source_raw as *mut core::ffi::c_void;
            if let Some(src) = IMFMediaSource::from_raw_borrowed(&raw_ptr) {
                let _ = src.Shutdown();
            }
        }
    });

    let mut reader_attrs = None;
    let _ = MFCreateAttributes(&mut reader_attrs, 1);
    if let Some(ra) = &reader_attrs {
        let _ = ra.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
    }

    let reader: IMFSourceReader =
        match MFCreateSourceReaderFromMediaSource(&source, reader_attrs.as_ref()) {
            Ok(r) => r,
            Err(_) => {
                finished.store(true, Ordering::Release);
                let _ = source.Shutdown();
                return None;
            }
        };

    if let Ok(mt) = MFCreateMediaType() {
        let _ = mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        let _ = mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32);
        let _ = reader.SetCurrentMediaType(
            MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
            None,
            &mt,
        );
    }

    let (frame_w, frame_h) = reader
        .GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)
        .ok()
        .and_then(|mt| mt.GetUINT64(&MF_MT_FRAME_SIZE).ok())
        .map(|packed| ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize))
        .unwrap_or((640, 480));

    let mut samples = Vec::new();
    let mut valid_frames = 0usize;
    let start = std::time::Instant::now();

    while samples.len() < 4 && start.elapsed() < std::time::Duration::from_millis(2200) {
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        if reader
            .ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            )
            .is_err()
        {
            break;
        }
        if (flags & 0x3) != 0 {
            break;
        }
        if let Some(s) = sample {
            valid_frames += 1;
            // Discard the first 2 warmup frames, then sample the next 4
            if valid_frames >= 3 {
                if let Some(luma) = sample_peripheral_luma_rgb32(&s, frame_w, frame_h) {
                    samples.push(luma);
                }
            }
        }
    }

    let exposure_bias = read_auto_exposure_bias(&source);

    finished.store(true, Ordering::Release);
    drop(reader);
    let _ = source.Shutdown();

    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_luma = samples[samples.len() / 2];

    if let Some(bias) = exposure_bias {
        let combined = (median_luma * 0.45 + bias * 0.55).clamp(0.0, 1.0);
        Some(luma_to_brightness(combined))
    } else {
        Some(luma_to_brightness(median_luma))
    }
}

unsafe fn read_auto_exposure_bias(source: &IMFMediaSource) -> Option<f32> {
    let ctrl = source.cast::<IAMCameraControl>().ok()?;
    let (mut min, mut max, mut step, mut def, mut caps) = (0, 0, 0, 0, 0);
    ctrl.GetRange(
        CameraControl_Exposure.0,
        &mut min,
        &mut max,
        &mut step,
        &mut def,
        &mut caps,
    )
    .ok()?;
    if max <= min {
        return None;
    }
    let (mut cur, mut flags) = (0, 0);
    ctrl.Get(CameraControl_Exposure.0, &mut cur, &mut flags).ok()?;
    let norm = 1.0 - ((cur - min) as f32 / (max - min) as f32).clamp(0.0, 1.0);
    Some(norm * 0.65)
}

unsafe fn sample_peripheral_luma_rgb32(
    sample: &IMFSample,
    width: usize,
    height: usize,
) -> Option<f32> {
    let buffer: IMFMediaBuffer = sample.ConvertToContiguousBuffer().ok()?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut max_len = 0u32;
    let mut cur_len = 0u32;
    buffer.Lock(&mut ptr, Some(&mut max_len), Some(&mut cur_len)).ok()?;

    let luma = if !ptr.is_null() && width > 0 && height > 0 && cur_len as usize >= width * height * 4
    {
        let bytes = std::slice::from_raw_parts(ptr, cur_len as usize);
        let mut sum = 0.0f64;
        let mut count = 0u64;

        let top_band = (height * 30) / 100;
        let side_band = (width * 20) / 100;

        for y in (0..height).step_by(4) {
            let row_offset = y * width * 4;
            for x in (0..width).step_by(4) {
                let in_periphery = y < top_band || x < side_band || x >= (width - side_band);
                if !in_periphery {
                    continue;
                }
                let idx = row_offset + x * 4;
                let b = bytes[idx] as f64;
                let g = bytes[idx + 1] as f64;
                let r = bytes[idx + 2] as f64;
                sum += (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
                count += 1;
            }
        }
        if count > 0 {
            Some((sum / count as f64) as f32)
        } else {
            None
        }
    } else {
        None
    };

    let _ = buffer.Unlock();
    luma
}

/// Maps webcam sRGB luma (0.0..=1.0) to perceptual display brightness (12..=100%)
/// using sRGB-to-linear conversion followed by Weber-Fechner logarithmic compression.
fn luma_to_brightness(luma: f32) -> u32 {
    let luma = luma.clamp(0.0, 1.0);
    let linear = luma.powf(2.2);
    let min_l = 0.00045_f32;
    let max_l = 0.40_f32;
    let k = 60.0_f32;
    let clamped = linear.clamp(min_l, max_l);
    let log_norm = ((1.0 + clamped * k).ln() - (1.0 + min_l * k).ln())
        / ((1.0 + max_l * k).ln() - (1.0 + min_l * k).ln());
    (12.0 + log_norm.clamp(0.0, 1.0) * 88.0).round().clamp(12.0, 100.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_luma_mapping() {
        assert_eq!(luma_to_brightness(0.0), 12);
        assert_eq!(luma_to_brightness(0.02), 12);
        assert_eq!(luma_to_brightness(0.66), 100);
        assert_eq!(luma_to_brightness(0.95), 100);
        let dim = luma_to_brightness(0.10);
        let indoor = luma_to_brightness(0.32);
        let bright = luma_to_brightness(0.50);
        println!("Perceptual curve: dim(0.10)={dim}%, indoor(0.32)={indoor}%, bright(0.50)={bright}%");
        assert!(dim >= 20 && dim <= 45);
        assert!(indoor >= 55 && indoor <= 80);
        assert!(bright >= 80 && bright <= 98);
    }

    #[test]
    fn test_consent_store_check() {
        let _ = is_camera_in_use_by_other_app();
        let _ = is_camera_access_denied();
    }

    #[test]
    fn test_virtual_camera_filter() {
        assert!(is_virtual_camera_name("OBS Virtual Camera"));
        assert!(is_virtual_camera_name("NVIDIA Broadcast"));
        assert!(!is_virtual_camera_name("Integrated Webcam"));
        assert!(!is_virtual_camera_name("Logitech BRIO"));
    }

    #[test]
    #[ignore = "Live webcam hardware diagnostic"]
    fn test_camera_live_probe() {
        println!("in_use={}", is_camera_in_use_by_other_app());
        println!("measure={:?}", measure_room_brightness());
    }
}
