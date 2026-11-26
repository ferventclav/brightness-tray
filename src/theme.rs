use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
};

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub is_dark: bool,
    pub window_bg: D2D1_COLOR_F,
    pub window_border: D2D1_COLOR_F,
    pub card_bg: D2D1_COLOR_F,
    pub card_hover_bg: D2D1_COLOR_F,
    pub card_border: D2D1_COLOR_F,
    pub text_primary: D2D1_COLOR_F,
    pub text_secondary: D2D1_COLOR_F,
    pub icon_color: D2D1_COLOR_F,
    pub slider_track_bg: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub thumb_ring: D2D1_COLOR_F,
    pub divider: D2D1_COLOR_F,
    pub footer_bg: D2D1_COLOR_F,
    pub button_hover_bg: D2D1_COLOR_F,
}

impl ThemeColors {
    pub fn load() -> Self {
        let is_dark = read_is_dark_mode();
        let accent = read_accent_color(is_dark);

        if is_dark {
            // Microsoft Fluent 2 / WinUI 3 Dark Theme tokens
            Self {
                is_dark: true,
                window_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
                window_border: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.085 },
                card_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0512 },
                card_hover_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.075 },
                card_border: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0578 },
                text_primary: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
                text_secondary: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.786 },
                icon_color: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.92 },
                slider_track_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.45 },
                accent,
                thumb_ring: D2D1_COLOR_F { r: 0.27, g: 0.27, b: 0.27, a: 1.0 },
                divider: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.075 },
                footer_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.16 },
                button_hover_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0837 },
            }
        } else {
            // Microsoft Fluent 2 / WinUI 3 Light Theme tokens
            Self {
                is_dark: false,
                window_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
                window_border: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.08 },
                card_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.70 },
                card_hover_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.85 },
                card_border: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0578 },
                text_primary: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.895 },
                text_secondary: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.606 },
                icon_color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.85 },
                slider_track_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.445 },
                accent,
                thumb_ring: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
                divider: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0578 },
                footer_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.035 },
                button_hover_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.05 },
            }
        }
    }
}

fn read_dword_reg(subkey: windows::core::PCWSTR, value_name: windows::core::PCWSTR) -> Option<u32> {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_READ, &mut hkey).is_err() {
            return None;
        }
        let mut data: u32 = 0;
        let mut data_size = std::mem::size_of::<u32>() as u32;
        let mut reg_type = REG_DWORD;
        let res = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut reg_type),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey);
        if res.is_ok() {
            Some(data)
        } else {
            None
        }
    }
}

fn read_is_dark_mode() -> bool {
    let val = read_dword_reg(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("SystemUsesLightTheme"),
    )
    .or_else(|| {
        read_dword_reg(
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
        )
    })
    .unwrap_or(0);
    val == 0
}

fn read_accent_color(is_dark: bool) -> D2D1_COLOR_F {
    // HKCU\Software\Microsoft\Windows\DWM\AccentColor is stored as ABGR
    if let Some(abgr) = read_dword_reg(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor")) {
        let r = (abgr & 0xFF) as f32 / 255.0;
        let g = ((abgr >> 8) & 0xFF) as f32 / 255.0;
        let b = ((abgr >> 16) & 0xFF) as f32 / 255.0;
        // Lighten slightly in dark mode for Win11 AccentLight2 feel
        if is_dark {
            return D2D1_COLOR_F {
                r: (r * 0.65 + 0.35).min(1.0),
                g: (g * 0.65 + 0.35).min(1.0),
                b: (b * 0.65 + 0.35).min(1.0),
                a: 1.0,
            };
        }
        return D2D1_COLOR_F { r, g, b, a: 1.0 };
    }

    // Fallback: Win11 sage/accent
    if is_dark {
        D2D1_COLOR_F { r: 0.68, g: 0.78, b: 0.60, a: 1.0 }
    } else {
        D2D1_COLOR_F { r: 0.0, g: 0.47, b: 0.84, a: 1.0 }
    }
}
