use std::os::windows::ffi::OsStrExt;
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};

const RUN_SUBKEY: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const APPROVED_SUBKEY: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("BrightnessTray");

pub fn is_startup_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_READ, &mut hkey).is_err() {
            return false;
        }
        let mut data_size: u32 = 0;
        let res = RegQueryValueExW(hkey, VALUE_NAME, None, None, None, Some(&mut data_size));
        let _ = RegCloseKey(hkey);
        res.is_ok() && data_size > 0
    }
}

pub fn set_startup_enabled(enable: bool) -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_SET_VALUE, &mut hkey).is_err() {
            return false;
        }

        let ok = if enable {
            if let Ok(exe_path) = std::env::current_exe() {
                let quoted = format!("\"{}\"", exe_path.display());
                let wide: Vec<u16> = std::ffi::OsStr::new(&quoted)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let bytes = std::slice::from_raw_parts(
                    wide.as_ptr() as *const u8,
                    wide.len() * std::mem::size_of::<u16>(),
                );
                RegSetValueExW(hkey, VALUE_NAME, 0, REG_SZ, Some(bytes)).is_ok()
            } else {
                false
            }
        } else {
            RegDeleteValueW(hkey, VALUE_NAME).is_ok()
        };

        let _ = RegCloseKey(hkey);

        // Ensure Windows 11 StartupApproved\Run marks the entry enabled/removed
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
    }
}
