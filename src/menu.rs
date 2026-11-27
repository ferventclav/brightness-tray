use windows::core::{w, Interface};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
    ID2D1RenderTarget, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_REGULAR,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, LoadCursorW,
    PostMessageW, RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow,
    TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST,
    IDC_ARROW, MSG, SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_CREATE, WM_DESTROY,
    WM_GETOBJECT, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::theme::ThemeColors;
use crate::ui::{apply_win11_backdrop, create_d3d11_device, monitor_work_area_and_dpi_at};
use crate::uia::{
    build_menu_items, is_screen_reader_active, is_uia_getobject, return_raw_provider, MenuUia,
    WM_UIA_MENU_FOCUS, WM_UIA_MENU_INVOKE,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const MENU_WIDTH_DIP: f32 = 256.0;
const MENU_PAD: f32 = 2.0;
const ITEM_HEIGHT: f32 = 37.0;
const SEP_HEIGHT: f32 = 1.0;

#[derive(Clone)]
pub enum MenuEntry {
    Item {
        id: usize,
        label: String,
        glyph: u16,
        checked: Option<bool>,
    },
    Separator,
}

struct MenuState {
    hwnd: HWND,
    entries: Vec<MenuEntry>,
    theme: ThemeColors,
    hover_idx: Option<usize>,
    keyboard_focus: bool,
    selected_id: Option<usize>,
    closed: bool,
    tracking_mouse: bool,
    _d3d_device: ID3D11Device,
    d3d_swap: IDXGISwapChain1,
    _dcomp_device: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    _dcomp_visual: IDCompositionVisual,
    d2d_context: ID2D1DeviceContext,
    font_text: IDWriteTextFormat,
    font_icon: IDWriteTextFormat,
    font_check: IDWriteTextFormat,
    total_h_dip: f32,
    uia: MenuUia,
}

fn menu_item_rect(entries: &[MenuEntry], target_idx: usize) -> Option<D2D_RECT_F> {
    let mut y = MENU_PAD;
    for (i, entry) in entries.iter().enumerate() {
        match entry {
            MenuEntry::Item { .. } => {
                let r = D2D_RECT_F {
                    left: MENU_PAD,
                    top: y,
                    right: MENU_WIDTH_DIP - MENU_PAD,
                    bottom: y + ITEM_HEIGHT,
                };
                if i == target_idx {
                    return Some(r);
                }
                y += ITEM_HEIGHT;
            }
            MenuEntry::Separator => {
                y += SEP_HEIGHT;
            }
        }
    }
    None
}

impl MenuState {
    fn dpi(&self) -> u32 {
        unsafe {
            let d = GetDpiForWindow(self.hwnd);
            if d == 0 { 96 } else { d }
        }
    }

    fn item_rect(&self, target_idx: usize) -> Option<D2D_RECT_F> {
        menu_item_rect(&self.entries, target_idx)
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if matches!(entry, MenuEntry::Item { .. }) {
                if let Some(r) = self.item_rect(i) {
                    if x >= r.left && x <= r.right && y >= r.top && y <= r.bottom {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    fn render(&self) {
        unsafe {
            let ctx = &self.d2d_context;
            let Ok(rt) = ctx.cast::<ID2D1RenderTarget>() else {
                return;
            };
            ctx.BeginDraw();
            ctx.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            let Ok(brush_hover) = rt.CreateSolidColorBrush(&self.theme.button_hover_bg, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };
            let Ok(brush_text) = rt.CreateSolidColorBrush(&self.theme.text_primary, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };
            let Ok(brush_icon) = rt.CreateSolidColorBrush(&self.theme.icon_color, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };
            let Ok(brush_accent) = rt.CreateSolidColorBrush(&self.theme.accent, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };
            let Ok(brush_div) = rt.CreateSolidColorBrush(&self.theme.divider, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };
            let Ok(brush_border) = rt.CreateSolidColorBrush(&self.theme.window_border, None) else {
                let _ = ctx.EndDraw(None, None);
                return;
            };

            let mut y = MENU_PAD;
            for (i, entry) in self.entries.iter().enumerate() {
                match entry {
                    MenuEntry::Item { label, glyph, checked, .. } => {
                        let r = D2D_RECT_F {
                            left: MENU_PAD,
                            top: y,
                            right: MENU_WIDTH_DIP - MENU_PAD,
                            bottom: y + ITEM_HEIGHT,
                        };
                        if self.hover_idx == Some(i) {
                            let rr = D2D1_ROUNDED_RECT {
                                rect: r,
                                radiusX: 4.0,
                                radiusY: 4.0,
                            };
                            ctx.FillRoundedRectangle(&rr, &brush_hover);
                            if self.keyboard_focus {
                                ctx.DrawRoundedRectangle(&rr, &brush_accent, 1.5, None);
                            }
                        }

                        // Left icon (16px Fluent icon)
                        let icon_glyph = [*glyph];
                        ctx.DrawText(
                            &icon_glyph,
                            &self.font_icon,
                            &D2D_RECT_F {
                                left: r.left + 12.0,
                                top: r.top,
                                right: r.left + 32.0,
                                bottom: r.bottom,
                            },
                            &brush_icon,
                            Default::default(),
                            Default::default(),
                        );

                        // Label
                        let label_w: Vec<u16> = label.encode_utf16().collect();
                        ctx.DrawText(
                            &label_w,
                            &self.font_text,
                            &D2D_RECT_F {
                                left: r.left + 44.0,
                                top: r.top,
                                right: r.right - 28.0,
                                bottom: r.bottom,
                            },
                            &brush_text,
                            Default::default(),
                            Default::default(),
                        );

                        // Right checkmark if checked
                        if *checked == Some(true) {
                            let chk: [u16; 1] = [0xE73E];
                            ctx.DrawText(
                                &chk,
                                &self.font_check,
                                &D2D_RECT_F {
                                    left: r.right - 26.0,
                                    top: r.top,
                                    right: r.right - 10.0,
                                    bottom: r.bottom,
                                },
                                &brush_text,
                                Default::default(),
                                Default::default(),
                            );
                        }
                        y += ITEM_HEIGHT;
                    }
                    MenuEntry::Separator => {
                        ctx.DrawLine(
                            D2D_POINT_2F { x: MENU_PAD + 8.0, y },
                            D2D_POINT_2F { x: MENU_WIDTH_DIP - MENU_PAD - 8.0, y },
                            &brush_div,
                            1.0,
                            None,
                        );
                        y += SEP_HEIGHT;
                    }
                }
            }

            ctx.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.5,
                        top: 0.5,
                        right: MENU_WIDTH_DIP - 0.5,
                        bottom: self.total_h_dip - 0.5,
                    },
                    radiusX: 8.0,
                    radiusY: 8.0,
                },
                &brush_border,
                1.0,
                None,
            );

            let _ = ctx.EndDraw(None, None);
            let _ = self.d3d_swap.Present(0, DXGI_PRESENT(0));
        }
    }
}

pub fn show_fluent_menu(
    parent: HWND,
    anchor: POINT,
    theme: ThemeColors,
    entries: Vec<MenuEntry>,
) -> Option<usize> {
    unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let class_name = w!("BrightnessTrayFluentMenuClass");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(menu_wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let total_h_dip: f32 = MENU_PAD * 2.0
            + entries
                .iter()
                .map(|e| match e {
                    MenuEntry::Item { .. } => ITEM_HEIGHT,
                    MenuEntry::Separator => SEP_HEIGHT,
                })
                .sum::<f32>();

        let (work_area, mon_dpi) = monitor_work_area_and_dpi_at(anchor);
        let dpi = if mon_dpi != 0 {
            mon_dpi
        } else {
            let d = GetDpiForWindow(parent);
            if d == 0 { 96 } else { d }
        };
        let scale = dpi as f32 / 96.0;
        let w_px = (MENU_WIDTH_DIP * scale).round() as i32;
        let h_px = (total_h_dip * scale).round() as i32;

        let margin = (8.0 * scale).round() as i32;
        let x = (anchor.x - w_px / 2)
            .max(work_area.left + margin)
            .min(work_area.right - w_px - margin);
        let y = (anchor.y - h_px - margin)
            .max(work_area.top + margin)
            .min(work_area.bottom - h_px - margin);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            class_name,
            w!("Menu"),
            WS_POPUP,
            x,
            y,
            w_px,
            h_px,
            parent,
            None,
            hinstance,
            None,
        )
        .ok()?;

        apply_win11_backdrop(hwnd, theme.is_dark);

        let Ok(d3d_device) = create_d3d11_device() else {
            let _ = DestroyWindow(hwnd);
            return None;
        };
        let dxgi_device: IDXGIDevice = d3d_device.cast().ok()?;
        let adapter = dxgi_device.GetAdapter().ok()?;
        let dxgi_factory: IDXGIFactory2 = adapter.GetParent().ok()?;

        let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w_px.max(1) as u32,
            Height: h_px.max(1) as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: windows::Win32::Graphics::Dxgi::DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let d3d_swap = dxgi_factory
            .CreateSwapChainForComposition(&dxgi_device, &swap_desc, None)
            .ok()?;

        let dcomp_device: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device).ok()?;
        let dcomp_target = dcomp_device.CreateTargetForHwnd(hwnd, true).ok()?;
        let dcomp_visual = dcomp_device.CreateVisual().ok()?;
        let _ = dcomp_visual.SetContent(&d3d_swap);
        let _ = dcomp_target.SetRoot(&dcomp_visual);
        let _ = dcomp_device.Commit();

        let d2d_factory: ID2D1Factory1 =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
        let d2d_device: ID2D1Device = d2d_factory.CreateDevice(&dxgi_device).ok()?;
        let d2d_context = d2d_device
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
            .ok()?;

        let surface: IDXGISurface = d3d_swap.GetBuffer(0).ok()?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let bitmap: ID2D1Bitmap1 = d2d_context
            .CreateBitmapFromDxgiSurface(&surface, Some(&props))
            .ok()?;
        d2d_context.SetTarget(&bitmap);
        d2d_context.SetDpi(dpi as f32, dpi as f32);

        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let font_text = dwrite
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                14.0,
                w!("en-US"),
            )
            .ok()?;
        let _ = font_text.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = font_text.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = font_text.SetWordWrapping(
            windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
        );

        let icon_family = crate::ui::icon_font_family(&dwrite);
        let font_icon = dwrite
            .CreateTextFormat(
                icon_family,
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                16.0,
                w!("en-US"),
            )
            .ok()?;
        let _ = font_icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = font_icon.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);

        let font_check = dwrite
            .CreateTextFormat(
                icon_family,
                None,
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0,
                w!("en-US"),
            )
            .ok()?;
        let _ = font_check.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = font_check.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);

        let initial_hover = if is_screen_reader_active() {
            move_menu_selection(&entries, None, 1)
        } else {
            None
        };
        let uia_items = build_menu_items(&entries, |idx| {
            menu_item_rect(&entries, idx).map(|r| (r.left, r.top, r.right, r.bottom))
        });
        let uia = MenuUia::new(hwnd, dpi, MENU_WIDTH_DIP, total_h_dip, uia_items);

        let mut state = Box::new(MenuState {
            hwnd,
            entries,
            theme,
            hover_idx: initial_hover,
            keyboard_focus: initial_hover.is_some(),
            selected_id: None,
            closed: false,
            tracking_mouse: false,
            _d3d_device: d3d_device,
            d3d_swap,
            _dcomp_device: dcomp_device,
            _dcomp_target: dcomp_target,
            _dcomp_visual: dcomp_visual,
            d2d_context,
            font_text,
            font_icon,
            font_check,
            total_h_dip,
            uia,
        });

        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (&mut *state as *mut MenuState) as isize,
        );

        state.render();
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w_px, h_px, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
        state.uia.raise_opened();
        if state.hover_idx.is_some() {
            state.uia.sync_focus(dpi, state.hover_idx);
        }

        let mut msg = MSG::default();
        while !state.closed {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 == 0 {
                // WM_QUIT belongs to the app's main loop: hand it back so Exit still works
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            if got.0 == -1 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        state.uia.raise_closed();
        let _ = return_raw_provider(hwnd, WPARAM(0), LPARAM(0), None);
        state.uia.disconnect();
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DestroyWindow(hwnd);
        let _ = PostMessageW(parent, windows::Win32::UI::WindowsAndMessaging::WM_NULL, WPARAM(0), LPARAM(0));
        state.selected_id
    }
}

unsafe extern "system" fn menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        let _ = cs;
        return LRESULT(0);
    }

    let ptr = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
        as *mut MenuState;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                state.closed = true;
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 {
                0x1B => {
                    // VK_ESCAPE
                    state.closed = true;
                }
                0x26 => {
                    // VK_UP
                    let next = move_menu_selection(&state.entries, state.hover_idx, -1);
                    if next != state.hover_idx || !state.keyboard_focus {
                        state.hover_idx = next;
                        state.keyboard_focus = true;
                        state.render();
                        state.uia.sync_focus(state.dpi(), state.hover_idx);
                    }
                }
                0x28 => {
                    // VK_DOWN
                    let next = move_menu_selection(&state.entries, state.hover_idx, 1);
                    if next != state.hover_idx || !state.keyboard_focus {
                        state.hover_idx = next;
                        state.keyboard_focus = true;
                        state.render();
                        state.uia.sync_focus(state.dpi(), state.hover_idx);
                    }
                }
                0x24 => {
                    // VK_HOME
                    let next = move_menu_selection(&state.entries, None, 1);
                    if next != state.hover_idx || !state.keyboard_focus {
                        state.hover_idx = next;
                        state.keyboard_focus = true;
                        state.render();
                        state.uia.sync_focus(state.dpi(), state.hover_idx);
                    }
                }
                0x23 => {
                    // VK_END
                    let next = move_menu_selection(&state.entries, None, -1);
                    if next != state.hover_idx || !state.keyboard_focus {
                        state.hover_idx = next;
                        state.keyboard_focus = true;
                        state.render();
                        state.uia.sync_focus(state.dpi(), state.hover_idx);
                    }
                }
                0x0D | 0x20 => {
                    // VK_RETURN | VK_SPACE
                    if let Some(idx) = state.hover_idx {
                        if let Some(MenuEntry::Item { id, .. }) = state.entries.get(idx) {
                            state.selected_id = Some(*id);
                            state.closed = true;
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !state.tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                state.tracking_mouse = true;
            }
            let scale = state.dpi() as f32 / 96.0;
            let x = ((lparam.0 & 0xFFFF) as i16) as f32 / scale;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32 / scale;
            let new_hover = state.hit_test(x, y);
            if new_hover != state.hover_idx || state.keyboard_focus {
                state.hover_idx = new_hover;
                state.keyboard_focus = false;
                state.render();
                state.uia.sync_focus(state.dpi(), state.hover_idx);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            state.tracking_mouse = false;
            if !state.keyboard_focus && state.hover_idx.is_some() {
                state.hover_idx = None;
                state.render();
                state.uia.sync_focus(state.dpi(), state.hover_idx);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let scale = state.dpi() as f32 / 96.0;
            let x = ((lparam.0 & 0xFFFF) as i16) as f32 / scale;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32 / scale;
            if let Some(idx) = state.hit_test(x, y) {
                if let MenuEntry::Item { id, .. } = state.entries[idx] {
                    state.selected_id = Some(id);
                }
            }
            state.closed = true;
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_GETOBJECT => {
            if is_uia_getobject(lparam) {
                let root = state.uia.root_for_hwnd().clone();
                return return_raw_provider(hwnd, wparam, lparam, Some(&root));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_UIA_MENU_INVOKE => {
            let cmd_id = wparam.0;
            if state
                .entries
                .iter()
                .any(|e| matches!(e, MenuEntry::Item { id, .. } if *id == cmd_id))
            {
                state.selected_id = Some(cmd_id);
                state.closed = true;
            }
            LRESULT(0)
        }
        WM_UIA_MENU_FOCUS => {
            let entry_idx = wparam.0;
            if matches!(state.entries.get(entry_idx), Some(MenuEntry::Item { .. }))
                && (state.hover_idx != Some(entry_idx) || !state.keyboard_focus)
            {
                state.hover_idx = Some(entry_idx);
                state.keyboard_focus = true;
                state.render();
                state.uia.sync_focus(state.dpi(), state.hover_idx);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = return_raw_provider(hwnd, WPARAM(0), LPARAM(0), None);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn move_menu_selection(entries: &[MenuEntry], current: Option<usize>, dir: isize) -> Option<usize> {
    let item_indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e, MenuEntry::Item { .. }).then_some(i))
        .collect();
    if item_indices.is_empty() {
        return None;
    }
    match current.and_then(|c| item_indices.iter().position(|&idx| idx == c)) {
        Some(pos) => {
            let len = item_indices.len() as isize;
            let next_pos = (pos as isize + dir).rem_euclid(len) as usize;
            Some(item_indices[next_pos])
        }
        None => {
            if dir >= 0 {
                Some(item_indices[0])
            } else {
                Some(*item_indices.last().unwrap())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_keyboard_navigation_skips_separators() {
        let entries = vec![
            MenuEntry::Item { id: 1, label: "One".into(), glyph: 0, checked: None },
            MenuEntry::Separator,
            MenuEntry::Item { id: 2, label: "Two".into(), glyph: 0, checked: None },
        ];
        assert_eq!(move_menu_selection(&entries, None, 1), Some(0));
        assert_eq!(move_menu_selection(&entries, Some(0), 1), Some(2));
        assert_eq!(move_menu_selection(&entries, Some(2), 1), Some(0));
        assert_eq!(move_menu_selection(&entries, Some(0), -1), Some(2));
    }
}
