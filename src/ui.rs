use std::f32::consts::PI;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{BOOL, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory,
    ID2D1Factory1, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
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
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_MEDIUM,
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
        let (l, r, t, b) = slider_bounds_for_card(i);
        if x >= l - 12.0 && x <= r + 12.0 && y >= t && y <= b {
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
                // AABBGGRR: ~45% dark charcoal in dark mode so live wallpaper blur shines through like Win11 menus
                let gradient_color = if is_dark { 0x721C1C1C } else { 0xCCF2F2F2 };
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
    _dcomp_device: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    _dcomp_visual: IDCompositionVisual,
    swap_chain: IDXGISwapChain1,
    d2d_context: ID2D1DeviceContext,
    round_cap: ID2D1StrokeStyle,
    _dwrite_factory: IDWriteFactory,
    font_title: IDWriteTextFormat,
    font_small: IDWriteTextFormat,
    font_button: IDWriteTextFormat,
    font_status: IDWriteTextFormat,
    font_note: IDWriteTextFormat,
    font_badge: IDWriteTextFormat,
    width_px: u32,
    height_px: u32,
}

impl Renderer {
    pub fn new(hwnd: HWND, width_px: u32, height_px: u32, dpi: f32) -> windows::core::Result<Self> {
        unsafe {
            let d3d_device = create_d3d11_device()?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
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

            let mut renderer = Self {
                _d3d_device: d3d_device,
                _dcomp_device: dcomp_device,
                _dcomp_target: dcomp_target,
                _dcomp_visual: dcomp_visual,
                swap_chain,
                d2d_context,
                round_cap,
                _dwrite_factory: dwrite_factory,
                font_title,
                font_small,
                font_button,
                font_status,
                font_note,
                font_badge,
                width_px,
                height_px,
            };
            renderer.bind_backbuffer(dpi)?;
            Ok(renderer)
        }
    }

    fn bind_backbuffer(&mut self, dpi: f32) -> windows::core::Result<()> {
        unsafe {
            self.d2d_context.SetTarget(None);
            let surface: IDXGISurface = self.swap_chain.GetBuffer(0)?;
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
        unsafe {
            self.d2d_context.SetTarget(None);
            self.swap_chain.ResizeBuffers(
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
        let total_h = compute_flyout_height_dip(rs.monitors.len());
        let scale = rs.dpi / 96.0;
        let _w_dip = self.width_px as f32 / scale;
        let theme = rs.theme;

        unsafe {
            let ctx = &self.d2d_context;
            let rt: ID2D1RenderTarget = ctx.cast()?;
            ctx.BeginDraw();
            ctx.Clear(Some(&theme.window_bg));

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
            let focus_color = D2D1_COLOR_F {
                r: theme.accent.r,
                g: theme.accent.g,
                b: theme.accent.b,
                a: 0.78,
            };
            let brush_focus = rt.CreateSolidColorBrush(&focus_color, None)?;

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

                let msg: Vec<u16> = "No controllable displays found"
                    .encode_utf16()
                    .collect();
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

                    // Sun icon
                    draw_sun_icon(ctx, OUTER_PAD + 25.0, top + 60.0, &brush_icon);

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
            let hint_left = if rs.show_auto { OUTER_PAD + 64.0 } else { OUTER_PAD + 4.0 };
            let hint_right = buttons
                .iter()
                .find(|(t, _)| *t == HoverTarget::LinkButton)
                .map(|(_, r)| r.left - 8.0)
                .unwrap_or(FLYOUT_WIDTH_DIP - OUTER_PAD - 120.0);

            if let Some(hint) = rs.footer_hint {
                if hint_right > hint_left + 20.0 {
                    let hint_w: Vec<u16> = hint.encode_utf16().collect();
                    ctx.DrawText(
                        &hint_w,
                        &self.font_status,
                        &D2D_RECT_F {
                            left: hint_left,
                            top: div_y,
                            right: hint_right,
                            bottom: total_h,
                        },
                        if rs.footer_hint_strong { &brush_text } else { &brush_text_dim },
                        Default::default(),
                        Default::default(),
                    );
                }
            }

            for (target, r) in buttons {
                let is_hovered = rs.hover == target;
                let is_active_toggle = (target == HoverTarget::LinkButton && rs.linked)
                    || (target == HoverTarget::AutoButton && rs.measuring_ambient)
                    || (target == HoverTarget::RefreshButton && rs.refreshing);

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
                }
                if rs.focused_btn == Some(target) {
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
                let icon_brush = if is_active_toggle { &brush_accent } else { &brush_icon };

                match target {
                    HoverTarget::AutoButton => {
                        let text = if rs.measuring_ambient { "Auto..." } else { "Auto" };
                        let label: Vec<u16> = text.encode_utf16().collect();
                        ctx.DrawText(
                            &label,
                            &self.font_button,
                            &r,
                            icon_brush,
                            Default::default(),
                            Default::default(),
                        );
                    }
                    HoverTarget::LinkButton => draw_link_icon(ctx, cx, cy, icon_brush, &self.round_cap),
                    HoverTarget::RefreshButton => {
                        draw_refresh_icon(ctx, cx, cy, rs.refresh_angle, icon_brush, &self.round_cap)
                    }
                    HoverTarget::SettingsButton => draw_settings_icon(ctx, cx, cy, icon_brush),
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

            ctx.EndDraw(None, None)?;
            self.swap_chain.Present(0, DXGI_PRESENT(0)).ok()?;
        }

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

unsafe fn draw_sun_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    brush: &ID2D1SolidColorBrush,
) {
    ctx.DrawEllipse(
        &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: 3.4,
            radiusY: 3.4,
        },
        brush,
        1.25,
        None,
    );
    for k in 0..8 {
        let angle = (k as f32) * (PI / 4.0);
        let cos = angle.cos();
        let sin = angle.sin();
        ctx.DrawLine(
            D2D_POINT_2F {
                x: cx + cos * 5.6,
                y: cy + sin * 5.6,
            },
            D2D_POINT_2F {
                x: cx + cos * 7.6,
                y: cy + sin * 7.6,
            },
            brush,
            1.25,
            None,
        );
    }
}

unsafe fn draw_link_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    brush: &ID2D1SolidColorBrush,
    cap: &windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle,
) {
    let left_pill = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - 8.5,
            top: cy - 3.5,
            right: cx - 0.5,
            bottom: cy + 3.5,
        },
        radiusX: 3.5,
        radiusY: 3.5,
    };
    let right_pill = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx + 0.5,
            top: cy - 3.5,
            right: cx + 8.5,
            bottom: cy + 3.5,
        },
        radiusX: 3.5,
        radiusY: 3.5,
    };
    ctx.DrawRoundedRectangle(&left_pill, brush, 1.4, None);
    ctx.DrawRoundedRectangle(&right_pill, brush, 1.4, None);
    ctx.DrawLine(
        D2D_POINT_2F { x: cx - 3.5, y: cy },
        D2D_POINT_2F { x: cx + 3.5, y: cy },
        brush,
        1.4,
        cap,
    );
}

unsafe fn draw_refresh_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    angle_rad: f32,
    brush: &ID2D1SolidColorBrush,
    cap: &ID2D1StrokeStyle,
) {
    // Draw a ~290-degree arc via short segments + rotated arrowhead
    let r = 6.5;
    let steps = 18;
    let start_angle = -0.35_f32 + angle_rad;
    let end_angle = start_angle + 5.1;
    let mut prev = D2D_POINT_2F {
        x: cx + r * start_angle.cos(),
        y: cy + r * start_angle.sin(),
    };
    for s in 1..=steps {
        let a = start_angle + (end_angle - start_angle) * (s as f32 / steps as f32);
        let cur = D2D_POINT_2F {
            x: cx + r * a.cos(),
            y: cy + r * a.sin(),
        };
        ctx.DrawLine(prev, cur, brush, 1.4, cap);
        prev = cur;
    }
    let tip = D2D_POINT_2F {
        x: cx + r * start_angle.cos(),
        y: cy + r * start_angle.sin(),
    };
    let (rot_c, rot_s) = (angle_rad.cos(), angle_rad.sin());
    let rot_offset = |dx: f32, dy: f32| D2D_POINT_2F {
        x: tip.x + dx * rot_c - dy * rot_s,
        y: tip.y + dx * rot_s + dy * rot_c,
    };
    ctx.DrawLine(tip, rot_offset(-3.8, -0.5), brush, 1.4, cap);
    ctx.DrawLine(tip, rot_offset(0.5, -3.8), brush, 1.4, cap);
}

unsafe fn draw_settings_icon(
    ctx: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    brush: &ID2D1SolidColorBrush,
) {
    ctx.DrawEllipse(
        &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: 2.6,
            radiusY: 2.6,
        },
        brush,
        1.35,
        None,
    );
    ctx.DrawEllipse(
        &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: 5.4,
            radiusY: 5.4,
        },
        brush,
        1.35,
        None,
    );
    for k in 0..8 {
        let angle = (k as f32) * (PI / 4.0);
        let cos = angle.cos();
        let sin = angle.sin();
        ctx.DrawLine(
            D2D_POINT_2F {
                x: cx + cos * 5.8,
                y: cy + sin * 5.8,
            },
            D2D_POINT_2F {
                x: cx + cos * 7.5,
                y: cy + sin * 7.5,
            },
            brush,
            1.6,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
