use std::os::windows::ffi::OsStrExt;
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
};

const RUN_SUBKEY: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const APPROVED_SUBKEY: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("BrightnessTray");

/// Reads a HKCU value as raw bytes. `None` when the key or value is missing.
fn read_value(subkey: windows::core::PCWSTR) -> Option<Vec<u8>> {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_READ, &mut hkey).is_err() {
            return None;
        }
        let mut size: u32 = 0;
        let mut kind = REG_VALUE_TYPE::default();
        let mut out = None;
        if RegQueryValueExW(hkey, VALUE_NAME, None, Some(&mut kind), None, Some(&mut size)).is_ok() {
            let mut buf = vec![0u8; size as usize];
            if RegQueryValueExW(
                hkey,
                VALUE_NAME,
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr()),
                Some(&mut size),
            )
            .is_ok()
            {
                buf.truncate(size as usize);
                out = Some(buf);
            }
        }
        let _ = RegCloseKey(hkey);
        out
    }
}

/// Task Manager and Settings > Apps > Startup switch an entry off by writing an odd first byte
/// (03) to StartupApproved\Run and leaving the Run value in place. No value means enabled.
pub fn approved_bytes_enabled(bytes: &[u8]) -> bool {
    bytes.first().map_or(true, |b| b & 1 == 0)
}

/// Path stored in the Run value, without the surrounding quotes.
fn run_entry_path() -> Option<String> {
    let bytes = read_value(RUN_SUBKEY)?;
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let s = String::from_utf16_lossy(&wide);
    let s = s.trim();
    let path = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        s
    };
    (!path.is_empty()).then(|| path.to_string())
}

pub fn is_startup_enabled() -> bool {
    if run_entry_path().is_none() {
        return false;
    }
    read_value(APPROVED_SUBKEY)
        .map(|b| approved_bytes_enabled(&b))
        .unwrap_or(true)
}

/// If the exe was moved, the Run entry points at a file that no longer exists and Windows
/// silently skips it at logon. Point it at this copy instead. Leaves a valid entry alone.
pub fn repair_moved_path() -> bool {
    let Some(path) = run_entry_path() else {
        return false;
    };
    if std::path::Path::new(&path).exists() {
        return false;
    }
    write_run_value()
}

fn write_run_value() -> bool {
    unsafe {
        let Ok(exe_path) = std::env::current_exe() else {
            return false;
        };
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_SET_VALUE, &mut hkey).is_err() {
            return false;
        }
        let quoted = format!("\"{}\"", exe_path.display());
        let wide: Vec<u16> = std::ffi::OsStr::new(&quoted)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let bytes = std::slice::from_raw_parts(
            wide.as_ptr() as *const u8,
            wide.len() * std::mem::size_of::<u16>(),
        );
        let ok = RegSetValueExW(hkey, VALUE_NAME, 0, REG_SZ, Some(bytes)).is_ok();
        let _ = RegCloseKey(hkey);
        ok
    }
}

pub fn set_startup_enabled(enable: bool) -> bool {
    unsafe {
        let ok = if enable {
            write_run_value()
        } else {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_SET_VALUE, &mut hkey).is_err() {
                return false;
            }
            let deleted = RegDeleteValueW(hkey, VALUE_NAME).is_ok();
            let _ = RegCloseKey(hkey);
            deleted
        };

        // Mark the entry approved (or remove it) so Windows 11 honours the toggle
        let mut app_key = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            APPROVED_SUBKEY,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut app_key,
            None,
        )
        .is_ok()
        {
            if enable {
                let approved: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                let _ = RegSetValueExW(app_key, VALUE_NAME, 0, REG_BINARY, Some(&approved));
            } else {
                let _ = RegDeleteValueW(app_key, VALUE_NAME);
            }
            let _ = RegCloseKey(app_key);
        }

        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_query() {
        // Should cleanly query HKCU Run without panicking
        let _ = is_startup_enabled();
        let _ = run_entry_path();
    }

    #[test]
    fn test_startup_approved_flag() {
        assert!(approved_bytes_enabled(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!approved_bytes_enabled(&[3, 0, 0, 0, 0x9a, 0x1b, 0, 0, 0, 0, 0, 0]));
        assert!(approved_bytes_enabled(&[]));
    }
}
