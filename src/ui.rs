use std::f32::consts::PI;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{BOOL, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory,
    ID2D1Factory1, ID2D1RenderTarget, ID2D1SolidColorBrush, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_CAP_STYLE_ROUND,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
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
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
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

pub fn compute_flyout_height_dip(monitor_count: usize) -> f32 {
    let cards = monitor_count.max(1) as f32;
    let gaps = (monitor_count.max(1).saturating_sub(1)) as f32;
    OUTER_PAD + cards * CARD_HEIGHT + gaps * CARD_GAP + OUTER_PAD + TOOLBAR_HEIGHT
}

pub fn compute_flyout_position(width_px: i32, height_px: i32, dpi: u32) -> (i32, i32) {
    let mut work_area = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work_area as *mut _ as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }

    let margin = ((12.0 * dpi as f32) / 96.0).round() as i32;
    let mut cursor = POINT::default();
    let has_cursor = unsafe { GetCursorPos(&mut cursor).is_ok() };

    let x = if has_cursor && cursor.x >= work_area.left && cursor.x <= work_area.right {
        // Center horizontally on cursor if near bottom-right, clamped to work area
        (cursor.x - width_px / 2)
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
    let btn_w = 36.0;
    let btn_h = 30.0;
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
                // AABBGGRR: ~60% dark charcoal in dark mode, ~80% soft white in light mode
                let gradient_color = if is_dark { 0x99202020 } else { 0xCCF2F2F2 };
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
    d2d_factory: ID2D1Factory1,
    d2d_context: ID2D1DeviceContext,
    _dwrite_factory: IDWriteFactory,
    font_title: IDWriteTextFormat,
    font_small: IDWriteTextFormat,
    font_badge: IDWriteTextFormat,
    width_px: u32,
    height_px: u32,
}

impl Renderer {
    pub fn new(hwnd: HWND, width_px: u32, height_px: u32, dpi: f32) -> windows::core::Result<Self> {
        unsafe {
            let mut d3d_device: Option<ID3D11Device> = None;
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                None,
            )?;
            let d3d_device = d3d_device.unwrap();
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
                d2d_factory,
                d2d_context,
                _dwrite_factory: dwrite_factory,
                font_title,
                font_small,
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

    pub fn render(
        &self,
        monitors: &[UiMonitor],
        theme: &ThemeColors,
        hover: HoverTarget,
        dragging_idx: Option<usize>,
        linked: bool,
        auto_brightness: bool,
        show_auto: bool,
        dpi: f32,
    ) -> windows::core::Result<()> {
        let total_h = compute_flyout_height_dip(monitors.len());
        let scale = dpi / 96.0;
        let _w_dip = self.width_px as f32 / scale;

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
            let _brush_btn_hover = rt.CreateSolidColorBrush(&theme.button_hover_bg, None)?;

            let d2d1_factory: ID2D1Factory = self.d2d_factory.cast()?;
            let round_cap = d2d1_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    ..Default::default()
                },
                None,
            )?;

            if monitors.is_empty() {
                let card_rect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: OUTER_PAD,
                        top: OUTER_PAD,
                        right: FLYOUT_WIDTH_DIP - OUTER_PAD,
                        bottom: OUTER_PAD + CARD_HEIGHT,
                    },
                    radiusX: 8.0,
                    radiusY: 8.0,
                };
                ctx.FillRoundedRectangle(&card_rect, &brush_card);
                ctx.DrawRoundedRectangle(&card_rect, &brush_card_border, 1.0, None);

                let msg: Vec<u16> = "No DDC/CI or internal displays detected\0"
                    .encode_utf16()
                    .collect();
                ctx.DrawText(
                    &msg[..msg.len() - 1],
                    &self.font_title,
                    &D2D_RECT_F {
                        left: OUTER_PAD + 16.0,
                        top: OUTER_PAD + 12.0,
                        right: FLYOUT_WIDTH_DIP - OUTER_PAD - 16.0,
                        bottom: OUTER_PAD + CARD_HEIGHT - 12.0,
                    },
                    &brush_text_dim,
                    Default::default(),
                    Default::default(),
                );
            } else {
                for (i, mon) in monitors.iter().enumerate() {
                    let top = OUTER_PAD + (i as f32) * (CARD_HEIGHT + CARD_GAP);
                    let card_rect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: OUTER_PAD,
                            top,
                            right: FLYOUT_WIDTH_DIP - OUTER_PAD,
                            bottom: top + CARD_HEIGHT,
                        },
                        radiusX: 8.0,
                        radiusY: 8.0,
                    };

                    let is_active = hover == HoverTarget::Slider(i) || dragging_idx == Some(i);
                    ctx.FillRoundedRectangle(
                        &card_rect,
                        if is_active { &brush_card_hover } else { &brush_card },
                    );
                    ctx.DrawRoundedRectangle(&card_rect, &brush_card_border, 1.0, None);

                    // Monitor badge icon [1]
                    draw_monitor_badge_icon(
                        ctx,
                        OUTER_PAD + 16.0,
                        top + 17.0,
                        mon.display_number,
                        &brush_icon,
                        &self.font_badge,
                    );

                    // Monitor name
                    let name_w: Vec<u16> = mon.name.encode_utf16().collect();
                    ctx.DrawText(
                        &name_w,
                        &self.font_title,
                        &D2D_RECT_F {
                            left: SLIDER_LEFT,
                            top: top + 12.0,
                            right: SLIDER_RIGHT - 36.0,
                            bottom: top + 36.0,
                        },
                        &brush_text,
                        Default::default(),
                        Default::default(),
                    );

                    // Brightness percentage on right (shown subtly, brighter when dragging/hovering)
                    let pct_str = format!("{}%", mon.brightness);
                    let pct_w: Vec<u16> = pct_str.encode_utf16().collect();
                    ctx.DrawText(
                        &pct_w,
                        &self.font_small,
                        &D2D_RECT_F {
                            left: SLIDER_RIGHT - 40.0,
                            top: top + 12.0,
                            right: SLIDER_RIGHT,
                            bottom: top + 36.0,
                        },
                        if is_active { &brush_text } else { &brush_text_dim },
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
                        &round_cap,
                    );
                    // Filled track
                    ctx.DrawLine(
                        D2D_POINT_2F { x: SLIDER_LEFT, y: slider_y },
                        D2D_POINT_2F { x: thumb_x, y: slider_y },
                        &brush_accent,
                        4.0,
                        &round_cap,
                    );

                    // Thumb outer ring
                    let outer_r = if dragging_idx == Some(i) || hover == HoverTarget::Slider(i) {
                        11.0
                    } else {
                        10.0
                    };
                    let inner_r = if dragging_idx == Some(i) {
                        4.5
                    } else if hover == HoverTarget::Slider(i) {
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
            for (target, r) in button_rects(total_h, show_auto) {
                let is_hovered = hover == target;
                let is_active_toggle = (target == HoverTarget::LinkButton && linked)
                    || (target == HoverTarget::AutoButton && auto_brightness);

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
                            radiusX: 6.0,
                            radiusY: 6.0,
                        },
                        &b,
                    );
                }

                let cx = (r.left + r.right) / 2.0;
                let cy = (r.top + r.bottom) / 2.0;
                let icon_brush = if is_active_toggle { &brush_accent } else { &brush_icon };

                match target {
                    HoverTarget::AutoButton => {
                        let label: Vec<u16> = "Auto".encode_utf16().collect();
                        self.font_small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                        ctx.DrawText(
                            &label,
                            &self.font_small,
                            &r,
                            icon_brush,
                            Default::default(),
                            Default::default(),
                        );
                        self.font_small.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                    }
                    HoverTarget::LinkButton => draw_link_icon(ctx, cx, cy, icon_brush, &round_cap),
                    HoverTarget::RefreshButton => draw_refresh_icon(ctx, cx, cy, icon_brush, &round_cap),
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
    brush: &ID2D1SolidColorBrush,
    cap: &windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle,
) {
    // Draw a ~290-degree arc via short segments + arrowhead
    let r = 6.5;
    let steps = 18;
    let start_angle = -0.35_f32;
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
    // Arrowhead at start_angle
    let tip = D2D_POINT_2F {
        x: cx + r * start_angle.cos(),
        y: cy + r * start_angle.sin(),
    };
    ctx.DrawLine(
        tip,
        D2D_POINT_2F {
            x: tip.x - 3.8,
            y: tip.y - 0.5,
        },
        brush,
        1.4,
        cap,
    );
    ctx.DrawLine(
        tip,
        D2D_POINT_2F {
            x: tip.x + 0.5,
            y: tip.y - 3.8,
        },
        brush,
        1.4,
        cap,
    );
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
