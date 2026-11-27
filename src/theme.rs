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
    pub button_subtle_bg: D2D1_COLOR_F,
    pub error_text: D2D1_COLOR_F,
    pub error_bg: D2D1_COLOR_F,
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
                button_subtle_bg: D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.04 },
                error_text: D2D1_COLOR_F { r: 1.0, g: 0.60, b: 0.643, a: 1.0 },
                error_bg: D2D1_COLOR_F { r: 1.0, g: 0.60, b: 0.643, a: 0.14 },
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
                button_subtle_bg: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.03 },
                error_text: D2D1_COLOR_F { r: 0.769, g: 0.169, b: 0.110, a: 1.0 },
                error_bg: D2D1_COLOR_F { r: 0.769, g: 0.169, b: 0.110, a: 0.10 },
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

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance(c: D2D1_COLOR_F) -> f32 {
    0.2126 * srgb_to_linear(c.r) + 0.7152 * srgb_to_linear(c.g) + 0.0722 * srgb_to_linear(c.b)
}

fn contrast_ratio(l1: f32, l2: f32) -> f32 {
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn clamp_accent_contrast(mut c: D2D1_COLOR_F, is_dark: bool) -> D2D1_COLOR_F {
    // Ensure >= 4.6:1 against card/footer surfaces even with pastel or dark Windows accents
    let bg_l = if is_dark { 0.025 } else { 0.82 };
    for _ in 0..16 {
        if contrast_ratio(relative_luminance(c), bg_l) >= 4.6 {
            break;
        }
        if is_dark {
            c.r += (1.0 - c.r) * 0.20;
            c.g += (1.0 - c.g) * 0.20;
            c.b += (1.0 - c.b) * 0.20;
        } else {
            c.r *= 0.82;
            c.g *= 0.82;
            c.b *= 0.82;
        }
    }
    c
}

fn read_accent_color(is_dark: bool) -> D2D1_COLOR_F {
    // HKCU\Software\Microsoft\Windows\DWM\AccentColor is stored as ABGR
    if let Some(abgr) = read_dword_reg(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor")) {
        let r = (abgr & 0xFF) as f32 / 255.0;
        let g = ((abgr >> 8) & 0xFF) as f32 / 255.0;
        let b = ((abgr >> 16) & 0xFF) as f32 / 255.0;
        let base = if is_dark {
            D2D1_COLOR_F {
                r: (r * 0.65 + 0.35).min(1.0),
                g: (g * 0.65 + 0.35).min(1.0),
                b: (b * 0.65 + 0.35).min(1.0),
                a: 1.0,
            }
        } else {
            D2D1_COLOR_F { r, g, b, a: 1.0 }
        };
        return clamp_accent_contrast(base, is_dark);
    }

    // Fallback: Win11 sage (dark) / AccentDark1 (light)
    if is_dark {
        D2D1_COLOR_F { r: 0.68, g: 0.78, b: 0.60, a: 1.0 }
    } else {
        D2D1_COLOR_F { r: 0.0, g: 0.373, b: 0.722, a: 1.0 }
    }
}
