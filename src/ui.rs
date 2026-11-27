use windows::core::{w, Interface, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory,
    ID2D1Factory1, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_NO_SNAP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_FONT_WEIGHT_REGULAR, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

use crate::theme::ThemeColors;

pub const FLYOUT_WIDTH_DIP: f32 = 364.0;
const OUTER_PAD: f32 = 12.0;
const CARD_HEIGHT: f32 = 88.0;
const CARD_GAP: f32 = 8.0;
const TOOLBAR_HEIGHT: f32 = 44.0;
const SLIDER_LEFT: f32 = OUTER_PAD + 46.0;
const SLIDER_RIGHT: f32 = FLYOUT_WIDTH_DIP - OUTER_PAD - 18.0;

#[derive(Clone, Debug)]
pub struct UiMonitor {
    pub id: String,
    pub display_number: u32,
    pub name: String,
    pub brightness: u32,
    pub visual_brightness: f32,
    pub anim_from: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoverTarget {
    None,
    Slider(usize),
    AutoButton,
    LinkButton,
    RefreshButton,
    SettingsButton,
}

pub struct RenderState<'a> {
    pub monitors: &'a [UiMonitor],
    pub theme: &'a ThemeColors,
    pub hover: HoverTarget,
    pub dragging_idx: Option<usize>,
    pub focused_idx: Option<usize>,
    pub linked: bool,
    pub measuring_ambient: bool,
    pub refreshing: bool,
    pub refresh_angle: f32,
    pub show_auto: bool,
    pub footer_hint: Option<&'a str>,
    /// Errors read in primary text so they don't pass for a hover hint
    pub footer_hint_strong: bool,
    pub focused_btn: Option<HoverTarget>,
    pub dpi: f32,
}

/// WinUI ControlCornerRadius: cards and buttons. The window itself keeps the 8 DIP overlay radius.
const CONTROL_RADIUS: f32 = 4.0;

/// WinUI's standard icon size for footer and command buttons
const ICON_SIZE: f32 = 16.0;

// Code points shared by Segoe Fluent Icons and Segoe MDL2 Assets
pub const GLYPH_BRIGHTNESS: u16 = 0xE706;
pub const GLYPH_LINK: u16 = 0xE71B;
pub const GLYPH_REFRESH: u16 = 0xE72C;
pub const GLYPH_SETTINGS: u16 = 0xE713;
pub const GLYPH_WARNING: u16 = 0xE7BA;

const FLUENT_ICONS: PCWSTR = w!("Segoe Fluent Icons");
const MDL2_ASSETS: PCWSTR = w!("Segoe MDL2 Assets");

/// Segoe Fluent Icons ships with Windows 11. Windows 10 only has Segoe MDL2 Assets, which uses
/// the same code points, so fall back to it rather than let DirectWrite draw empty boxes.
pub fn icon_font_family(dwrite: &IDWriteFactory) -> PCWSTR {
    unsafe {
        let mut collection: Option<IDWriteFontCollection> = None;
        if dwrite.GetSystemFontCollection(&mut collection, false).is_ok() {
            if let Some(collection) = collection {
                let mut index = 0u32;
                let mut exists = BOOL(0);
                if collection.FindFamilyName(FLUENT_ICONS, &mut index, &mut exists).is_ok()
                    && exists.as_bool()
                {
                    return FLUENT_ICONS;
                }
            }
        }
    }
    MDL2_ASSETS
}

pub fn monitor_work_area_and_dpi_at(pt: POINT) -> (RECT, u32) {
    unsafe {
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if !hmon.is_invalid() {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                let mut dpi_x = 96u32;
                let mut dpi_y = 96u32;
                let dpi = if GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok()
                    && dpi_x > 0
                {
                    dpi_x
                } else {
                    96
                };
                return (mi.rcWork, dpi);
            }
        }
        let mut work_area = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work_area as *mut _ as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        (work_area, 96)
    }
}

pub fn create_d3d11_device() -> windows::core::Result<ID3D11Device> {
    unsafe {
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        let mut d3d_device: Option<ID3D11Device> = None;
        if D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        )
        .is_err()
        {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                None,
            )?;
        }
        Ok(d3d_device.ok_or_else(|| {
            windows::core::Error::from(windows::Win32::Foundation::E_FAIL)
        })?)
    }
}

pub fn compute_flyout_height_dip(monitor_count: usize) -> f32 {
    let cards = monitor_count.max(1) as f32;
    let gaps = (monitor_count.max(1).saturating_sub(1)) as f32;
    OUTER_PAD + cards * CARD_HEIGHT + gaps * CARD_GAP + OUTER_PAD + TOOLBAR_HEIGHT
}

pub fn compute_flyout_position(anchor: POINT, width_px: i32, height_px: i32, dpi: u32) -> (i32, i32) {
    let (work_area, _) = monitor_work_area_and_dpi_at(anchor);

    let margin = ((12.0 * dpi as f32) / 96.0).round() as i32;
    let x = if anchor.x >= work_area.left && anchor.x <= work_area.right {
        (anchor.x - width_px / 2)
            .max(work_area.left + margin)
            .min(work_area.right - width_px - margin)
    } else {
        work_area.right - width_px - margin
    };

    let y = (work_area.bottom - height_px - margin).max(work_area.top + margin);
    (x, y)
}

pub fn slider_bounds_for_card(card_idx: usize) -> (f32, f32, f32, f32) {
    let card_top = OUTER_PAD + (card_idx as f32) * (CARD_HEIGHT + CARD_GAP);
    let y = card_top + 60.0;
    (SLIDER_LEFT, SLIDER_RIGHT, y - 16.0, y + 16.0)
}

pub fn slider_hit_rect(card_idx: usize) -> D2D_RECT_F {
    let (l, r, t, b) = slider_bounds_for_card(card_idx);
    D2D_RECT_F {
        left: l - 12.0,
        top: t,
        right: r + 12.0,
        bottom: b,
    }
}

pub fn card_index_at(x: f32, y: f32, count: usize) -> Option<usize> {
    if x < OUTER_PAD || x > FLYOUT_WIDTH_DIP - OUTER_PAD {
        return None;
    }
    for i in 0..count {
        let top = OUTER_PAD + (i as f32) * (CARD_HEIGHT + CARD_GAP);
        let bottom = top + CARD_HEIGHT;
        if y >= top && y <= bottom {
            return Some(i);
        }
    }
    None
}

pub fn button_rects(total_height_dip: f32, show_auto: bool) -> Vec<(HoverTarget, D2D_RECT_F)> {
    // 32 DIP is the Windows minimum hit target
    let btn_w = 36.0;
    let btn_h = 32.0;
    let gap = 6.0;
    let top = total_height_dip - TOOLBAR_HEIGHT + (TOOLBAR_HEIGHT - btn_h) / 2.0;
    let right_edge = FLYOUT_WIDTH_DIP - OUTER_PAD;

    let settings_rect = D2D_RECT_F {
        left: right_edge - btn_w,
        top,
        right: right_edge,
        bottom: top + btn_h,
    };
    let refresh_rect = D2D_RECT_F {
        left: settings_rect.left - gap - btn_w,
        top,
        right: settings_rect.left - gap,
        bottom: top + btn_h,
    };
    let link_rect = D2D_RECT_F {
        left: refresh_rect.left - gap - btn_w,
        top,
        right: refresh_rect.left - gap,
        bottom: top + btn_h,
    };
    let auto_rect = D2D_RECT_F {
        left: OUTER_PAD,
        top,
        right: OUTER_PAD + 56.0,
        bottom: top + btn_h,
    };

    let mut btns = Vec::with_capacity(4);
    if show_auto {
        btns.push((HoverTarget::AutoButton, auto_rect));
    }
    btns.push((HoverTarget::LinkButton, link_rect));
    btns.push((HoverTarget::RefreshButton, refresh_rect));
    btns.push((HoverTarget::SettingsButton, settings_rect));
    btns
}

pub fn hit_test(
    x: f32,
    y: f32,
    monitor_count: usize,
    total_height_dip: f32,
    show_auto: bool,
) -> HoverTarget {
    for (target, r) in button_rects(total_height_dip, show_auto) {
        if x >= r.left && x <= r.right && y >= r.top && y <= r.bottom {
            return target;
        }
    }
    for i in 0..monitor_count {
        let rect = slider_hit_rect(i);
        if x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom {
            return HoverTarget::Slider(i);
        }
    }
    HoverTarget::None
}

pub fn x_to_brightness_f32(x: f32) -> f32 {
    let t = ((x - SLIDER_LEFT) / (SLIDER_RIGHT - SLIDER_LEFT)).clamp(0.0, 1.0);
    t * 100.0
}

#[cfg(test)]
pub fn x_to_brightness(x: f32) -> u32 {
    x_to_brightness_f32(x).round() as u32
}

pub fn apply_win11_backdrop(hwnd: HWND, is_dark: bool) {
    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32,
        pv_data: *mut core::ffi::c_void,
        cb_data: usize,
    }
    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

    unsafe {
        let dark: BOOL = if is_dark { BOOL(1) } else { BOOL(0) };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );

        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );

        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

        // Use SetWindowCompositionAttribute (ACCENT_ENABLE_ACRYLICBLURBEHIND = 4)
        // because DWMWA_SYSTEMBACKDROP_TYPE does not blur borderless WS_POPUP windows.
        if let Ok(huser32) =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(w!("user32.dll"))
        {
            if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
                huser32,
                windows::core::s!("SetWindowCompositionAttribute"),
            ) {
                let func: SetWindowCompositionAttributeFn = std::mem::transmute(proc);
                // AABBGGRR: 72% charcoal in dark mode preserves blurred wallpaper depth while keeping card text legible over bright windows
                let gradient_color = if is_dark { 0xB81C1C1C } else { 0xCCF2F2F2 };
                let mut policy = AccentPolicy {
                    accent_state: 4, // ACCENT_ENABLE_ACRYLICBLURBEHIND
                    accent_flags: 2,
                    gradient_color,
                    animation_id: 0,
                };
                let mut data = WindowCompositionAttribData {
                    attrib: 19, // WCA_ACCENT_POLICY
                    pv_data: &mut policy as *mut _ as *mut core::ffi::c_void,
                    cb_data: std::mem::size_of::<AccentPolicy>(),
                };
                let _ = func(hwnd, &mut data);
            }
        }
    }
}

pub struct Renderer {
    _d3d_device: ID3D11Device,
    _dcomp_device: Option<IDCompositionDevice>,
    _dcomp_target: Option<IDCompositionTarget>,
    _dcomp_visual: Option<IDCompositionVisual>,
    swap_chain: Option<IDXGISwapChain1>,
    d2d_context: ID2D1DeviceContext,
    round_cap: ID2D1StrokeStyle,
    _dwrite_factory: IDWriteFactory,
    font_title: IDWriteTextFormat,
    font_small: IDWriteTextFormat,
    font_button: IDWriteTextFormat,
    font_status: IDWriteTextFormat,
    font_note: IDWriteTextFormat,
    font_badge: IDWriteTextFormat,
    font_icon: IDWriteTextFormat,
    font_icon_sm: IDWriteTextFormat,
    width_px: u32,
    height_px: u32,
}

impl Renderer {
    fn create_core(width_px: u32, height_px: u32, dpi: f32) -> windows::core::Result<(ID3D11Device, IDXGIDevice, Self)> {
        unsafe {
            let d3d_device = create_d3d11_device()?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;

            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device: ID2D1Device = d2d_factory.CreateDevice(&dxgi_device)?;
            let d2d_context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            d2d_context.SetDpi(dpi, dpi);

            let d2d1_factory: ID2D1Factory = d2d_factory.cast()?;
            let round_cap = d2d1_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    ..Default::default()
                },
                None,
            )?;

            let dwrite_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let font_title = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                14.0,
                w!("en-US"),
            )?;
            font_title.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            font_title.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            font_title.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            if let Ok(ellipsis) = dwrite_factory.CreateEllipsisTrimmingSign(&font_title) {
                let _ = font_title.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    Some(&ellipsis),
                );
            }

            let font_small = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_MEDIUM,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                w!("en-US"),
            )?;
            font_small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            font_small.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let font_button = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_MEDIUM,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                w!("en-US"),
            )?;
            font_button.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            font_button.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let font_status = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                11.5,
                w!("en-US"),
            )?;
            font_status.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            font_status.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            font_status.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            if let Ok(ellipsis) = dwrite_factory.CreateEllipsisTrimmingSign(&font_status) {
                let _ = font_status.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    Some(&ellipsis),
                );
            }

            // Left-aligned secondary line for the empty state, under a left-aligned title
            let font_note = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                w!("en-US"),
            )?;
            font_note.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            font_note.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let font_badge = dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                9.5,
                w!("en-US"),
            )?;
            font_badge.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            font_badge.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let icon_family = icon_font_family(&dwrite_factory);
            let font_icon = dwrite_factory.CreateTextFormat(
                icon_family,
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                ICON_SIZE,
                w!("en-US"),
            )?;
            font_icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            font_icon.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            font_icon.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

            let font_icon_sm = dwrite_factory.CreateTextFormat(
                icon_family,
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                w!("en-US"),
            )?;
            font_icon_sm.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            font_icon_sm.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            font_icon_sm.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

            let renderer = Self {
                _d3d_device: d3d_device.clone(),
                _dcomp_device: None,
                _dcomp_target: None,
                _dcomp_visual: None,
                swap_chain: None,
                d2d_context,
                round_cap,
                _dwrite_factory: dwrite_factory,
                font_title,
                font_small,
                font_button,
                font_status,
                font_note,
                font_badge,
                font_icon,
                font_icon_sm,
                width_px,
                height_px,
            };
            Ok((d3d_device, dxgi_device, renderer))
        }
    }

    pub fn new(hwnd: HWND, width_px: u32, height_px: u32, dpi: f32) -> windows::core::Result<Self> {
        unsafe {
            let (_, dxgi_device, mut renderer) = Self::create_core(width_px, height_px, dpi)?;
            let adapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;

            let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width_px.max(1),
                Height: height_px.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: BOOL(0),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: windows::Win32::Graphics::Dxgi::DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };

            let swap_chain = dxgi_factory.CreateSwapChainForComposition(&dxgi_device, &swap_desc, None)?;

            let dcomp_device: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let dcomp_target = dcomp_device.CreateTargetForHwnd(hwnd, true)?;
            let dcomp_visual = dcomp_device.CreateVisual()?;
            dcomp_visual.SetContent(&swap_chain)?;
            dcomp_target.SetRoot(&dcomp_visual)?;
            dcomp_device.Commit()?;

            renderer._dcomp_device = Some(dcomp_device);
            renderer._dcomp_target = Some(dcomp_target);
            renderer._dcomp_visual = Some(dcomp_visual);
            renderer.swap_chain = Some(swap_chain);
            renderer.bind_backbuffer(dpi)?;
            Ok(renderer)
        }
    }

    fn bind_backbuffer(&mut self, dpi: f32) -> windows::core::Result<()> {
        let Some(swap_chain) = &self.swap_chain else {
            return Ok(());
        };
        unsafe {
            self.d2d_context.SetTarget(None);
            let surface: IDXGISurface = swap_chain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bitmap: ID2D1Bitmap1 = self.d2d_context.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.d2d_context.SetTarget(&bitmap);
            self.d2d_context.SetDpi(dpi, dpi);
            Ok(())
        }
    }

    pub fn resize(&mut self, width_px: u32, height_px: u32, dpi: f32) -> windows::core::Result<()> {
        if self.width_px == width_px && self.height_px == height_px {
            unsafe {
                self.d2d_context.SetDpi(dpi, dpi);
            }
            return Ok(());
        }
        let Some(swap_chain) = &self.swap_chain else {
            return Ok(());
        };
        unsafe {
            self.d2d_context.SetTarget(None);
            swap_chain.ResizeBuffers(
                2,
                width_px.max(1),
                height_px.max(1),
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
            self.width_px = width_px;
            self.height_px = height_px;
            self.bind_backbuffer(dpi)?;
        }
        Ok(())
    }

    pub fn render(&self, rs: &RenderState) -> windows::core::Result<()> {
        unsafe {
            let ctx = &self.d2d_context;
            ctx.BeginDraw();
            ctx.Clear(Some(&rs.theme.window_bg));
            self.draw_contents(ctx, rs)?;
            ctx.EndDraw(None, None)?;
            if let Some(swap_chain) = &self.swap_chain {
                swap_chain.Present(0, DXGI_PRESENT(0)).ok()?;
            }
        }
        Ok(())
    }

    unsafe fn draw_contents(
        &self,
        ctx: &ID2D1DeviceContext,
        rs: &RenderState,
    ) -> windows::core::Result<()> {
        let total_h = compute_flyout_height_dip(rs.monitors.len());
        let scale = rs.dpi / 96.0;
        let theme = rs.theme;

        let rt: ID2D1RenderTarget = ctx.cast()?;
        let brush_card = rt.CreateSolidColorBrush(&theme.card_bg, None)?;
        let brush_card_hover = rt.CreateSolidColorBrush(&theme.card_hover_bg, None)?;
        let brush_card_border = rt.CreateSolidColorBrush(&theme.card_border, None)?;
        let brush_text = rt.CreateSolidColorBrush(&theme.text_primary, None)?;
        let brush_text_dim = rt.CreateSolidColorBrush(&theme.text_secondary, None)?;
        let brush_icon = rt.CreateSolidColorBrush(&theme.icon_color, None)?;
        let brush_track = rt.CreateSolidColorBrush(&theme.slider_track_bg, None)?;
        let brush_accent = rt.CreateSolidColorBrush(&theme.accent, None)?;
        let brush_ring = rt.CreateSolidColorBrush(&theme.thumb_ring, None)?;
        let brush_divider = rt.CreateSolidColorBrush(&theme.divider, None)?;
        let brush_focus = rt.CreateSolidColorBrush(&theme.accent, None)?;

        if rs.monitors.is_empty() {
            let card_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: OUTER_PAD,
                    top: OUTER_PAD,
                    right: FLYOUT_WIDTH_DIP - OUTER_PAD,
                    bottom: OUTER_PAD + CARD_HEIGHT,
                },
                radiusX: CONTROL_RADIUS,
                radiusY: CONTROL_RADIUS,
            };
            ctx.FillRoundedRectangle(&card_rect, &brush_card);
            ctx.DrawRoundedRectangle(&card_rect, &brush_card_border, 1.0, None);

            let msg: Vec<u16> = "No controllable displays found".encode_utf16().collect();
            ctx.DrawText(
                &msg,
                &self.font_title,
                &D2D_RECT_F {
                    left: OUTER_PAD + 16.0,
                    top: OUTER_PAD + 12.0,
                    right: FLYOUT_WIDTH_DIP - OUTER_PAD - 16.0,
                    bottom: OUTER_PAD + CARD_HEIGHT - 28.0,
                },
                &brush_text,
                Default::default(),
                Default::default(),
            );

            // "DDC/CI" stays: it is the exact name of the switch in monitor menus
            let sub_msg: Vec<u16> = "Turn on DDC/CI in your monitor's menu, then Refresh"
                .encode_utf16()
                .collect();
            ctx.DrawText(
                &sub_msg,
                &self.font_note,
                &D2D_RECT_F {
                    left: OUTER_PAD + 16.0,
                    top: OUTER_PAD + 44.0,
                    right: FLYOUT_WIDTH_DIP - OUTER_PAD - 16.0,
                    bottom: OUTER_PAD + CARD_HEIGHT - 10.0,
                },
                &brush_text_dim,
                Default::default(),
                Default::default(),
            );
        } else {
            for (i, mon) in rs.monitors.iter().enumerate() {
                let top = OUTER_PAD + (i as f32) * (CARD_HEIGHT + CARD_GAP);
                let card_rect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: OUTER_PAD,
                        top,
                        right: FLYOUT_WIDTH_DIP - OUTER_PAD,
                        bottom: top + CARD_HEIGHT,
                    },
                    radiusX: CONTROL_RADIUS,
                    radiusY: CONTROL_RADIUS,
                };

                let is_active = rs.hover == HoverTarget::Slider(i) || rs.dragging_idx == Some(i);
                let is_focused = rs.focused_idx == Some(i);
                ctx.FillRoundedRectangle(
                    &card_rect,
                    if is_active || is_focused { &brush_card_hover } else { &brush_card },
                );
                if is_focused {
                    ctx.DrawRoundedRectangle(&card_rect, &brush_focus, 1.5, None);
                } else {
                    ctx.DrawRoundedRectangle(&card_rect, &brush_card_border, 1.0, None);
                }

                // Monitor badge icon [1]
                draw_monitor_badge_icon(
                    ctx,
                    OUTER_PAD + 16.0,
                    top + 17.0,
                    mon.display_number,
                    &brush_icon,
                    &self.font_badge,
                );

                // Monitor name (single-line with ellipsis trimming)
                let name_w: Vec<u16> = mon.name.encode_utf16().collect();
                ctx.DrawText(
                    &name_w,
                    &self.font_title,
                    &D2D_RECT_F {
                        left: SLIDER_LEFT,
                        top: top + 12.0,
                        right: SLIDER_RIGHT - 44.0,
                        bottom: top + 36.0,
                    },
                    &brush_text,
                    Default::default(),
                    Default::default(),
                );

                // Brightness percentage on right
                let pct_str = format!("{}%", mon.brightness);
                let pct_w: Vec<u16> = pct_str.encode_utf16().collect();
                ctx.DrawText(
                    &pct_w,
                    &self.font_small,
                    &D2D_RECT_F {
                        left: SLIDER_RIGHT - 46.0,
                        top: top + 12.0,
                        right: SLIDER_RIGHT,
                        bottom: top + 36.0,
                    },
                    if is_active || is_focused { &brush_text } else { &brush_text_dim },
                    Default::default(),
                    Default::default(),
                );

                draw_glyph(
                    ctx,
                    &self.font_icon,
                    GLYPH_BRIGHTNESS,
                    OUTER_PAD + 25.0,
                    top + 60.0,
                    scale,
                    0.0,
                    &brush_icon,
                );

                // Slider track + thumb
                let slider_y = top + 60.0;
                let t = (mon.visual_brightness / 100.0).clamp(0.0, 1.0);
                let thumb_x = SLIDER_LEFT + t * (SLIDER_RIGHT - SLIDER_LEFT);

                // Background track
                ctx.DrawLine(
                    D2D_POINT_2F { x: thumb_x, y: slider_y },
                    D2D_POINT_2F { x: SLIDER_RIGHT, y: slider_y },
                    &brush_track,
                    4.0,
                    &self.round_cap,
                );
                // Filled track
                ctx.DrawLine(
                    D2D_POINT_2F { x: SLIDER_LEFT, y: slider_y },
                    D2D_POINT_2F { x: thumb_x, y: slider_y },
                    &brush_accent,
                    4.0,
                    &self.round_cap,
                );

                // Thumb outer ring
                let outer_r = if rs.dragging_idx == Some(i) || rs.hover == HoverTarget::Slider(i) || is_focused {
                    11.0
                } else {
                    10.0
                };
                let inner_r = if rs.dragging_idx == Some(i) {
                    4.5
                } else if rs.hover == HoverTarget::Slider(i) || is_focused {
                    6.5
                } else {
                    5.5
                };

                ctx.FillEllipse(
                    &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: thumb_x, y: slider_y },
                        radiusX: outer_r,
                        radiusY: outer_r,
                    },
                    &brush_ring,
                );
                ctx.FillEllipse(
                    &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: thumb_x, y: slider_y },
                        radiusX: inner_r,
                        radiusY: inner_r,
                    },
                    &brush_accent,
                );
            }
        }

        // Bottom toolbar footer surface & hairline divider (Fluent 2 flyout pattern)
        let div_y = total_h - TOOLBAR_HEIGHT;
        let brush_footer = rt.CreateSolidColorBrush(&theme.footer_bg, None)?;
        ctx.FillRectangle(
            &D2D_RECT_F {
                left: 0.0,
                top: div_y,
                right: FLYOUT_WIDTH_DIP,
                bottom: total_h,
            },
            &brush_footer,
        );
        ctx.DrawLine(
            D2D_POINT_2F { x: 0.0, y: div_y },
            D2D_POINT_2F { x: FLYOUT_WIDTH_DIP, y: div_y },
            &brush_divider,
            1.0,
            None,
        );

        // Toolbar buttons
        let buttons = button_rects(total_h, rs.show_auto);
        let hint_left = if rs.show_auto { OUTER_PAD + 60.0 } else { OUTER_PAD + 4.0 };
        let hint_right = buttons
            .iter()
            .find(|(t, _)| *t == HoverTarget::LinkButton)
            .map(|(_, r)| r.left - 6.0)
            .unwrap_or(FLYOUT_WIDTH_DIP - OUTER_PAD - 120.0);

        if let Some(hint) = rs.footer_hint {
            if hint_right > hint_left + 20.0 {
                let hint_w: Vec<u16> = hint.encode_utf16().collect();
                if rs.footer_hint_strong {
                    let err_rect = D2D_RECT_F {
                        left: hint_left,
                        top: div_y + 7.0,
                        right: hint_right,
                        bottom: total_h - 7.0,
                    };
                    let err_rr = D2D1_ROUNDED_RECT {
                        rect: err_rect,
                        radiusX: CONTROL_RADIUS,
                        radiusY: CONTROL_RADIUS,
                    };
                    let brush_err_bg = rt.CreateSolidColorBrush(&theme.error_bg, None)?;
                    let err_border = D2D1_COLOR_F {
                        a: 0.35,
                        ..theme.error_text
                    };
                    let brush_err_border = rt.CreateSolidColorBrush(&err_border, None)?;
                    let brush_err = rt.CreateSolidColorBrush(&theme.error_text, None)?;
                    ctx.FillRoundedRectangle(&err_rr, &brush_err_bg);
                    ctx.DrawRoundedRectangle(&err_rr, &brush_err_border, 1.0, None);

                    let warn_w: [u16; 1] = [GLYPH_WARNING];
                    let icon_cx = (hint_left + 13.0) * scale;
                    let icon_l = (icon_cx - 6.0).round() / scale;
                    let icon_t = ((div_y + TOOLBAR_HEIGHT * 0.5) * scale - 6.0).round() / scale;
                    ctx.DrawText(
                        &warn_w,
                        &self.font_icon_sm,
                        &D2D_RECT_F {
                            left: icon_l,
                            top: icon_t,
                            right: icon_l + 12.0,
                            bottom: icon_t + 12.0,
                        },
                        &brush_err,
                        Default::default(),
                        Default::default(),
                    );
                    ctx.DrawText(
                        &hint_w,
                        &self.font_status,
                        &D2D_RECT_F {
                            left: hint_left + 22.0,
                            top: div_y,
                            right: hint_right - 4.0,
                            bottom: total_h,
                        },
                        &brush_err,
                        Default::default(),
                        Default::default(),
                    );
                } else {
                    ctx.DrawText(
                        &hint_w,
                        &self.font_status,
                        &D2D_RECT_F {
                            left: hint_left,
                            top: div_y,
                            right: hint_right,
                            bottom: total_h,
                        },
                        &brush_text_dim,
                        Default::default(),
                        Default::default(),
                    );
                }
            }
        }

        let link_disabled = rs.monitors.len() <= 1;
        let auto_disabled = rs.monitors.is_empty();
        for (target, r) in buttons {
            let is_disabled = (target == HoverTarget::LinkButton && link_disabled)
                || (target == HoverTarget::AutoButton && auto_disabled);
            let is_hovered = !is_disabled && rs.hover == target;
            let is_active_toggle = !is_disabled
                && ((target == HoverTarget::LinkButton && rs.linked)
                    || (target == HoverTarget::AutoButton && rs.measuring_ambient));
            let is_spinning_refresh = target == HoverTarget::RefreshButton && rs.refreshing;

            if is_hovered || is_active_toggle {
                let bg_color = if is_active_toggle && !is_hovered {
                    D2D1_COLOR_F {
                        r: theme.accent.r,
                        g: theme.accent.g,
                        b: theme.accent.b,
                        a: 0.16,
                    }
                } else if is_active_toggle && is_hovered {
                    D2D1_COLOR_F {
                        r: theme.accent.r,
                        g: theme.accent.g,
                        b: theme.accent.b,
                        a: 0.25,
                    }
                } else {
                    theme.button_hover_bg
                };
                let b = rt.CreateSolidColorBrush(&bg_color, None)?;
                ctx.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: CONTROL_RADIUS,
                        radiusY: CONTROL_RADIUS,
                    },
                    &b,
                );
            } else if target == HoverTarget::AutoButton {
                let subtle = D2D1_COLOR_F {
                    a: if is_disabled { theme.button_subtle_bg.a * 0.5 } else { theme.button_subtle_bg.a },
                    ..theme.button_subtle_bg
                };
                let b = rt.CreateSolidColorBrush(&subtle, None)?;
                let rr = D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: CONTROL_RADIUS,
                    radiusY: CONTROL_RADIUS,
                };
                ctx.FillRoundedRectangle(&rr, &b);
                ctx.DrawRoundedRectangle(&rr, &brush_card_border, 1.0, None);
            }
            if !is_disabled && rs.focused_btn == Some(target) {
                ctx.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: CONTROL_RADIUS,
                        radiusY: CONTROL_RADIUS,
                    },
                    &brush_focus,
                    1.5,
                    None,
                );
            }

            let cx = (r.left + r.right) / 2.0;
            let cy = (r.top + r.bottom) / 2.0;
            let disabled_color = D2D1_COLOR_F {
                a: theme.icon_color.a * 0.35,
                ..theme.icon_color
            };
            let brush_disabled = if is_disabled {
                Some(rt.CreateSolidColorBrush(&disabled_color, None)?)
            } else {
                None
            };
            let icon_brush = if let Some(ref b) = brush_disabled {
                b
            } else if is_active_toggle || is_spinning_refresh {
                &brush_accent
            } else {
                &brush_icon
            };

            match target {
                HoverTarget::AutoButton => {
                    let label: Vec<u16> = "Auto".encode_utf16().collect();
                    ctx.DrawText(
                        &label,
                        &self.font_button,
                        &r,
                        icon_brush,
                        Default::default(),
                        Default::default(),
                    );
                }
                HoverTarget::LinkButton => {
                    draw_glyph(ctx, &self.font_icon, GLYPH_LINK, cx, cy, scale, 0.0, icon_brush)
                }
                HoverTarget::RefreshButton => draw_glyph(
                    ctx,
                    &self.font_icon,
                    GLYPH_REFRESH,
                    cx,
                    cy,
                    scale,
                    rs.refresh_angle,
                    icon_brush,
                ),
                HoverTarget::SettingsButton => draw_glyph(
                    ctx,
                    &self.font_icon,
                    GLYPH_SETTINGS,
                    cx,
                    cy,
                    scale,
                    0.0,
                    icon_brush,
                ),
                _ => {}
            }
        }

        // Outer window border
        let border_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: FLYOUT_WIDTH_DIP - 0.5,
                bottom: total_h - 0.5,
            },
            radiusX: 8.0,
            radiusY: 8.0,
        };
        let brush_win_border = rt.CreateSolidColorBrush(&theme.window_border, None)?;
        ctx.DrawRoundedRectangle(&border_rect, &brush_win_border, 1.0, None);
        Ok(())
    }
}

unsafe fn draw_monitor_badge_icon(
    ctx: &ID2D1DeviceContext,
    x: f32,
    y: f32,
    num: u32,
    brush: &ID2D1SolidColorBrush,
    font: &IDWriteTextFormat,
) {
    let rect = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: x,
            top: y,
            right: x + 18.0,
            bottom: y + 13.0,
        },
        radiusX: 2.0,
        radiusY: 2.0,
    };
    ctx.DrawRoundedRectangle(&rect, brush, 1.25, None);
    // Stand
    ctx.DrawLine(
        D2D_POINT_2F { x: x + 9.0, y: y + 13.0 },
        D2D_POINT_2F { x: x + 9.0, y: y + 16.0 },
        brush,
        1.25,
        None,
    );
    ctx.DrawLine(
        D2D_POINT_2F { x: x + 5.0, y: y + 16.0 },
        D2D_POINT_2F { x: x + 13.0, y: y + 16.0 },
        brush,
        1.25,
        None,
    );

    let s = num.to_string();
    let w: Vec<u16> = s.encode_utf16().collect();
    ctx.DrawText(
        &w,
        font,
        &D2D_RECT_F {
            left: x,
            top: y - 0.5,
            right: x + 18.0,
            bottom: y + 13.0,
        },
        brush,
        Default::default(),
        Default::default(),
    );
}

/// Draws one icon-font glyph with its em box centred on (cx, cy), rotated by `angle_rad` around
/// that centre. Both icon fonts set ascent = em and descent = 0, so the em box is the line box
/// DirectWrite centres. The box corner snaps to whole device pixels so the hinted outlines stay
/// crisp at fractional scales such as 150%.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_glyph(
    ctx: &ID2D1DeviceContext,
    font: &IDWriteTextFormat,
    glyph: u16,
    cx: f32,
    cy: f32,
    scale: f32,
    angle_rad: f32,
    brush: &ID2D1SolidColorBrush,
) {
    let half = ICON_SIZE / 2.0;
    let snap = |v: f32| (v * scale).round() / scale;
    let left = snap(cx - half);
    let top = snap(cy - half);
    let rect = D2D_RECT_F {
        left,
        top,
        right: left + ICON_SIZE,
        bottom: top + ICON_SIZE,
    };

    let spinning = angle_rad != 0.0;
    let mut previous = Matrix3x2::identity();
    if spinning {
        ctx.GetTransform(&mut previous);
        let rotation = Matrix3x2::rotation(angle_rad.to_degrees(), left + half, top + half);
        ctx.SetTransform(&(rotation * previous));
    }
    ctx.DrawText(
        &[glyph],
        font,
        &rect,
        brush,
        // Snapping a rotating glyph's baseline makes it wobble from frame to frame
        if spinning { D2D1_DRAW_TEXT_OPTIONS_NO_SNAP } else { D2D1_DRAW_TEXT_OPTIONS_NONE },
        DWRITE_MEASURING_MODE_NATURAL,
    );
    if spinning {
        ctx.SetTransform(&previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Offscreen Direct2D visual preview dump"]
    fn render_fluent_icon_previews() {
        use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
        use windows::Win32::Graphics::Direct2D::{
            D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_MAP_OPTIONS_READ,
        };

        let out_dir = match std::env::var("PREVIEW_OUT_DIR") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => return,
        };
        let _ = std::fs::create_dir_all(&out_dir);

        let dump = |filename: &str, dpi: f32, rs: &RenderState, w_px: u32, h_px: u32| unsafe {
            let (_, _, renderer) = Renderer::create_core(w_px, h_px, dpi).unwrap();
            let size = D2D_SIZE_U { width: w_px, height: h_px };
            let target_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let target = renderer
                .d2d_context
                .CreateBitmap(size, None, 0, &target_props)
                .unwrap();
            renderer.d2d_context.SetTarget(&target);
            renderer.d2d_context.SetDpi(dpi, dpi);
            renderer.d2d_context.BeginDraw();
            renderer.d2d_context.Clear(Some(&rs.theme.window_bg));
            renderer.draw_contents(&renderer.d2d_context, rs).unwrap();
            renderer.d2d_context.EndDraw(None, None).unwrap();

            let cpu_props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let cpu = renderer
                .d2d_context
                .CreateBitmap(size, None, 0, &cpu_props)
                .unwrap();
            cpu.CopyFromBitmap(None, &target, None).unwrap();
            let mapped = cpu.Map(D2D1_MAP_OPTIONS_READ).unwrap();
            let mut raw = Vec::with_capacity((w_px * h_px * 4) as usize);
            for row in 0..h_px as usize {
                let row_ptr = mapped.bits.add(row * mapped.pitch as usize);
                raw.extend_from_slice(std::slice::from_raw_parts(row_ptr, (w_px * 4) as usize));
            }
            cpu.Unmap().unwrap();
            std::fs::write(out_dir.join(format!("{filename}_{w_px}x{h_px}.bgra")), &raw).unwrap();
        };

        let mut theme = ThemeColors::load();
        theme.window_bg = D2D1_COLOR_F { r: 0.11, g: 0.11, b: 0.11, a: 1.0 };
        let mons = vec![
            UiMonitor { id: "m1".into(), display_number: 1, name: "MAG 27CQ6F".into(), brightness: 51, visual_brightness: 51.0, anim_from: 51.0 },
            UiMonitor { id: "m2".into(), display_number: 2, name: "Built-in display".into(), brightness: 51, visual_brightness: 51.0, anim_from: 51.0 },
        ];
        let h_dip = compute_flyout_height_dip(mons.len());
        for (tag, dpi) in [("100", 96.0_f32), ("150", 144.0_f32)] {
            let w_px = (FLYOUT_WIDTH_DIP * dpi / 96.0).round() as u32;
            let h_px = (h_dip * dpi / 96.0).round() as u32;
            dump(
                &format!("icons_{tag}"),
                dpi,
                &RenderState {
                    monitors: &mons,
                    theme: &theme,
                    hover: HoverTarget::None,
                    dragging_idx: None,
                    focused_idx: None,
                    linked: false,
                    measuring_ambient: false,
                    refreshing: false,
                    refresh_angle: 0.0,
                    show_auto: true,
                    footer_hint: None,
                    footer_hint_strong: false,
                    focused_btn: None,
                    dpi,
                },
                w_px,
                h_px,
            );
        }
        // Linked + focus ring + error + spin frames at 150%
        let dpi = 144.0_f32;
        let w_px = (FLYOUT_WIDTH_DIP * dpi / 96.0).round() as u32;
        let h_px = (h_dip * dpi / 96.0).round() as u32;
        dump(
            "icons_link_focus_150",
            dpi,
            &RenderState {
                monitors: &mons,
                theme: &theme,
                hover: HoverTarget::None,
                dragging_idx: None,
                focused_idx: None,
                linked: true,
                measuring_ambient: false,
                refreshing: false,
                refresh_angle: 0.0,
                show_auto: true,
                footer_hint: Some("Displays linked (L)"),
                footer_hint_strong: false,
                focused_btn: Some(HoverTarget::LinkButton),
                dpi,
            },
            w_px,
            h_px,
        );
        dump(
            "icons_error_150",
            dpi,
            &RenderState {
                monitors: &mons,
                theme: &theme,
                hover: HoverTarget::None,
                dragging_idx: None,
                focused_idx: None,
                linked: false,
                measuring_ambient: false,
                refreshing: false,
                refresh_angle: 0.0,
                show_auto: true,
                footer_hint: Some("Camera in use"),
                footer_hint_strong: true,
                focused_btn: None,
                dpi,
            },
            w_px,
            h_px,
        );
        for k in 0..4 {
            dump(
                &format!("icons_spin{k}_150"),
                dpi,
                &RenderState {
                    monitors: &mons,
                    theme: &theme,
                    hover: HoverTarget::None,
                    dragging_idx: None,
                    focused_idx: None,
                    linked: false,
                    measuring_ambient: false,
                    refreshing: true,
                    refresh_angle: k as f32 * 0.48,
                    show_auto: true,
                    footer_hint: Some("Refreshing displays..."),
                    footer_hint_strong: false,
                    focused_btn: None,
                    dpi,
                },
                w_px,
                h_px,
            );
        }
    }

    #[test]
    fn icon_fonts_carry_every_glyph() {
        // Flyout glyphs, then the settings menu's (main.rs) and its check mark (menu.rs)
        let glyphs = [
            GLYPH_BRIGHTNESS,
            GLYPH_LINK,
            GLYPH_REFRESH,
            GLYPH_SETTINGS,
            GLYPH_WARNING,
            0xE7E8,
            0xE770,
            0xE711,
            0xE73E,
        ];
        unsafe {
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let mut collection: Option<IDWriteFontCollection> = None;
            dwrite.GetSystemFontCollection(&mut collection, false).unwrap();
            let collection = collection.unwrap();

            let mut installed = Vec::new();
            for family in [FLUENT_ICONS, MDL2_ASSETS] {
                let mut index = 0u32;
                let mut exists = BOOL(0);
                collection.FindFamilyName(family, &mut index, &mut exists).unwrap();
                if !exists.as_bool() {
                    continue;
                }
                let font = collection
                    .GetFontFamily(index)
                    .unwrap()
                    .GetFirstMatchingFont(
                        DWRITE_FONT_WEIGHT_REGULAR,
                        DWRITE_FONT_STRETCH_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                    )
                    .unwrap();
                for g in glyphs {
                    assert!(
                        font.HasCharacter(g as u32).unwrap().as_bool(),
                        "{} lacks U+{:04X}",
                        family.to_string().unwrap(),
                        g
                    );
                }
                installed.push(family.to_string().unwrap());
            }
            assert!(!installed.is_empty(), "no icon font installed");
            let picked = icon_font_family(&dwrite).to_string().unwrap();
            assert_eq!(picked, installed[0], "should prefer the first installed family");
        }
    }

    #[test]
    fn test_flyout_layout_and_hit_testing() {
        let h1 = compute_flyout_height_dip(1);
        let h2 = compute_flyout_height_dip(2);
        assert!(h2 > h1);
        assert_eq!(compute_flyout_height_dip(0), h1);

        assert_eq!(x_to_brightness(SLIDER_LEFT), 0);
        assert_eq!(x_to_brightness(SLIDER_RIGHT), 100);
        assert_eq!(x_to_brightness(0.0), 0);
        assert_eq!(x_to_brightness(999.0), 100);
        assert_eq!(x_to_brightness((SLIDER_LEFT + SLIDER_RIGHT) / 2.0), 50);

        let (l, r, t, b) = slider_bounds_for_card(1);
        let mid_x = (l + r) / 2.0;
        let mid_y = (t + b) / 2.0;
        assert_eq!(hit_test(mid_x, mid_y, 2, h2, true), HoverTarget::Slider(1));
        assert_eq!(card_index_at(mid_x, mid_y, 2), Some(1));
        assert_eq!(card_index_at(2.0, mid_y, 2), None);

        for (expected, rect) in button_rects(h2, true) {
            let bx = (rect.left + rect.right) / 2.0;
            let by = (rect.top + rect.bottom) / 2.0;
            assert_eq!(hit_test(bx, by, 2, h2, true), expected);
        }
        assert_eq!(button_rects(h2, false).len(), 3);
    }
}
