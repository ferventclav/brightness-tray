#![windows_subsystem = "windows"]

mod ambient;
mod camera;
mod menu;
mod monitors;
mod startup;
mod theme;
mod ui;
mod uia;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, SetBkColor, SetTextColor, TextOutW, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, FF_DONTCARE, FW_NORMAL, OUT_TT_PRECIS,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, POWERBROADCAST_SETTING,
};

const GUID_LIDSWITCH_STATE_CHANGE: windows::core::GUID =
    windows::core::GUID::from_u128(0xBA3E0F4D_B817_4094_A2D1_D56379E6A0F3);
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DispatchMessageW, FindWindowW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, KillTimer, LoadCursorW,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetTimer, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    HICON, HWND_TOPMOST, ICONINFO, IDC_ARROW, MSG, SM_CXSMICON, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_GETOBJECT, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_POWERBROADCAST, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WM_TIMER, DEVICE_NOTIFY_WINDOW_HANDLE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ambient::{AmbientCapability, AmbientResult};
use monitors::{BackendKind, MonitorManager};
use theme::ThemeColors;
use ui::{
    apply_win11_backdrop, button_rects, card_index_at, compute_flyout_height_dip, compute_flyout_position,
    hit_test, monitor_work_area_and_dpi_at, x_to_brightness_f32, HoverTarget, RenderState,
    Renderer, UiMonitor, FLYOUT_WIDTH_DIP,
};
use uia::{decode_action, FlyoutUia, UiaAction, WM_UIA_ACTION};

const WM_TRAY_ICON: u32 = WM_APP + 1;
const WM_MONITORS_UPDATED: u32 = WM_APP + 2;
const WM_AUTO_BRIGHTNESS_RESULT: u32 = WM_APP + 3;
const WM_WORKER_STATUS: u32 = WM_APP + 4;
const WM_SHOW_FROM_RELAUNCH: u32 = WM_APP + 5;
/// Covers a slow USB webcam: up to ~2 s to start streaming plus the 2.8 s capture watchdog.
const AMBIENT_TIMEOUT_MS: u32 = 6000;
const WHEEL_DELTA: i32 = 120;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;
const TRAY_ICON_ID: u32 = 1;
const TIMER_REBUILD_TRAY: usize = 100;
const TIMER_WAKE_AUTO: usize = 102;
const TIMER_ANIM_SLIDER: usize = 103;
const TIMER_AMBIENT_TIMEOUT: usize = 104;
const TIMER_RETRY_TRAY: usize = 105;
const TIMER_DISPLAY_CHANGE: usize = 106;
const TIMER_DDC_RETRY: usize = 107;
const TIMER_REFRESH_ANIM: usize = 108;
const TIMER_STATUS_CLEAR: usize = 109;

const AMBIENT_RES_CAMERA_IN_USE: usize = 101;
const AMBIENT_RES_CAMERA_DENIED: usize = 102;
const AMBIENT_RES_UNAVAILABLE: usize = 103;

const CMD_TOGGLE_STARTUP: usize = 1001;
const CMD_TOGGLE_LINK: usize = 1002;
const CMD_REFRESH: usize = 1003;
const CMD_EXIT: usize = 1004;
const CMD_MEASURE_AMBIENT: usize = 1005;
const CMD_TOGGLE_WAKE_AUTO: usize = 1006;

enum WorkerCmd {
    Refresh { gen: u64 },
    SetBrightness { id: String, brightness: u32, smooth: bool },
    Shutdown,
}

struct RefreshSnapshot {
    monitors: Vec<UiMonitor>,
    ambient_cap: AmbientCapability,
    has_pending_ddc: bool,
    gen: u64,
}

struct AppState {
    hwnd: HWND,
    tray_icon: HICON,
    taskbar_created_msg: u32,
    theme: ThemeColors,
    renderer: Option<Renderer>,
    monitors: Vec<UiMonitor>,
    shared_snapshot: Arc<Mutex<RefreshSnapshot>>,
    latest_targets: Arc<Mutex<HashMap<String, u32>>>,
    interaction_gen: Arc<AtomicU64>,
    abort_ramp: Arc<AtomicBool>,
    worker_tx: Sender<WorkerCmd>,
    hover: HoverTarget,
    dragging_idx: Option<usize>,
    drag_anchors: Option<Vec<(String, f32)>>,
    focused_idx: Option<usize>,
    /// Keyboard focus on a footer button; never set together with `focused_idx`
    focused_btn: Option<HoverTarget>,
    linked: bool,
    ambient_cap: AmbientCapability,
    measuring_ambient: bool,
    refreshing: bool,
    refresh_angle: f32,
    ddc_retry_scheduled: bool,
    status_msg: Option<String>,
    status_is_error: bool,
    auto_on_wake: bool,
    visible: bool,
    menu_open: bool,
    anchor_x: i32,
    last_show_time: Instant,
    last_hide_time: Instant,
    anim_start: Option<Instant>,
    tracking_mouse: bool,
    wheel_accum: i32,
    wheel_card: Option<usize>,
    lid_open: Option<bool>,
    ambient_seq: u64,
    ambient_busy: Arc<AtomicBool>,
    worker_status: Arc<Mutex<Option<String>>>,
    uia: FlyoutUia,
}

impl AppState {
    fn dpi(&self) -> u32 {
        unsafe {
            let d = GetDpiForWindow(self.hwnd);
            if d == 0 { 96 } else { d }
        }
    }

    fn size_px_for_dpi(&self, dpi: u32) -> (u32, u32) {
        let dpi_f = dpi as f32;
        let w = (FLYOUT_WIDTH_DIP * dpi_f / 96.0).round() as u32;
        let h_dip = compute_flyout_height_dip(self.monitors.len());
        let h = (h_dip * dpi_f / 96.0).round() as u32;
        (w.max(100), h.max(100))
    }

    fn footer_hint(&self) -> Option<&str> {
        if self.measuring_ambient {
            Some("Sampling room light...")
        } else if self.refreshing {
            Some("Refreshing displays...")
        } else if let Some(msg) = self.status_msg.as_deref() {
            Some(msg)
        } else {
            // Hover wins; otherwise describe the footer button that has keyboard focus.
            // The letter is the keyboard shortcut, so hovering teaches it.
            let target = match (self.hover, self.focused_btn) {
                (HoverTarget::None, Some(b)) => b,
                (h, _) => h,
            };
            match target {
                HoverTarget::AutoButton => Some("Match room light (A)"),
                HoverTarget::LinkButton => Some(if self.linked {
                    "Displays linked (L)"
                } else {
                    "Link displays (L)"
                }),
                HoverTarget::RefreshButton => Some("Refresh displays (R)"),
                HoverTarget::SettingsButton => Some("Settings"),
                _ => None,
            }
        }
    }

    fn footer_hint_is_error(&self) -> bool {
        self.status_is_error && self.status_msg.is_some() && !self.measuring_ambient && !self.refreshing
    }

    fn show_status(&mut self, msg: &str, duration_ms: u32, is_error: bool) {
        self.status_msg = Some(msg.to_string());
        self.status_is_error = is_error;
        unsafe {
            let _ = SetTimer(self.hwnd, TIMER_STATUS_CLEAR, duration_ms, None);
        }
        if self.visible {
            self.render();
            self.uia.notify(msg, is_error);
        }
    }

    fn set_status(&mut self, msg: &str, duration_ms: u32) {
        self.show_status(msg, duration_ms, false);
    }

    /// Something the user asked for did not happen: primary text, and long enough to read.
    fn set_error(&mut self, msg: &str) {
        self.show_status(msg, 4500, true);
    }

    fn sync_uia(&mut self) {
        let dpi = self.dpi();
        self.uia.sync(
            self.hwnd,
            self.visible,
            dpi,
            &self.monitors,
            self.ambient_cap != AmbientCapability::None,
            self.linked,
            self.measuring_ambient,
            self.focused_idx,
            self.focused_btn,
        );
    }

    fn footer_buttons(&self) -> Vec<HoverTarget> {
        let total_h = compute_flyout_height_dip(self.monitors.len());
        button_rects(total_h, self.ambient_cap != AmbientCapability::None)
            .into_iter()
            .map(|(t, _)| t)
            .collect()
    }

    fn button_enabled(&self, target: HoverTarget) -> bool {
        match target {
            HoverTarget::AutoButton => !self.monitors.is_empty(),
            HoverTarget::LinkButton => self.monitors.len() > 1,
            HoverTarget::RefreshButton | HoverTarget::SettingsButton => true,
            _ => false,
        }
    }

    /// Tab order: each display card, then enabled footer buttons left to right, then around again.
    fn cycle_focus(&mut self, backwards: bool) {
        let mut order: Vec<(Option<usize>, Option<HoverTarget>)> =
            (0..self.monitors.len()).map(|i| (Some(i), None)).collect();
        order.extend(
            self.footer_buttons()
                .into_iter()
                .filter(|&b| self.button_enabled(b))
                .map(|b| (None, Some(b))),
        );
        let n = order.len();
        if n == 0 {
            return;
        }
        let current = order.iter().position(|(card, btn)| {
            (card.is_some() && *card == self.focused_idx) || (btn.is_some() && *btn == self.focused_btn)
        });
        let next = match current {
            Some(p) if backwards => (p + n - 1) % n,
            Some(p) => (p + 1) % n,
            None if backwards => n - 1,
            None => 0,
        };
        self.focused_idx = order[next].0;
        self.focused_btn = order[next].1;
        self.render();
    }

    /// Screen point just above the middle of a footer button, for anchoring the settings menu.
    fn button_screen_anchor(&self, target: HoverTarget) -> Option<POINT> {
        let total_h = compute_flyout_height_dip(self.monitors.len());
        let (_, r) = button_rects(total_h, self.ambient_cap != AmbientCapability::None)
            .into_iter()
            .find(|(t, _)| *t == target)?;
        let scale = self.dpi() as f32 / 96.0;
        let mut pt = POINT {
            x: (((r.left + r.right) / 2.0) * scale).round() as i32,
            y: (r.top * scale).round() as i32,
        };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut pt);
        }
        Some(pt)
    }

    fn invoke_button(&mut self, target: HoverTarget, from_keyboard: bool) {
        if !self.button_enabled(target) {
            return;
        }
        match target {
            HoverTarget::AutoButton => self.trigger_ambient_measure(),
            HoverTarget::LinkButton => {
                self.linked = !self.linked;
                write_reg_bool(w!("Linked"), self.linked);
                let msg = if self.linked {
                    "Displays linked (L)"
                } else {
                    "Displays unlinked (L)"
                };
                self.set_status(msg, 1800);
            }
            HoverTarget::RefreshButton => {
                self.ddc_retry_scheduled = false;
                self.request_refresh(true);
            }
            HoverTarget::SettingsButton => {
                let anchor = if from_keyboard {
                    self.button_screen_anchor(HoverTarget::SettingsButton)
                } else {
                    None
                };
                self.show_context_menu(anchor);
            }
            _ => {}
        }
    }

    fn render(&mut self) {
        let dpi = self.dpi() as f32;
        let hint = self.footer_hint().map(str::to_owned);
        let rs = RenderState {
            monitors: &self.monitors,
            theme: &self.theme,
            hover: self.hover,
            dragging_idx: self.dragging_idx,
            focused_idx: self.focused_idx,
            linked: self.linked,
            measuring_ambient: self.measuring_ambient,
            refreshing: self.refreshing,
            refresh_angle: self.refresh_angle,
            show_auto: self.ambient_cap != AmbientCapability::None,
            footer_hint: hint.as_deref(),
            footer_hint_strong: self.footer_hint_is_error(),
            focused_btn: self.focused_btn,
            dpi,
        };
        let needs_recreate = match &self.renderer {
            Some(r) => r.render(&rs).is_err(),
            None => true,
        };
        if needs_recreate {
            let (w, h) = self.size_px_for_dpi(self.dpi());
            self.renderer = Renderer::new(self.hwnd, w, h, dpi).ok();
            if let Some(r) = &self.renderer {
                let _ = r.render(&rs);
            }
        }
        self.sync_uia();
    }

    fn request_refresh(&mut self, animate_icon: bool) {
        let gen = self.interaction_gen.load(Ordering::Acquire);
        let _ = self.worker_tx.send(WorkerCmd::Refresh { gen });
        if animate_icon && self.visible {
            self.refreshing = true;
            unsafe {
                let _ = SetTimer(self.hwnd, TIMER_REFRESH_ANIM, 16, None);
            }
            self.render();
            self.uia.notify("Refreshing displays", false);
        }
    }

    fn trigger_ambient_measure(&mut self) {
        if self.ambient_cap == AmbientCapability::None
            || self.monitors.is_empty()
            || self.measuring_ambient
        {
            return;
        }
        // A measurement that timed out can still be holding the camera; never start a second one.
        if self.ambient_busy.swap(true, Ordering::AcqRel) {
            self.set_status("Still measuring, try again", 2400);
            return;
        }
        self.measuring_ambient = true;
        self.status_msg = None;
        self.ambient_seq = self.ambient_seq.wrapping_add(1);
        let seq = self.ambient_seq;
        let busy = self.ambient_busy.clone();
        let hwnd_raw = self.hwnd.0 as isize;
        unsafe {
            let _ = SetTimer(self.hwnd, TIMER_AMBIENT_TIMEOUT, AMBIENT_TIMEOUT_MS, None);
        }
        std::thread::spawn(move || {
            // WinRT LightSensor and Media Foundation both need COM on this thread
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let code = match ambient::measure_ambient_brightness() {
                AmbientResult::Brightness(b) => b.min(100) as usize,
                AmbientResult::CameraInUse => AMBIENT_RES_CAMERA_IN_USE,
                AmbientResult::CameraDenied => AMBIENT_RES_CAMERA_DENIED,
                AmbientResult::Unavailable => AMBIENT_RES_UNAVAILABLE,
            };
            if com {
                unsafe { CoUninitialize() };
            }
            busy.store(false, Ordering::Release);
            unsafe {
                let _ = PostMessageW(
                    HWND(hwnd_raw as *mut core::ffi::c_void),
                    WM_AUTO_BRIGHTNESS_RESULT,
                    WPARAM(code),
                    LPARAM(seq as isize),
                );
            }
        });
        if self.visible {
            self.render();
            self.uia.notify("Sampling room light", false);
        }
    }

    fn resize_and_position(&mut self) {
        let dpi = self.dpi();
        let (w, h) = self.size_px_for_dpi(dpi);
        let center_pt = POINT {
            x: self.anchor_x + (w as i32) / 2,
            y: 0,
        };
        let (work_area, _) = monitor_work_area_and_dpi_at(center_pt);
        let margin = ((12.0 * dpi as f32) / 96.0).round() as i32;
        let x = self
            .anchor_x
            .max(work_area.left + margin)
            .min(work_area.right - (w as i32) - margin);
        self.anchor_x = x;
        let y = (work_area.bottom - (h as i32) - margin).max(work_area.top + margin);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                w as i32,
                h as i32,
                SWP_NOACTIVATE,
            );
        }
        if let Some(r) = &mut self.renderer {
            let _ = r.resize(w, h, dpi as f32);
        }
    }

    /// `anchor` is the screen point the flyout opens beside: the cursor for a mouse click,
    /// the tray icon for keyboard selection or a relaunch.
    fn show_flyout(&mut self, anchor: POINT) {
        self.theme = ThemeColors::load();
        apply_win11_backdrop(self.hwnd, self.theme.is_dark);
        if uia::is_screen_reader_active() {
            if !self.monitors.is_empty() {
                self.focused_idx = Some(0);
                self.focused_btn = None;
            } else {
                self.focused_idx = None;
                self.focused_btn = self
                    .footer_buttons()
                    .into_iter()
                    .find(|&b| self.button_enabled(b));
            }
        } else {
            self.focused_idx = None;
            self.focused_btn = None;
        }
        self.status_msg = None;
        self.status_is_error = false;
        self.wheel_accum = 0;
        self.wheel_card = None;
        self.request_refresh(false);

        let (_, target_dpi) = monitor_work_area_and_dpi_at(anchor);
        let (w, h) = self.size_px_for_dpi(target_dpi);
        let (x, y) = compute_flyout_position(anchor, w as i32, h as i32, target_dpi);
        self.anchor_x = x;
        if let Some(r) = &mut self.renderer {
            let _ = r.resize(w, h, target_dpi as f32);
        }
        // Mark visible and sync UIA before SetForegroundWindow so screen readers never see an offscreen 0x0 pane
        self.visible = true;
        self.last_show_time = Instant::now();
        self.render();
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                w as i32,
                h as i32,
                SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(self.hwnd);
        }
        // Re-render after SetWindowPos so DPI and screen bounds match the destination monitor
        let actual_dpi = self.dpi();
        if actual_dpi != target_dpi {
            let (aw, ah) = self.size_px_for_dpi(actual_dpi);
            if let Some(r) = &mut self.renderer {
                let _ = r.resize(aw, ah, actual_dpi as f32);
            }
        }
        self.render();
    }

    fn hide_flyout(&mut self) {
        if self.visible {
            self.visible = false;
            self.dragging_idx = None;
            self.drag_anchors = None;
            self.focused_idx = None;
            self.focused_btn = None;
            self.hover = HoverTarget::None;
            self.refreshing = false;
            self.refresh_angle = 0.0;
            self.status_msg = None;
            self.status_is_error = false;
            self.last_hide_time = Instant::now();
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_REFRESH_ANIM);
                let _ = KillTimer(self.hwnd, TIMER_STATUS_CLEAR);
                let _ = ReleaseCapture();
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.sync_uia();
        }
    }

    fn toggle_flyout(&mut self, anchor: POINT) {
        if self.visible {
            // A double click or a repeated select notification must not close what just opened
            if self.last_show_time.elapsed() > Duration::from_millis(250) {
                self.hide_flyout();
            }
        } else if self.last_hide_time.elapsed() > Duration::from_millis(180) {
            self.show_flyout(anchor);
        }
    }

    fn nudge_brightness(&mut self, idx: usize, next: i32) {
        let next = next.clamp(0, 100) as f32;
        self.set_monitor_brightness_f32(idx, next, false);
        update_tray_tooltip(self.hwnd, &self.monitors);
    }

    fn set_monitor_brightness_f32(&mut self, idx: usize, new_val_f32: f32, smooth: bool) {
        let new_val_f32 = new_val_f32.clamp(0.0, 100.0);
        let new_val = new_val_f32.round() as u32;
        let Some(old_val_f32) = self.monitors.get(idx).map(|m| m.visual_brightness) else {
            return;
        };
        let old_val = self.monitors[idx].brightness;

        self.interaction_gen.fetch_add(1, Ordering::Release);
        if !smooth {
            self.abort_ramp.store(true, Ordering::Release);
            self.anim_start = None;
        }

        if self.linked {
            let (use_anchors, delta_f32) = if let Some(ref anchors) = self.drag_anchors {
                let anchor_idx = self
                    .monitors
                    .get(idx)
                    .and_then(|m| anchors.iter().find(|(id, _)| id == &m.id).map(|(_, v)| *v))
                    .unwrap_or(old_val_f32);
                (true, new_val_f32 - anchor_idx)
            } else {
                (false, new_val_f32 - old_val_f32)
            };

            for (i, mon) in self.monitors.iter_mut().enumerate() {
                let target_f32 = if i == idx {
                    new_val_f32
                } else if use_anchors {
                    let anchor_i = self
                        .drag_anchors
                        .as_ref()
                        .and_then(|a| a.iter().find(|(id, _)| id == &mon.id).map(|(_, v)| *v))
                        .unwrap_or(mon.visual_brightness);
                    (anchor_i + delta_f32).clamp(0.0, 100.0)
                } else {
                    (mon.visual_brightness + delta_f32).clamp(0.0, 100.0)
                };
                if !smooth {
                    mon.visual_brightness = target_f32;
                }
                let target = target_f32.round() as u32;
                if mon.brightness != target {
                    mon.brightness = target;
                    if let Ok(mut map) = self.latest_targets.lock() {
                        map.insert(mon.id.clone(), target);
                    }
                    let _ = self.worker_tx.send(WorkerCmd::SetBrightness {
                        id: mon.id.clone(),
                        brightness: target,
                        smooth,
                    });
                }
            }
        } else if let Some(mon) = self.monitors.get_mut(idx) {
            if !smooth {
                mon.visual_brightness = new_val_f32;
            }
            if old_val != new_val {
                mon.brightness = new_val;
                if let Ok(mut map) = self.latest_targets.lock() {
                    map.insert(mon.id.clone(), new_val);
                }
                let _ = self.worker_tx.send(WorkerCmd::SetBrightness {
                    id: mon.id.clone(),
                    brightness: new_val,
                    smooth,
                });
            }
        }

        self.render();
    }

    /// `anchor`: where to open the menu; the cursor when `None` (right-click, gear click).
    fn show_context_menu(&mut self, anchor: Option<POINT>) {
        let mut entries = Vec::new();
        if self.ambient_cap != AmbientCapability::None {
            entries.push(menu::MenuEntry::Item {
                id: CMD_MEASURE_AMBIENT,
                label: "Match room light (Auto)".to_string(),
                glyph: 0xE706, // Brightness
                checked: None,
            });
            entries.push(menu::MenuEntry::Item {
                id: CMD_TOGGLE_WAKE_AUTO,
                label: "Auto-adjust on wake".to_string(),
                glyph: 0xE7E8, // Power / wake
                checked: Some(self.auto_on_wake),
            });
        }
        entries.push(menu::MenuEntry::Item {
            id: CMD_TOGGLE_LINK,
            label: "Link displays".to_string(),
            glyph: 0xE71B, // Link
            checked: Some(self.linked),
        });
        entries.push(menu::MenuEntry::Item {
            id: CMD_TOGGLE_STARTUP,
            label: "Run at Windows startup".to_string(),
            glyph: 0xE770, // System / startup
            checked: Some(startup::is_startup_enabled()),
        });
        entries.push(menu::MenuEntry::Item {
            id: CMD_REFRESH,
            label: "Refresh displays".to_string(),
            glyph: 0xE72C, // Refresh
            checked: None,
        });
        entries.push(menu::MenuEntry::Separator);
        entries.push(menu::MenuEntry::Item {
            id: CMD_EXIT,
            label: "Exit".to_string(),
            glyph: 0xE711, // Cancel / Exit
            checked: None,
        });

        let pt = anchor.unwrap_or_else(|| {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
            }
            cursor
        });
        self.menu_open = true;
        let selected = menu::show_fluent_menu(self.hwnd, pt, ThemeColors::load(), entries);
        self.menu_open = false;

        if let Some(cmd_id) = selected {
            unsafe {
                let _ = PostMessageW(self.hwnd, WM_COMMAND, WPARAM(cmd_id), LPARAM(0));
                if self.visible && cmd_id != CMD_EXIT {
                    let _ = SetForegroundWindow(self.hwnd);
                }
            }
        } else if self.visible {
            unsafe {
                let mut cur_pt = POINT::default();
                let mut win_rect = RECT::default();
                let _ = GetCursorPos(&mut cur_pt);
                let _ = GetWindowRect(self.hwnd, &mut win_rect);
                let inside_flyout = cur_pt.x >= win_rect.left
                    && cur_pt.x <= win_rect.right
                    && cur_pt.y >= win_rect.top
                    && cur_pt.y <= win_rect.bottom;
                if !inside_flyout && GetForegroundWindow() != self.hwnd {
                    self.hide_flyout();
                } else {
                    let _ = SetForegroundWindow(self.hwnd);
                }
            }
        }
        if self.visible {
            self.hover = HoverTarget::None;
            self.render();
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let dir = std::path::PathBuf::from(local).join("BrightnessTray");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("crash.log"), format!("{info}\n"));
        }
    }));

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let class_name = w!("BrightnessTrayFlyoutClass");

        // Single-instance guard: launching the exe again opens the running copy's flyout
        let _mutex = CreateMutexW(None, true, w!("Local\\BrightnessTraySingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(existing) = FindWindowW(class_name, PCWSTR::null()) {
                let mut pid = 0u32;
                GetWindowThreadProcessId(existing, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(existing, WM_SHOW_FROM_RELAUNCH, WPARAM(0), LPARAM(0));
            }
            CoUninitialize();
            return;
        }

        // A moved exe leaves a dead Run entry behind; point it here
        startup::repair_moved_path();

        let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap().into();

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let shared_snapshot: Arc<Mutex<RefreshSnapshot>> = Arc::new(Mutex::new(RefreshSnapshot {
            monitors: Vec::new(),
            ambient_cap: AmbientCapability::None,
            has_pending_ddc: false,
            gen: 0,
        }));
        let latest_targets: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        let interaction_gen = Arc::new(AtomicU64::new(0));
        let abort_ramp = Arc::new(AtomicBool::new(false));
        let worker_status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerCmd>();

        let theme = ThemeColors::load();
        let tray_icon = create_monitor_tray_icon(theme.is_dark);
        let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));

        let mut state = Box::new(AppState {
            hwnd: HWND::default(),
            tray_icon,
            taskbar_created_msg,
            theme,
            renderer: None,
            monitors: Vec::new(),
            shared_snapshot: shared_snapshot.clone(),
            latest_targets: latest_targets.clone(),
            interaction_gen: interaction_gen.clone(),
            abort_ramp: abort_ramp.clone(),
            worker_tx: worker_tx.clone(),
            hover: HoverTarget::None,
            dragging_idx: None,
            drag_anchors: None,
            focused_idx: None,
            focused_btn: None,
            linked: read_reg_bool(w!("Linked"), false),
            ambient_cap: AmbientCapability::None,
            measuring_ambient: false,
            refreshing: false,
            refresh_angle: 0.0,
            ddc_retry_scheduled: false,
            status_msg: None,
            status_is_error: false,
            auto_on_wake: read_reg_bool(w!("AutoOnWake"), false),
            visible: false,
            menu_open: false,
            anchor_x: 0,
            last_show_time: Instant::now() - Duration::from_secs(10),
            last_hide_time: Instant::now() - Duration::from_secs(10),
            anim_start: None,
            tracking_mouse: false,
            wheel_accum: 0,
            wheel_card: None,
            lid_open: None,
            ambient_seq: 0,
            ambient_busy: Arc::new(AtomicBool::new(false)),
            worker_status: worker_status.clone(),
            uia: FlyoutUia::new(HWND::default()),
        });

        let state_ptr = &mut *state as *mut AppState;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            class_name,
            w!("Display Brightness"),
            WS_POPUP,
            0,
            0,
            364,
            240,
            None,
            None,
            hinstance,
            Some(state_ptr as *const core::ffi::c_void),
        )
        .expect("CreateWindowExW failed");

        state.hwnd = hwnd;
        state.uia = FlyoutUia::new(hwnd);
        apply_win11_backdrop(hwnd, state.theme.is_dark);
        let (w, h) = state.size_px_for_dpi(state.dpi());
        state.renderer = Renderer::new(hwnd, w, h, state.dpi() as f32).ok();

        if !add_tray_icon(hwnd, state.tray_icon) {
            let _ = SetTimer(hwnd, TIMER_RETRY_TRAY, 1000, None);
        }
        let _lid_notify = RegisterPowerSettingNotification(
            hwnd,
            &GUID_LIDSWITCH_STATE_CHANGE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );

        // Spawn dedicated background brightness worker thread (COM + WMI + DDC/CI)
        let hwnd_raw = hwnd.0 as isize;
        let shared_for_worker = shared_snapshot.clone();
        let targets_for_worker = latest_targets.clone();
        let abort_for_worker = abort_ramp.clone();
        let status_for_worker = worker_status.clone();
        std::thread::spawn(move || {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut mgr = MonitorManager::new();
            let mut cap = ambient::detect_capability();
            publish_snapshot(&mgr, cap, 0, &shared_for_worker, hwnd_raw);
            let mut last_recovery = Instant::now() - Duration::from_secs(60);

            while let Ok(first_cmd) = worker_rx.recv() {
                let mut refresh_gen: Option<u64> = None;
                let mut pending_sets: HashMap<String, (u32, bool)> = HashMap::new();
                let mut should_exit = false;

                let mut process_cmd = |cmd: WorkerCmd| match cmd {
                    WorkerCmd::Refresh { gen } => refresh_gen = Some(gen),
                    WorkerCmd::SetBrightness { id, brightness, smooth } => {
                        pending_sets.insert(id, (brightness, smooth));
                    }
                    WorkerCmd::Shutdown => should_exit = true,
                };

                process_cmd(first_cmd);
                while let Ok(next_cmd) = worker_rx.try_recv() {
                    process_cmd(next_cmd);
                }

                if should_exit {
                    break;
                }
                if let Some(gen) = refresh_gen {
                    mgr.refresh();
                    cap = ambient::detect_capability();
                    publish_snapshot(&mgr, cap, gen, &shared_for_worker, hwnd_raw);
                }

                let mut failed: Vec<String> = Vec::new();
                let mut smooth_jobs: Vec<(String, u32, u32, bool)> = Vec::new();
                for (id, (initial_target, smooth)) in pending_sets {
                    let target = targets_for_worker
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&id).copied())
                        .unwrap_or(initial_target);
                    let mon_meta = mgr.monitors.iter().find(|m| m.id == id);
                    let current = mon_meta.map(|m| m.brightness).unwrap_or(target);
                    let is_internal = mon_meta
                        .map(|m| matches!(m.backend, BackendKind::InternalWmi))
                        .unwrap_or(false);

                    if smooth && current.abs_diff(target) > 3 {
                        smooth_jobs.push((id, current, target, is_internal));
                    } else if !mgr.set_brightness(&id, target) {
                        failed.push(id);
                    }
                }

                // Interleave smooth transitions across ALL monitors (internal WMI first each step)
                if !smooth_jobs.is_empty() {
                    smooth_jobs.sort_by_key(|(_, _, _, is_int)| !*is_int);
                    abort_for_worker.store(false, Ordering::Release);

                    let max_diff = smooth_jobs
                        .iter()
                        .map(|(_, c, t, _)| c.abs_diff(*t))
                        .max()
                        .unwrap_or(0);
                    let steps = max_diff.min(24);
                    let delay_ms = 26u64;
                    let mut last_sent: Vec<Option<u32>> = vec![None; smooth_jobs.len()];
                    let mut alive: Vec<bool> = vec![true; smooth_jobs.len()];

                    'ramp: for s in 1..=steps {
                        if abort_for_worker.load(Ordering::Acquire) {
                            break 'ramp;
                        }
                        let t = s as f32 / steps as f32;
                        let eased = ease_in_out_cubic(t);

                        for (j, (id, current, target, _)) in smooth_jobs.iter().enumerate() {
                            if abort_for_worker.load(Ordering::Acquire) {
                                break 'ramp;
                            }
                            let latest = targets_for_worker
                                .lock()
                                .ok()
                                .and_then(|m| m.get(id).copied())
                                .unwrap_or(*target);
                            if latest != *target {
                                for (rem_id, _, rem_tgt, _) in &smooth_jobs {
                                    let l = targets_for_worker
                                        .lock()
                                        .ok()
                                        .and_then(|m| m.get(rem_id).copied())
                                        .unwrap_or(*rem_tgt);
                                    mgr.set_brightness(rem_id, l);
                                }
                                break 'ramp;
                            }
                            if !alive[j] {
                                continue;
                            }
                            let val = (*current as f32 + (*target as f32 - *current as f32) * eased)
                                .round()
                                .clamp(0.0, 100.0) as u32;
                            if last_sent[j] != Some(val) {
                                if mgr.set_brightness(id, val) {
                                    last_sent[j] = Some(val);
                                } else {
                                    alive[j] = false;
                                    failed.push(id.clone());
                                }
                            }
                        }
                        if s < steps {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }
                }

                if !failed.is_empty() {
                    recover_failed_sets(
                        &mut mgr,
                        failed,
                        cap,
                        &targets_for_worker,
                        &shared_for_worker,
                        &status_for_worker,
                        hwnd_raw,
                        &mut last_recovery,
                    );
                }
            }

            drop(mgr);
            CoUninitialize();
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        state.abort_ramp.store(true, Ordering::Release);
        let _ = worker_tx.send(WorkerCmd::Shutdown);
        let _ = uia::return_raw_provider(hwnd, WPARAM(0), LPARAM(0), None);
        uia::disconnect_all_providers();
        remove_tray_icon(hwnd);
        if !state.tray_icon.is_invalid() {
            let _ = DestroyIcon(state.tray_icon);
        }
        CoUninitialize();
    }
}

fn publish_snapshot(
    mgr: &MonitorManager,
    ambient_cap: AmbientCapability,
    gen: u64,
    shared: &Arc<Mutex<RefreshSnapshot>>,
    hwnd_raw: isize,
) {
    let list: Vec<UiMonitor> = mgr
        .monitors
        .iter()
        .map(|m| UiMonitor {
            id: m.id.clone(),
            display_number: m.display_number,
            name: m.name.clone(),
            brightness: m.brightness,
            visual_brightness: m.brightness as f32,
            anim_from: m.brightness as f32,
        })
        .collect();
    if let Ok(mut guard) = shared.lock() {
        *guard = RefreshSnapshot {
            monitors: list,
            ambient_cap,
            has_pending_ddc: mgr.has_pending_ddc,
            gen,
        };
    }
    unsafe {
        let _ = PostMessageW(
            HWND(hwnd_raw as *mut core::ffi::c_void),
            WM_MONITORS_UPDATED,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// A display that stops answering (after sleep, a cable re-plug or an input switch) usually has a
/// stale handle. Re-scan once, retry the latest target, and tell the user if it still fails, so
/// the slider never claims a brightness the monitor did not take.
#[allow(clippy::too_many_arguments)]
fn recover_failed_sets(
    mgr: &mut MonitorManager,
    mut failed: Vec<String>,
    cap: AmbientCapability,
    targets: &Arc<Mutex<HashMap<String, u32>>>,
    shared: &Arc<Mutex<RefreshSnapshot>>,
    status: &Arc<Mutex<Option<String>>>,
    hwnd_raw: isize,
    last_recovery: &mut Instant,
) {
    failed.sort();
    failed.dedup();
    let numbers: Vec<(String, u32)> = failed
        .iter()
        .map(|id| {
            let num = mgr
                .monitors
                .iter()
                .find(|m| &m.id == id)
                .map(|m| m.display_number)
                .unwrap_or(0);
            (id.clone(), num)
        })
        .collect();

    let mut still_failing: Vec<u32> = Vec::new();
    // Re-scanning costs a DDC round trip per display; at most once every 3 s during a drag
    if last_recovery.elapsed() >= Duration::from_secs(3) {
        *last_recovery = Instant::now();
        mgr.refresh();
        for (id, num) in &numbers {
            let target = targets.lock().ok().and_then(|m| m.get(id).copied());
            if !target.map(|t| mgr.set_brightness(id, t)).unwrap_or(false) {
                still_failing.push(*num);
            }
        }
        // gen 0 keeps the sliders the user is holding; vanished displays drop out of the list
        publish_snapshot(mgr, cap, 0, shared, hwnd_raw);
    } else {
        still_failing = numbers.iter().map(|(_, n)| *n).collect();
    }

    if still_failing.is_empty() {
        return;
    }
    still_failing.sort_unstable();
    still_failing.dedup();
    let msg = match still_failing.as_slice() {
        [n] if *n > 0 => format!("Display {n} isn't responding"),
        _ => "A display isn't responding".to_string(),
    };
    if let Ok(mut slot) = status.lock() {
        *slot = Some(msg);
    }
    unsafe {
        let _ = PostMessageW(
            HWND(hwnd_raw as *mut core::ffi::c_void),
            WM_WORKER_STATUS,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// Where the tray icon sits on screen, for opening the flyout without a mouse click.
fn tray_icon_anchor(hwnd: HWND) -> POINT {
    unsafe {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            ..Default::default()
        };
        if let Ok(r) = Shell_NotifyIconGetRect(&id) {
            if r.right > r.left && r.bottom > r.top {
                return POINT {
                    x: (r.left + r.right) / 2,
                    y: (r.top + r.bottom) / 2,
                };
            }
        }
        // Icon hidden in the overflow: bottom-right of the primary display, where the tray lives
        let (work, _) = monitor_work_area_and_dpi_at(POINT { x: 0, y: 0 });
        POINT {
            x: work.right - 1,
            y: work.bottom - 1,
        }
    }
}

/// Precision touchpads send many small wheel deltas per notch. Brightness moves one step per
/// 120 units of travel, so a touchpad swipe and a mouse notch feel the same.
/// Returns (steps to apply, leftover travel). Reversing direction drops the leftover.
fn wheel_steps(accum: i32, delta: i32) -> (i32, i32) {
    let base = if (accum > 0 && delta < 0) || (accum < 0 && delta > 0) {
        0
    } else {
        accum
    };
    let total = base + delta;
    let steps = total / WHEEL_DELTA;
    (steps, total - steps * WHEEL_DELTA)
}

/// Windows reports the current lid state the moment the app registers for lid events, so only
/// a closed-to-open change counts as "opening the lid".
fn lid_was_opened(previous: Option<bool>, now_open: bool) -> bool {
    previous == Some(false) && now_open
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            cs.lpCreateParams as isize,
        );
        return LRESULT(0);
    }

    let ptr = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
    ) as *mut AppState;

    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;

    if msg == state.taskbar_created_msg && state.taskbar_created_msg != 0 {
        if !add_tray_icon(hwnd, state.tray_icon) {
            let _ = SetTimer(hwnd, TIMER_RETRY_TRAY, 1000, None);
        }
        return LRESULT(0);
    }

    match msg {
        WM_TRAY_ICON => {
            // Tray clicks while the menu's own message loop runs would open a second menu
            if state.menu_open {
                return LRESULT(0);
            }
            let notif = (lparam.0 & 0xFFFF) as u32;
            match notif {
                WM_LBUTTONUP => {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    state.toggle_flyout(cursor);
                }
                NIN_KEYSELECT => {
                    // Keyboard (Win+B, Enter): the mouse may be on another screen entirely
                    state.toggle_flyout(tray_icon_anchor(hwnd));
                }
                WM_RBUTTONUP => {
                    state.show_context_menu(None);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_SHOW_FROM_RELAUNCH => {
            if state.visible {
                let _ = SetForegroundWindow(hwnd);
            } else {
                state.show_flyout(tray_icon_anchor(hwnd));
            }
            LRESULT(0)
        }
        WM_WORKER_STATUS => {
            let msg = state.worker_status.lock().ok().and_then(|mut s| s.take());
            if let Some(msg) = msg {
                state.set_error(&msg);
            }
            LRESULT(0)
        }
        WM_MONITORS_UPDATED => {
            let snapshot = state.shared_snapshot.lock().ok().map(|g| RefreshSnapshot {
                monitors: g.monitors.clone(),
                ambient_cap: g.ambient_cap,
                has_pending_ddc: g.has_pending_ddc,
                gen: g.gen,
            });
            if let Some(snap) = snapshot {
                state.ambient_cap = snap.ambient_cap;
                if state.refreshing {
                    state.refreshing = false;
                    state.refresh_angle = 0.0;
                    let _ = KillTimer(hwnd, TIMER_REFRESH_ANIM);
                }
                if snap.has_pending_ddc && !state.ddc_retry_scheduled {
                    state.ddc_retry_scheduled = true;
                    let _ = SetTimer(hwnd, TIMER_DDC_RETRY, 2000, None);
                }

                let cur_gen = state.interaction_gen.load(Ordering::Acquire);
                let preserve_active =
                    snap.gen < cur_gen || state.dragging_idx.is_some() || state.anim_start.is_some();

                if preserve_active {
                    let prev_map: HashMap<String, UiMonitor> = state
                        .monitors
                        .iter()
                        .map(|m| (m.id.clone(), m.clone()))
                        .collect();
                    let mut merged = snap.monitors;
                    for m in &mut merged {
                        if let Some(prev) = prev_map.get(&m.id) {
                            m.brightness = prev.brightness;
                            m.visual_brightness = prev.visual_brightness;
                            m.anim_from = prev.anim_from;
                        }
                    }
                    state.monitors = merged;
                } else {
                    state.monitors = snap.monitors;
                }

                if let Some(f) = state.focused_idx {
                    if state.monitors.is_empty() {
                        state.focused_idx = None;
                        if state.visible && state.focused_btn.is_none() {
                            state.focused_btn = state
                                .footer_buttons()
                                .into_iter()
                                .find(|&b| state.button_enabled(b));
                        }
                    } else if f >= state.monitors.len() {
                        state.focused_idx = Some(state.monitors.len() - 1);
                    }
                } else if let Some(b) = state.focused_btn {
                    if !state.button_enabled(b) {
                        state.focused_btn = state
                            .footer_buttons()
                            .into_iter()
                            .find(|&btn| state.button_enabled(btn));
                    }
                } else if state.visible
                    && !state.monitors.is_empty()
                    && uia::is_screen_reader_active()
                {
                    state.focused_idx = Some(0);
                }

                update_tray_tooltip(hwnd, &state.monitors);
                if state.visible {
                    state.resize_and_position();
                    state.render();
                } else {
                    state.sync_uia();
                }
            }
            LRESULT(0)
        }
        WM_AUTO_BRIGHTNESS_RESULT => {
            // A result that lands after "timed out" was already reported must not move the sliders
            if !state.measuring_ambient || lparam.0 as u64 != state.ambient_seq {
                return LRESULT(0);
            }
            let _ = KillTimer(hwnd, TIMER_AMBIENT_TIMEOUT);
            state.measuring_ambient = false;
            let code = wparam.0;
            if code <= 100 {
                let pct = code as u32;
                state.interaction_gen.fetch_add(1, Ordering::Release);
                state.abort_ramp.store(false, Ordering::Release);
                for mon in &mut state.monitors {
                    mon.anim_from = mon.visual_brightness;
                    if mon.brightness != pct {
                        mon.brightness = pct;
                        if let Ok(mut map) = state.latest_targets.lock() {
                            map.insert(mon.id.clone(), pct);
                        }
                        let _ = state.worker_tx.send(WorkerCmd::SetBrightness {
                            id: mon.id.clone(),
                            brightness: pct,
                            smooth: true,
                        });
                    }
                }
                update_tray_tooltip(hwnd, &state.monitors);
                if state.visible {
                    state.anim_start = Some(Instant::now());
                    let _ = SetTimer(hwnd, TIMER_ANIM_SLIDER, 16, None);
                } else {
                    for mon in &mut state.monitors {
                        mon.visual_brightness = pct as f32;
                    }
                }
            } else {
                let reason = match code {
                    AMBIENT_RES_CAMERA_IN_USE => "Camera in use",
                    AMBIENT_RES_CAMERA_DENIED => "Camera access off",
                    // Usually another app holding the camera: newer Windows builds no longer
                    // record camera use where the in-use check can see it
                    _ if state.ambient_cap == AmbientCapability::Webcam => "Couldn't open camera",
                    _ => "Sensor unavailable",
                };
                state.set_error(reason);
            }
            if state.visible {
                state.render();
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            state.ddc_retry_scheduled = false;
            let _ = SetTimer(hwnd, TIMER_DISPLAY_CHANGE, 600, None);
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            let old_dark = state.theme.is_dark;
            state.theme = ThemeColors::load();
            if old_dark != state.theme.is_dark {
                let _ = SetTimer(hwnd, TIMER_REBUILD_TRAY, 250, None);
            }
            if state.visible {
                apply_win11_backdrop(hwnd, state.theme.is_dark);
                state.render();
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_REBUILD_TRAY {
                let _ = KillTimer(hwnd, TIMER_REBUILD_TRAY);
                remove_tray_icon(hwnd);
                if !state.tray_icon.is_invalid() {
                    let _ = DestroyIcon(state.tray_icon);
                }
                state.tray_icon = create_monitor_tray_icon(state.theme.is_dark);
                if !add_tray_icon(hwnd, state.tray_icon) {
                    let _ = SetTimer(hwnd, TIMER_RETRY_TRAY, 1000, None);
                }
            } else if wparam.0 == TIMER_DISPLAY_CHANGE {
                let _ = KillTimer(hwnd, TIMER_DISPLAY_CHANGE);
                state.request_refresh(state.visible);
            } else if wparam.0 == TIMER_DDC_RETRY {
                let _ = KillTimer(hwnd, TIMER_DDC_RETRY);
                state.request_refresh(state.visible);
            } else if wparam.0 == TIMER_REFRESH_ANIM {
                if !state.refreshing || !state.visible {
                    let _ = KillTimer(hwnd, TIMER_REFRESH_ANIM);
                    state.refresh_angle = 0.0;
                } else {
                    state.refresh_angle =
                        (state.refresh_angle + 0.24) % (2.0 * std::f32::consts::PI);
                    state.render();
                }
            } else if wparam.0 == TIMER_STATUS_CLEAR {
                let _ = KillTimer(hwnd, TIMER_STATUS_CLEAR);
                state.status_msg = None;
                state.status_is_error = false;
                if state.visible {
                    state.render();
                }
            } else if wparam.0 == TIMER_WAKE_AUTO {
                let _ = KillTimer(hwnd, TIMER_WAKE_AUTO);
                state.ddc_retry_scheduled = false;
                state.request_refresh(state.visible);
                if state.auto_on_wake {
                    state.trigger_ambient_measure();
                }
            } else if wparam.0 == TIMER_ANIM_SLIDER {
                let elapsed = state
                    .anim_start
                    .map(|s| s.elapsed().as_secs_f32())
                    .unwrap_or(1.0);
                let t = (elapsed / 0.70).clamp(0.0, 1.0);
                let eased = ease_in_out_cubic(t);

                for mon in &mut state.monitors {
                    let target = mon.brightness as f32;
                    mon.visual_brightness = mon.anim_from + (target - mon.anim_from) * eased;
                }
                if t >= 1.0 || !state.visible {
                    state.anim_start = None;
                    let _ = KillTimer(hwnd, TIMER_ANIM_SLIDER);
                    for mon in &mut state.monitors {
                        mon.visual_brightness = mon.brightness as f32;
                    }
                }
                if state.visible {
                    state.render();
                }
            } else if wparam.0 == TIMER_RETRY_TRAY {
                if add_tray_icon(hwnd, state.tray_icon) {
                    let _ = KillTimer(hwnd, TIMER_RETRY_TRAY);
                }
            } else if wparam.0 == TIMER_AMBIENT_TIMEOUT {
                let _ = KillTimer(hwnd, TIMER_AMBIENT_TIMEOUT);
                if state.measuring_ambient {
                    state.measuring_ambient = false;
                    state.set_error("Sensor timed out");
                }
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let evt = wparam.0 as u32;
            let mut should_schedule = evt == PBT_APMRESUMEAUTOMATIC || evt == PBT_APMRESUMESUSPEND;
            if evt == PBT_POWERSETTINGCHANGE && lparam.0 != 0 {
                let pbs = &*(lparam.0 as *const POWERBROADCAST_SETTING);
                if pbs.PowerSetting == GUID_LIDSWITCH_STATE_CHANGE && pbs.DataLength >= 1 {
                    let now_open = pbs.Data[0] != 0;
                    let previous = state.lid_open.replace(now_open);
                    if lid_was_opened(previous, now_open) {
                        should_schedule = true;
                    }
                }
            }
            if should_schedule && state.auto_on_wake {
                let _ = SetTimer(hwnd, TIMER_WAKE_AUTO, 1500, None);
            }
            LRESULT(1)
        }
        WM_DPICHANGED => {
            if state.visible {
                state.resize_and_position();
                state.render();
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let active_flag = (wparam.0 & 0xFFFF) as u32;
            if active_flag == WA_INACTIVE
                && !state.menu_open
                && state.last_show_time.elapsed() > Duration::from_millis(180)
            {
                state.hide_flyout();
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let shift_down = (GetKeyState(0x10) as u16 & 0x8000) != 0;
            match wparam.0 {
                0x1B => state.hide_flyout(),
                0x26 | 0x28 => {
                    // VK_UP / VK_DOWN move between display cards
                    let count = state.monitors.len();
                    if count > 0 {
                        let up = wparam.0 == 0x26;
                        state.focused_btn = None;
                        state.focused_idx = Some(match state.focused_idx {
                            Some(i) if up => (i + count - 1) % count,
                            Some(i) => (i + 1) % count,
                            None if up => count - 1,
                            None => 0,
                        });
                        state.render();
                    }
                }
                0x09 => state.cycle_focus(shift_down), // Tab / Shift+Tab: cards, then footer
                0x0D | 0x20 => {
                    // Enter / Space press the focused footer button
                    if let Some(b) = state.focused_btn {
                        state.invoke_button(b, true);
                    }
                }
                0x5D => {
                    // Menu (Apps) key opens Settings beside its button
                    state.focused_idx = None;
                    state.focused_btn = Some(HoverTarget::SettingsButton);
                    state.render();
                    state.invoke_button(HoverTarget::SettingsButton, true);
                }
                0x25 | 0x27 if state.focused_btn.is_some() => {
                    // Left/Right walk enabled footer buttons like a toolbar
                    let btns: Vec<HoverTarget> = state
                        .footer_buttons()
                        .into_iter()
                        .filter(|&b| state.button_enabled(b))
                        .collect();
                    if let Some(p) = btns.iter().position(|b| Some(*b) == state.focused_btn) {
                        let n = btns.len();
                        let next = if wparam.0 == 0x27 { (p + 1) % n } else { (p + n - 1) % n };
                        state.focused_btn = Some(btns[next]);
                        state.render();
                    }
                }
                0x21 | 0x22 | 0x24 | 0x23 if state.focused_btn.is_some() => {}
                0x25 | 0x27 | 0x21 | 0x22 | 0x24 | 0x23 => {
                    // Left/Right step, PageUp/PageDown 10%, Home/End to 0%/100% (WinUI slider keys)
                    if !state.monitors.is_empty() {
                        let idx = state.focused_idx.unwrap_or(0).min(state.monitors.len() - 1);
                        state.focused_idx = Some(idx);
                        let step = if shift_down { 1 } else { 5 };
                        let cur = state.monitors[idx].brightness as i32;
                        let next = match wparam.0 {
                            0x27 => cur + step,
                            0x25 => cur - step,
                            0x21 => cur + 10,
                            0x22 => cur - 10,
                            0x24 => 0,
                            _ => 100,
                        };
                        state.nudge_brightness(idx, next);
                    }
                }
                0x41 => state.trigger_ambient_measure(), // 'A'
                0x4C if state.monitors.len() > 1 => {
                    // 'L'
                    state.linked = !state.linked;
                    write_reg_bool(w!("Linked"), state.linked);
                    let msg = if state.linked {
                        "Displays linked (L)"
                    } else {
                        "Displays unlinked (L)"
                    };
                    state.set_status(msg, 1800);
                }
                0x52 => {
                    // 'R'
                    state.ddc_retry_scheduled = false;
                    state.request_refresh(true);
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

            if let Some(idx) = state.dragging_idx {
                let b = x_to_brightness_f32(x);
                state.set_monitor_brightness_f32(idx, b, false);
            } else {
                let total_h = compute_flyout_height_dip(state.monitors.len());
                let show_auto = state.ambient_cap != AmbientCapability::None;
                let mut new_hover = hit_test(x, y, state.monitors.len(), total_h, show_auto);
                if matches!(
                    new_hover,
                    HoverTarget::AutoButton | HoverTarget::LinkButton | HoverTarget::RefreshButton | HoverTarget::SettingsButton
                ) && !state.button_enabled(new_hover)
                {
                    new_hover = HoverTarget::None;
                }
                if new_hover != state.hover {
                    state.hover = new_hover;
                    state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            state.tracking_mouse = false;
            if state.dragging_idx.is_none() && state.hover != HoverTarget::None {
                state.hover = HoverTarget::None;
                state.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let scale = state.dpi() as f32 / 96.0;
            let x = ((lparam.0 & 0xFFFF) as i16) as f32 / scale;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32 / scale;
            let total_h = compute_flyout_height_dip(state.monitors.len());
            let show_auto = state.ambient_cap != AmbientCapability::None;
            let target = hit_test(x, y, state.monitors.len(), total_h, show_auto);
            state.focused_idx = None;
            state.focused_btn = None;

            match target {
                HoverTarget::Slider(idx) => {
                    state.dragging_idx = Some(idx);
                    state.drag_anchors = Some(
                        state
                            .monitors
                            .iter()
                            .map(|m| (m.id.clone(), m.visual_brightness))
                            .collect(),
                    );
                    SetCapture(hwnd);
                    let b = x_to_brightness_f32(x);
                    state.set_monitor_brightness_f32(idx, b, false);
                }
                HoverTarget::None => state.render(),
                button => state.invoke_button(button, false),
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.dragging_idx.is_some() {
                state.dragging_idx = None;
                state.drag_anchors = None;
                let _ = ReleaseCapture();
                update_tray_tooltip(hwnd, &state.monitors);
                state.render();
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if state.dragging_idx.is_some() && lparam.0 != hwnd.0 as isize {
                state.dragging_idx = None;
                state.drag_anchors = None;
                update_tray_tooltip(hwnd, &state.monitors);
                if state.visible {
                    state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let shift_down = (GetKeyState(0x10) as u16 & 0x8000) != 0;
            let mag = if shift_down { 1 } else { 5 };

            let mut pt = POINT {
                x: ((lparam.0 & 0xFFFF) as i16) as i32,
                y: (((lparam.0 >> 16) & 0xFFFF) as i16) as i32,
            };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
            let scale = state.dpi() as f32 / 96.0;
            let x = pt.x as f32 / scale;
            let y = pt.y as f32 / scale;

            if let Some(idx) = card_index_at(x, y, state.monitors.len()) {
                if state.wheel_card != Some(idx) {
                    state.wheel_card = Some(idx);
                    state.wheel_accum = 0;
                }
                let (steps, rest) = wheel_steps(state.wheel_accum, delta);
                state.wheel_accum = rest;
                if steps != 0 {
                    state.focused_idx = None;
                    state.focused_btn = None;
                    let cur = state.monitors[idx].brightness as i32;
                    state.nudge_brightness(idx, cur + steps * mag);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 & 0xFFFF;
            match cmd {
                CMD_TOGGLE_STARTUP => {
                    let enabled = startup::is_startup_enabled();
                    startup::set_startup_enabled(!enabled);
                }
                CMD_MEASURE_AMBIENT => {
                    state.trigger_ambient_measure();
                }
                CMD_TOGGLE_WAKE_AUTO => {
                    state.auto_on_wake = !state.auto_on_wake;
                    write_reg_bool(w!("AutoOnWake"), state.auto_on_wake);
                }
                CMD_TOGGLE_LINK => {
                    state.linked = !state.linked;
                    write_reg_bool(w!("Linked"), state.linked);
                    if state.visible {
                        state.render();
                    }
                }
                CMD_REFRESH => {
                    state.ddc_retry_scheduled = false;
                    state.request_refresh(state.visible);
                }
                CMD_EXIT => {
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_GETOBJECT => {
            if uia::is_uia_getobject(lparam) {
                let root = state.uia.root_for_hwnd(hwnd);
                return uia::return_raw_provider(hwnd, wparam, lparam, Some(&root));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_UIA_ACTION => {
            if let Some(action) = decode_action(wparam.0, lparam.0) {
                match action {
                    UiaAction::SetSliderValue { idx, pct_x100 } if !state.menu_open => {
                        if idx < state.monitors.len() {
                            state.set_monitor_brightness_f32(idx, pct_x100 as f32 / 100.0, false);
                            update_tray_tooltip(hwnd, &state.monitors);
                        }
                    }
                    UiaAction::FocusSlider(idx) if state.visible && !state.menu_open => {
                        if idx < state.monitors.len() {
                            if GetForegroundWindow() != hwnd {
                                let _ = SetForegroundWindow(hwnd);
                            }
                            state.focused_btn = None;
                            state.focused_idx = Some(idx);
                            state.render();
                        }
                    }
                    UiaAction::FocusButton(btn) if state.visible && !state.menu_open => {
                        if state.footer_buttons().contains(&btn) && state.button_enabled(btn) {
                            if GetForegroundWindow() != hwnd {
                                let _ = SetForegroundWindow(hwnd);
                            }
                            state.focused_idx = None;
                            state.focused_btn = Some(btn);
                            state.render();
                        }
                    }
                    UiaAction::InvokeButton(btn) if state.visible && !state.menu_open => {
                        if state.footer_buttons().contains(&btn) && state.button_enabled(btn) {
                            state.focused_idx = None;
                            state.focused_btn = Some(btn);
                            state.invoke_button(btn, true);
                        }
                    }
                    UiaAction::ToggleLink if state.visible && !state.menu_open => {
                        if state.monitors.len() > 1 {
                            state.linked = !state.linked;
                            write_reg_bool(w!("Linked"), state.linked);
                            let msg = if state.linked {
                                "Displays linked (L)"
                            } else {
                                "Displays unlinked (L)"
                            };
                            state.set_status(msg, 1800);
                        }
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = uia::return_raw_provider(hwnd, WPARAM(0), LPARAM(0), None);
            uia::disconnect_all_providers();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn add_tray_icon(hwnd: HWND, icon: HICON) -> bool {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY_ICON,
            hIcon: icon,
            ..Default::default()
        };
        let tip: Vec<u16> = "Display Brightness\0".encode_utf16().collect();
        let len = tip.len().min(nid.szTip.len());
        nid.szTip[..len].copy_from_slice(&tip[..len]);

        let added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
        if added {
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
        added
    }
}

fn remove_tray_icon(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn create_monitor_tray_icon(is_dark_taskbar: bool) -> HICON {
    use windows::Win32::Graphics::Gdi::CreateBitmap;

    let size = unsafe { GetSystemMetrics(SM_CXSMICON).clamp(16, 48) };
    let font_height = -((size as f32 * 0.85).round() as i32);

    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(screen_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(hbm_color) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return HICON::default();
        };

        let old_bmp = SelectObject(mem_dc, hbm_color);

        // Render Segoe Fluent Icons U+E706 (Brightness), falling back to Segoe MDL2 Assets on Win10
        let make_icon_font = |name: windows::core::PCWSTR| {
            CreateFontW(
                font_height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_TT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                ANTIALIASED_QUALITY.0 as u32,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                name,
            )
        };
        let mut font = make_icon_font(w!("Segoe Fluent Icons"));
        let mut old_font = SelectObject(mem_dc, font);
        let glyph: [u16; 1] = [0xE706];
        let glyph_z: [u16; 2] = [0xE706, 0];
        let mut glyph_idx = [0xFFFFu16; 1];
        let has_glyph = windows::Win32::Graphics::Gdi::GetGlyphIndicesW(
            mem_dc,
            PCWSTR(glyph_z.as_ptr()),
            1,
            glyph_idx.as_mut_ptr(),
            0x0001, // GGI_MARK_NONEXISTING_GLYPHS
        ) != u32::MAX
            && glyph_idx[0] != 0xFFFF;
        if !has_glyph {
            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(font);
            font = make_icon_font(w!("Segoe MDL2 Assets"));
            old_font = SelectObject(mem_dc, font);
        }

        let _ = SetBkColor(mem_dc, windows::Win32::Foundation::COLORREF(0x00000000));
        let _ = SetTextColor(mem_dc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));

        // Center glyph in the `size x size` canvas (U+E706 = Brightness)
        let mut text_sz = windows::Win32::Foundation::SIZE::default();
        let _ = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(mem_dc, &glyph, &mut text_sz);
        let x = ((size - text_sz.cx) / 2).max(0);
        let y = ((size - text_sz.cy) / 2).max(0);
        let _ = TextOutW(mem_dc, x, y, &glyph);

        let _ = SelectObject(mem_dc, old_font);
        let _ = DeleteObject(font);
        let _ = SelectObject(mem_dc, old_bmp);
        let _ = DeleteDC(mem_dc);

        // Convert white-on-black luminance into premultiplied ARGB
        if !bits.is_null() {
            let len = (size * size) as usize;
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, len);
            let (fg_r, fg_g, fg_b): (u32, u32, u32) = if is_dark_taskbar {
                (255, 255, 255)
            } else {
                (28, 28, 28)
            };
            for p in px.iter_mut() {
                let lum = *p & 0xFF;
                let pr = (fg_r * lum) / 255;
                let pg = (fg_g * lum) / 255;
                let pb = (fg_b * lum) / 255;
                *p = (lum << 24) | (pr << 16) | (pg << 8) | pb;
            }
        }

        let mask_stride = ((size + 31) / 32) * 4;
        let mask_bytes = vec![0u8; (mask_stride * size) as usize];
        let hbm_mask = CreateBitmap(size, size, 1, 1, Some(mask_bytes.as_ptr() as *const _));

        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();

        let _ = DeleteObject(hbm_color);
        let _ = DeleteObject(hbm_mask);
        ReleaseDC(None, screen_dc);
        icon
    }
}

fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn format_tray_tooltip(monitors: &[UiMonitor]) -> String {
    const HEADER: &str = "Display Brightness";
    const MAX_CHARS: usize = 127;
    if monitors.is_empty() {
        return HEADER.to_string();
    }

    // Names carry no number now; add it back only when two displays share a name
    let label = |m: &UiMonitor| -> String {
        if monitors.iter().filter(|o| o.name == m.name).count() > 1 {
            format!("{} ({})", m.name, m.display_number)
        } else {
            m.name.clone()
        }
    };
    let full_parts: Vec<String> = monitors
        .iter()
        .map(|m| format!("{}: {}%", label(m), m.brightness))
        .collect();
    let full = format!("{}\n{}", HEADER, full_parts.join(" · "));
    if full.encode_utf16().count() <= MAX_CHARS {
        return full;
    }

    // Budget per monitor so percentages never get clipped by szTip[128]
    let mut out = String::from(HEADER);
    let max_items = monitors.len().min(4);
    let per_item_budget = ((MAX_CHARS - HEADER.len() - 1) / max_items).saturating_sub(3);
    for (i, m) in monitors.iter().take(max_items).enumerate() {
        let suffix = format!(": {}%", m.brightness);
        let name = label(m);
        let name_budget = per_item_budget.saturating_sub(suffix.len());
        let short_name: String = if name.chars().count() > name_budget && name_budget >= 4 {
            let truncated: String = name.chars().take(name_budget - 1).collect();
            format!("{}…", truncated.trim_end())
        } else {
            name.chars().take(name_budget).collect()
        };
        let sep = if i == 0 { "\n" } else { " · " };
        let candidate = format!("{}{}{}{}", out, sep, short_name, suffix);
        if candidate.encode_utf16().count() <= MAX_CHARS {
            out = candidate;
        } else {
            break;
        }
    }
    out
}

fn update_tray_tooltip(hwnd: HWND, monitors: &[UiMonitor]) {
    let tip_text = format_tray_tooltip(monitors);
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_TIP | NIF_SHOWTIP,
            ..Default::default()
        };
        let tip: Vec<u16> = tip_text.encode_utf16().chain(std::iter::once(0)).collect();
        let len = tip.len().min(nid.szTip.len());
        nid.szTip[..len].copy_from_slice(&tip[..len]);
        nid.szTip[nid.szTip.len() - 1] = 0;
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tooltip_respects_127_utf16_limit() {
        let mons = (1..=4)
            .map(|n| UiMonitor {
                id: format!("id{n}"),
                display_number: n,
                name: format!("Very Long External Monitor Name Model ABCDEFG {n}"),
                brightness: 50 + n * 10,
                visual_brightness: 50.0,
                anim_from: 50.0,
            })
            .collect::<Vec<_>>();
        let tip = format_tray_tooltip(&mons);
        assert!(tip.encode_utf16().count() <= 127);
        assert!(tip.contains("60%"));
        assert!(tip.contains("70%"));
    }

    fn mon(n: u32, name: &str, b: u32) -> UiMonitor {
        UiMonitor {
            id: format!("id{n}"),
            display_number: n,
            name: name.to_string(),
            brightness: b,
            visual_brightness: b as f32,
            anim_from: b as f32,
        }
    }

    #[test]
    fn test_tooltip_numbers_only_twin_displays() {
        let tip = format_tray_tooltip(&[mon(1, "MAG 27CQ6F", 51), mon(2, "Built-in display", 40)]);
        assert!(tip.contains("MAG 27CQ6F: 51%"), "{tip}");
        assert!(tip.contains("Built-in display: 40%"), "{tip}");
        let twins = format_tray_tooltip(&[mon(2, "DELL U2419H", 30), mon(3, "DELL U2419H", 70)]);
        assert!(twins.contains("DELL U2419H (2): 30%"), "{twins}");
        assert!(twins.contains("DELL U2419H (3): 70%"), "{twins}");
    }

    #[test]
    fn test_touchpad_scroll_moves_one_step_per_notch() {
        // Precision touchpad: one notch arrives as 8 events of 15 units
        let (mut acc, mut total) = (0, 0);
        for _ in 0..8 {
            let (steps, rest) = wheel_steps(acc, 15);
            total += steps;
            acc = rest;
        }
        assert_eq!(total, 1);
        assert_eq!(acc, 0);
        // A mouse notch is one event of 120; two notches at once are two steps
        assert_eq!(wheel_steps(0, 120), (1, 0));
        assert_eq!(wheel_steps(0, -240), (-2, 0));
        // Turning back drops leftover travel instead of eating the first notch back
        assert_eq!(wheel_steps(90, -120), (-1, 0));
        assert_eq!(wheel_steps(-60, 30), (0, 30));
    }

    #[test]
    fn test_lid_open_needs_a_real_change() {
        // Windows reports "open" on registration; that is not a lid opening
        assert!(!lid_was_opened(None, true));
        assert!(!lid_was_opened(Some(true), true));
        assert!(!lid_was_opened(Some(true), false));
        assert!(lid_was_opened(Some(false), true));
    }
}

fn read_reg_bool(name: PCWSTR, default: bool) -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\BrightnessTray"),
            0,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return default;
        }
        let mut data: u32 = if default { 1 } else { 0 };
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut reg_type = REG_DWORD;
        let res = RegQueryValueExW(
            hkey,
            name,
            None,
            Some(&mut reg_type),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        if res.is_ok() { data != 0 } else { default }
    }
}

fn write_reg_bool(name: PCWSTR, enabled: bool) {
    unsafe {
        let mut hkey = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\BrightnessTray"),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
        .is_ok()
        {
            let data: u32 = if enabled { 1 } else { 0 };
            let bytes = std::slice::from_raw_parts(
                &data as *const u32 as *const u8,
                std::mem::size_of::<u32>(),
            );
            let _ = RegSetValueExW(hkey, name, 0, REG_DWORD, Some(bytes));
            let _ = RegCloseKey(hkey);
        }
    }
}
