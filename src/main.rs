#![windows_subsystem = "windows"]

mod ambient;
mod camera;
mod monitors;
mod startup;
mod theme;
mod ui;

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
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
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics,
    KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
    TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, HICON,
    HWND_TOPMOST, ICONINFO, IDC_ARROW, IDC_HAND, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, MSG, SM_CXSMICON, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NULL, WM_PAINT, WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_TIMER, DEVICE_NOTIFY_WINDOW_HANDLE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ambient::AmbientCapability;
use monitors::{BackendKind, MonitorManager};
use theme::ThemeColors;
use ui::{
    apply_win11_backdrop, card_index_at, compute_flyout_height_dip, compute_flyout_position,
    hit_test, x_to_brightness_f32, HoverTarget, Renderer, UiMonitor, FLYOUT_WIDTH_DIP,
};

const WM_TRAY_ICON: u32 = WM_APP + 1;
const WM_MONITORS_UPDATED: u32 = WM_APP + 2;
const WM_AUTO_BRIGHTNESS_RESULT: u32 = WM_APP + 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TRAY_ICON_ID: u32 = 1;
const TIMER_REBUILD_TRAY: usize = 100;
const TIMER_WAKE_AUTO: usize = 102;
const TIMER_ANIM_SLIDER: usize = 103;
const TIMER_AMBIENT_TIMEOUT: usize = 104;
const TIMER_RETRY_TRAY: usize = 105;

const CMD_TOGGLE_STARTUP: usize = 1001;
const CMD_TOGGLE_LINK: usize = 1002;
const CMD_REFRESH: usize = 1003;
const CMD_EXIT: usize = 1004;
const CMD_MEASURE_AMBIENT: usize = 1005;
const CMD_TOGGLE_WAKE_AUTO: usize = 1006;

enum WorkerCmd {
    Refresh,
    SetBrightness { id: String, brightness: u32 },
    Shutdown,
}

struct AppState {
    hwnd: HWND,
    tray_icon: HICON,
    taskbar_created_msg: u32,
    theme: ThemeColors,
    renderer: Option<Renderer>,
    monitors: Vec<UiMonitor>,
    shared_monitors: Arc<Mutex<Vec<UiMonitor>>>,
    latest_targets: Arc<Mutex<HashMap<String, u32>>>,
    worker_tx: Sender<WorkerCmd>,
    hover: HoverTarget,
    dragging_idx: Option<usize>,
    linked: bool,
    ambient_cap: AmbientCapability,
    measuring_ambient: bool,
    auto_on_wake: bool,
    visible: bool,
    menu_open: bool,
    anchor_x: i32,
    last_show_time: Instant,
    last_hide_time: Instant,
    anim_start: Option<Instant>,
    tracking_mouse: bool,
}

impl AppState {
    fn dpi(&self) -> u32 {
        unsafe {
            let d = GetDpiForWindow(self.hwnd);
            if d == 0 { 96 } else { d }
        }
    }

    fn size_px(&self) -> (u32, u32) {
        let dpi = self.dpi() as f32;
        let w = (FLYOUT_WIDTH_DIP * dpi / 96.0).round() as u32;
        let h_dip = compute_flyout_height_dip(self.monitors.len());
        let h = (h_dip * dpi / 96.0).round() as u32;
        (w.max(100), h.max(100))
    }

    fn render(&self) {
        if let Some(r) = &self.renderer {
            let _ = r.render(
                &self.monitors,
                &self.theme,
                self.hover,
                self.dragging_idx,
                self.linked,
                self.measuring_ambient,
                self.ambient_cap != AmbientCapability::None,
                self.dpi() as f32,
            );
        }
    }

    fn trigger_ambient_measure(&mut self) {
        if self.ambient_cap == AmbientCapability::None {
            return;
        }
        if !self.measuring_ambient {
            self.measuring_ambient = true;
            let hwnd_raw = self.hwnd.0 as isize;
            unsafe {
                let _ = SetTimer(self.hwnd, TIMER_AMBIENT_TIMEOUT, 3500, None);
            }
            std::thread::spawn(move || {
                let measured = ambient::measure_ambient_brightness().unwrap_or(u32::MAX);
                unsafe {
                    let _ = PostMessageW(
                        HWND(hwnd_raw as *mut core::ffi::c_void),
                        WM_AUTO_BRIGHTNESS_RESULT,
                        WPARAM(measured as usize),
                        LPARAM(0),
                    );
                }
            });
            if self.visible {
                self.render();
            }
        }
    }

    fn resize_and_position(&mut self) {
        let dpi = self.dpi();
        let (w, h) = self.size_px();
        let (_, y) = compute_flyout_position(w as i32, h as i32, dpi);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                self.anchor_x,
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

    fn show_flyout(&mut self) {
        self.theme = ThemeColors::load();
        apply_win11_backdrop(self.hwnd, self.theme.is_dark);
        let _ = self.worker_tx.send(WorkerCmd::Refresh);

        let dpi = self.dpi();
        let (w, h) = self.size_px();
        let (x, y) = compute_flyout_position(w as i32, h as i32, dpi);
        self.anchor_x = x;
        if let Some(r) = &mut self.renderer {
            let _ = r.resize(w, h, dpi as f32);
        }
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
        self.visible = true;
        self.last_show_time = Instant::now();
    }

    fn hide_flyout(&mut self) {
        if self.visible {
            self.visible = false;
            self.dragging_idx = None;
            self.hover = HoverTarget::None;
            self.last_hide_time = Instant::now();
            unsafe {
                let _ = ReleaseCapture();
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    fn toggle_flyout(&mut self) {
        if self.visible {
            self.hide_flyout();
        } else if self.last_hide_time.elapsed() > Duration::from_millis(180) {
            self.show_flyout();
        }
    }

    fn set_monitor_brightness_f32(&mut self, idx: usize, new_val_f32: f32) {
        let new_val_f32 = new_val_f32.clamp(0.0, 100.0);
        let new_val = new_val_f32.round() as u32;
        let Some(old_val_f32) = self.monitors.get(idx).map(|m| m.visual_brightness) else {
            return;
        };
        let old_val = self.monitors[idx].brightness;
        let delta_f32 = new_val_f32 - old_val_f32;

        if self.linked {
            for (i, mon) in self.monitors.iter_mut().enumerate() {
                let target_f32 = if i == idx {
                    new_val_f32
                } else {
                    (mon.visual_brightness + delta_f32).clamp(0.0, 100.0)
                };
                mon.visual_brightness = target_f32;
                let target = target_f32.round() as u32;
                if mon.brightness != target {
                    mon.brightness = target;
                    if let Ok(mut map) = self.latest_targets.lock() {
                        map.insert(mon.id.clone(), target);
                    }
                    let _ = self.worker_tx.send(WorkerCmd::SetBrightness {
                        id: mon.id.clone(),
                        brightness: target,
                    });
                }
            }
        } else if let Some(mon) = self.monitors.get_mut(idx) {
            mon.visual_brightness = new_val_f32;
            if old_val != new_val {
                mon.brightness = new_val;
                if let Ok(mut map) = self.latest_targets.lock() {
                    map.insert(mon.id.clone(), new_val);
                }
                let _ = self.worker_tx.send(WorkerCmd::SetBrightness {
                    id: mon.id.clone(),
                    brightness: new_val,
                });
            }
        }

        self.render();
    }

    fn show_context_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let startup_flags = MF_STRING
                | if startup::is_startup_enabled() {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
            let link_flags = MF_STRING | if self.linked { MF_CHECKED } else { MF_UNCHECKED };
            let wake_flags = MF_STRING | if self.auto_on_wake { MF_CHECKED } else { MF_UNCHECKED };

            if self.ambient_cap != AmbientCapability::None {
                let _ = AppendMenuW(menu, MF_STRING, CMD_MEASURE_AMBIENT, w!("Match room light (Auto)"));
                let _ = AppendMenuW(menu, wake_flags, CMD_TOGGLE_WAKE_AUTO, w!("Auto-adjust on wake / lid open"));
            }
            let _ = AppendMenuW(menu, link_flags, CMD_TOGGLE_LINK, w!("Link monitors"));
            let _ = AppendMenuW(menu, startup_flags, CMD_TOGGLE_STARTUP, w!("Run at Windows startup"));
            let _ = AppendMenuW(menu, MF_STRING, CMD_REFRESH, w!("Refresh monitors"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Exit"));

            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut pt);
            self.menu_open = true;
            let _ = SetForegroundWindow(self.hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                0,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0));
            self.menu_open = false;
            let _ = DestroyMenu(menu);
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

        // Single-instance guard
        let _mutex = CreateMutexW(None, true, w!("Local\\BrightnessTraySingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CoUninitialize();
            return;
        }

        let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        let class_name = w!("BrightnessTrayFlyoutClass");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let shared_monitors: Arc<Mutex<Vec<UiMonitor>>> = Arc::new(Mutex::new(Vec::new()));
        let latest_targets: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
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
            shared_monitors: shared_monitors.clone(),
            latest_targets: latest_targets.clone(),
            worker_tx: worker_tx.clone(),
            hover: HoverTarget::None,
            dragging_idx: None,
            linked: false,
            ambient_cap: ambient::detect_capability(),
            measuring_ambient: false,
            auto_on_wake: read_auto_on_wake(),
            visible: false,
            menu_open: false,
            anchor_x: 0,
            last_show_time: Instant::now() - Duration::from_secs(10),
            last_hide_time: Instant::now() - Duration::from_secs(10),
            anim_start: None,
            tracking_mouse: false,
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
        apply_win11_backdrop(hwnd, state.theme.is_dark);
        let (w, h) = state.size_px();
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
        let shared_for_worker = shared_monitors.clone();
        let targets_for_worker = latest_targets.clone();
        std::thread::spawn(move || {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut mgr = MonitorManager::new();
            publish_monitors(&mgr, &shared_for_worker, hwnd_raw);

            while let Ok(first_cmd) = worker_rx.recv() {
                let mut do_refresh = false;
                let mut pending_sets: HashMap<String, u32> = HashMap::new();
                let mut should_exit = false;

                let mut process_cmd = |cmd: WorkerCmd| match cmd {
                    WorkerCmd::Refresh => do_refresh = true,
                    WorkerCmd::SetBrightness { id, brightness } => {
                        pending_sets.insert(id, brightness);
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
                if do_refresh {
                    mgr.refresh();
                    publish_monitors(&mgr, &shared_for_worker, hwnd_raw);
                }
                for (id, initial_target) in pending_sets {
                    let is_wmi = mgr
                        .monitors
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| matches!(m.backend, BackendKind::InternalWmi))
                        .unwrap_or(false);
                    let current = mgr
                        .monitors
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.brightness)
                        .unwrap_or(initial_target);

                    let latest_for_id = || -> u32 {
                        targets_for_worker
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&id).copied())
                            .unwrap_or(initial_target)
                    };

                    let target = latest_for_id();
                    if current.abs_diff(target) > 3 {
                        let diff = current.abs_diff(target);
                        let steps = if is_wmi { diff.min(32) } else { diff.min(22) };
                        let delay_ms = if is_wmi { 22 } else { 32 };
                        for s in 1..=steps {
                            if latest_for_id() != target {
                                mgr.set_brightness(&id, latest_for_id());
                                break;
                            }
                            let t = s as f32 / steps as f32;
                            // Ease-in-out cubic: gentle start, smooth middle, soft settle (~700ms)
                            let eased = ease_in_out_cubic(t);
                            let val = (current as f32 + (target as f32 - current as f32) * eased)
                                .round()
                                .clamp(0.0, 100.0) as u32;
                            mgr.set_brightness(&id, val);
                            if s < steps {
                                std::thread::sleep(Duration::from_millis(delay_ms));
                            }
                        }
                    } else {
                        mgr.set_brightness(&id, target);
                        if !is_wmi {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    }
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

        let _ = worker_tx.send(WorkerCmd::Shutdown);
        remove_tray_icon(hwnd);
        if !state.tray_icon.is_invalid() {
            let _ = DestroyIcon(state.tray_icon);
        }
        CoUninitialize();
    }
}

fn publish_monitors(mgr: &MonitorManager, shared: &Arc<Mutex<Vec<UiMonitor>>>, hwnd_raw: isize) {
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
        *guard = list;
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
            let notif = (lparam.0 & 0xFFFF) as u32;
            match notif {
                WM_LBUTTONUP => {
                    state.toggle_flyout();
                }
                WM_RBUTTONUP => {
                    state.show_context_menu();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_MONITORS_UPDATED => {
            if state.dragging_idx.is_none() {
                if let Ok(guard) = state.shared_monitors.lock() {
                    state.monitors = guard.clone();
                }
                if state.visible {
                    state.resize_and_position();
                    state.render();
                }
            }
            LRESULT(0)
        }
        WM_AUTO_BRIGHTNESS_RESULT => {
            let _ = KillTimer(hwnd, TIMER_AMBIENT_TIMEOUT);
            state.measuring_ambient = false;
            let pct = wparam.0 as u32;
            if pct <= 100 {
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
                        });
                    }
                }
                if state.visible {
                    state.anim_start = Some(Instant::now());
                    let _ = SetTimer(hwnd, TIMER_ANIM_SLIDER, 16, None);
                } else {
                    for mon in &mut state.monitors {
                        mon.visual_brightness = pct as f32;
                    }
                }
            }
            if state.visible {
                state.render();
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            let _ = state.worker_tx.send(WorkerCmd::Refresh);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
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
            } else if wparam.0 == TIMER_WAKE_AUTO {
                let _ = KillTimer(hwnd, TIMER_WAKE_AUTO);
                let _ = state.worker_tx.send(WorkerCmd::Refresh);
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
                    if state.visible {
                        state.render();
                    }
                }
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let evt = wparam.0 as u32;
            let mut should_schedule = evt == PBT_APMRESUMEAUTOMATIC || evt == PBT_APMRESUMESUSPEND;
            if evt == PBT_POWERSETTINGCHANGE && lparam.0 != 0 {
                let pbs = &*(lparam.0 as *const POWERBROADCAST_SETTING);
                if pbs.PowerSetting == GUID_LIDSWITCH_STATE_CHANGE
                    && pbs.DataLength >= 1
                    && pbs.Data[0] == 1
                {
                    should_schedule = true;
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
        WM_SETCURSOR => {
            let cursor_id = if state.hover != HoverTarget::None || state.dragging_idx.is_some() {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            if let Ok(hcur) = LoadCursorW(None, cursor_id) {
                SetCursor(hcur);
            }
            LRESULT(1)
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
                state.set_monitor_brightness_f32(idx, b);
            } else {
                let total_h = compute_flyout_height_dip(state.monitors.len());
                let show_auto = state.ambient_cap != AmbientCapability::None;
                let new_hover = hit_test(x, y, state.monitors.len(), total_h, show_auto);
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

            match target {
                HoverTarget::Slider(idx) => {
                    state.dragging_idx = Some(idx);
                    SetCapture(hwnd);
                    let b = x_to_brightness_f32(x);
                    state.set_monitor_brightness_f32(idx, b);
                }
                HoverTarget::AutoButton => {
                    state.trigger_ambient_measure();
                }
                HoverTarget::LinkButton => {
                    state.linked = !state.linked;
                    state.render();
                }
                HoverTarget::RefreshButton => {
                    let _ = state.worker_tx.send(WorkerCmd::Refresh);
                }
                HoverTarget::SettingsButton => {
                    state.show_context_menu();
                }
                HoverTarget::None => {}
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.dragging_idx.is_some() {
                state.dragging_idx = None;
                let _ = ReleaseCapture();
                state.render();
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let step = if delta > 0 { 5 } else { -5 };

            let mut pt = windows::Win32::Foundation::POINT {
                x: ((lparam.0 & 0xFFFF) as i16) as i32,
                y: (((lparam.0 >> 16) & 0xFFFF) as i16) as i32,
            };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
            let scale = state.dpi() as f32 / 96.0;
            let x = pt.x as f32 / scale;
            let y = pt.y as f32 / scale;

            if let Some(idx) = card_index_at(x, y, state.monitors.len()) {
                let cur = state.monitors[idx].brightness as i32;
                let next = (cur + step).clamp(0, 100) as f32;
                state.set_monitor_brightness_f32(idx, next);
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
                    write_auto_on_wake(state.auto_on_wake);
                }
                CMD_TOGGLE_LINK => {
                    state.linked = !state.linked;
                    if state.visible {
                        state.render();
                    }
                }
                CMD_REFRESH => {
                    let _ = state.worker_tx.send(WorkerCmd::Refresh);
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
        WM_DESTROY => {
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

fn read_auto_on_wake() -> bool {
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
            return false;
        }
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut reg_type = REG_DWORD;
        let res = RegQueryValueExW(
            hkey,
            w!("AutoOnWake"),
            None,
            Some(&mut reg_type),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        if res.is_ok() { data != 0 } else { false }
    }
}

fn write_auto_on_wake(enabled: bool) {
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
            let _ = RegSetValueExW(hkey, w!("AutoOnWake"), 0, REG_DWORD, Some(bytes));
            let _ = RegCloseKey(hkey);
        }
    }
}
