use std::collections::HashMap;
use windows::core::{w, BSTR, PCWSTR, VARIANT};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, QueryDisplayConfig, SetMonitorBrightness,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, PHYSICAL_MONITOR,
    QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{BOOL, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows::Win32::System::Com::{
    CoCreateInstance, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE,
};

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendKind {
    InternalWmi,
    DdcCi {
        handle_raw: isize,
        min: u32,
        max: u32,
    },
}

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub id: String,
    pub display_number: u32,
    pub name: String,
    pub brightness: u32,
    pub backend: BackendKind,
}

struct DisplayConfigMeta {
    friendly_name: String,
    is_internal: bool,
}

pub struct MonitorManager {
    pub monitors: Vec<MonitorInfo>,
    pub has_pending_ddc: bool,
    ddc_handles: Vec<PHYSICAL_MONITOR>,
    wmi_services: Option<IWbemServices>,
    wmi_path: Option<BSTR>,
    wmi_in_params_def: Option<IWbemClassObject>,
}

impl Drop for MonitorManager {
    fn drop(&mut self) {
        self.clear_ddc_handles();
    }
}

impl MonitorManager {
    pub fn new() -> Self {
        let wmi_services = connect_wmi().ok();
        let (wmi_path, wmi_in_params_def) = wmi_services
            .as_ref()
            .and_then(prepare_wmi_set_method)
            .map(|(p, d)| (Some(p), Some(d)))
            .unwrap_or((None, None));
        let mut mgr = Self {
            monitors: Vec::new(),
            has_pending_ddc: false,
            ddc_handles: Vec::new(),
            wmi_services,
            wmi_path,
            wmi_in_params_def,
        };
        mgr.refresh();
        mgr
    }

    fn clear_ddc_handles(&mut self) {
        if !self.ddc_handles.is_empty() {
            unsafe {
                let _ = DestroyPhysicalMonitors(&self.ddc_handles);
            }
            self.ddc_handles.clear();
        }
    }

    pub fn refresh(&mut self) {
        self.clear_ddc_handles();
        self.monitors.clear();
        self.has_pending_ddc = false;

        if self.wmi_services.is_none() {
            self.wmi_services = connect_wmi().ok();
        }
        if self.wmi_in_params_def.is_none() {
            if let Some((p, d)) = self.wmi_services.as_ref().and_then(prepare_wmi_set_method) {
                self.wmi_path = Some(p);
                self.wmi_in_params_def = Some(d);
            }
        }

        let config_map = query_display_config_meta();
        let hmonitors = enumerate_hmonitors();
        let wmi_brightness = self
            .wmi_services
            .as_ref()
            .and_then(|svc| get_wmi_brightness(svc));

        for (idx, (hmon, gdi_name)) in hmonitors.into_iter().enumerate() {
            let display_number = parse_display_number(&gdi_name).unwrap_or((idx + 1) as u32);
            let meta = config_map.get(&gdi_name);
            let is_internal = meta.map(|m| m.is_internal).unwrap_or(false);

            // The card badge already shows the display number, so the label carries only the name.
            let display_label =
                display_label(is_internal, meta.map(|m| m.friendly_name.as_str()));

            if is_internal {
                if let Some(cur) = wmi_brightness {
                    self.monitors.push(MonitorInfo {
                        id: gdi_name,
                        display_number,
                        name: display_label,
                        brightness: cur.min(100),
                        backend: BackendKind::InternalWmi,
                    });
                    continue;
                }
            }

            // Try DDC/CI physical monitors for this HMONITOR
            let mut count = 0u32;
            let got_count = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count).is_ok() };
            if got_count && count > 0 {
                let mut phys = vec![PHYSICAL_MONITOR::default(); count as usize];
                if unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut phys).is_ok() } {
                    let mut added_ddc = false;
                    for (p_idx, pm) in phys.iter().enumerate() {
                        let mut min = 0u32;
                        let mut cur = 0u32;
                        let mut max = 100u32;
                        let ok = unsafe {
                            GetMonitorBrightness(pm.hPhysicalMonitor, &mut min, &mut cur, &mut max)
                        };
                        if ok != 0 && max > min {
                            let pct = raw_to_pct(cur, min, max);
                            let id = format!("{}#{}", gdi_name, p_idx);
                            self.monitors.push(MonitorInfo {
                                id,
                                display_number,
                                name: display_label.clone(),
                                brightness: pct,
                                backend: BackendKind::DdcCi {
                                    handle_raw: pm.hPhysicalMonitor.0 as isize,
                                    min,
                                    max,
                                },
                            });
                            added_ddc = true;
                        }
                    }
                    if added_ddc {
                        self.ddc_handles.extend(phys);
                    } else {
                        if !is_internal {
                            self.has_pending_ddc = true;
                        }
                        unsafe {
                            let _ = DestroyPhysicalMonitors(&phys);
                        }
                    }
                } else if !is_internal {
                    self.has_pending_ddc = true;
                }
            }
        }

        // Sort by display_number so [1], [2], ... appear in consistent order
        self.monitors.sort_by_key(|m| m.display_number);
    }

    pub fn set_brightness(&mut self, id: &str, pct: u32) -> bool {
        let pct = pct.min(100);
        let Some(mon) = self.monitors.iter_mut().find(|m| m.id == id) else {
            return false;
        };

        let ok = match &mon.backend {
            BackendKind::InternalWmi => {
                if let (Some(svc), Some(path), Some(def)) =
                    (&self.wmi_services, &self.wmi_path, &self.wmi_in_params_def)
                {
                    set_wmi_brightness_cached(svc, path, def, pct as u8)
                } else {
                    false
                }
            }
            BackendKind::DdcCi { handle_raw, min, max } => {
                let raw = pct_to_raw(pct, *min, *max);
                let h = HANDLE(*handle_raw as *mut core::ffi::c_void);
                unsafe { SetMonitorBrightness(h, raw) != 0 }
            }
        };

        if ok {
            mon.brightness = pct;
        }
        ok
    }
}

pub fn display_label(is_internal: bool, friendly_name: Option<&str>) -> String {
    if is_internal {
        return "Built-in display".to_string();
    }
    match friendly_name.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "External display".to_string(),
    }
}

pub fn raw_to_pct(cur: u32, min: u32, max: u32) -> u32 {
    if max <= min {
        return 0;
    }
    let span = max - min;
    (((cur.saturating_sub(min)) * 100 + span / 2) / span).min(100)
}

pub fn pct_to_raw(pct: u32, min: u32, max: u32) -> u32 {
    if max <= min {
        return min;
    }
    let pct = pct.min(100);
    min + ((max - min) * pct + 50) / 100
}

fn parse_display_number(gdi_name: &str) -> Option<u32> {
    gdi_name
        .trim_start_matches(r"\\.\DISPLAY")
        .parse::<u32>()
        .ok()
}

fn enumerate_hmonitors() -> Vec<(HMONITOR, String)> {
    unsafe extern "system" fn callback(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<(HMONITOR, String)>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut _).as_bool() {
            let end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            let name = String::from_utf16_lossy(&info.szDevice[..end]);
            list.push((hmon, name));
        }
        BOOL(1)
    }

    let mut list: Vec<(HMONITOR, String)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(callback),
            LPARAM(&mut list as *mut _ as isize),
        );
    }
    list
}

fn query_display_config_meta() -> HashMap<String, DisplayConfigMeta> {
    let mut map = HashMap::new();
    unsafe {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return map;
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        ) != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return map;
        }

        for path in &paths[..path_count as usize] {
            let mut src_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src_name.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src_name.header.adapterId = path.sourceInfo.adapterId;
            src_name.header.id = path.sourceInfo.id;

            if DisplayConfigGetDeviceInfo(&mut src_name.header) != 0 {
                continue;
            }
            let src_end = src_name
                .viewGdiDeviceName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(src_name.viewGdiDeviceName.len());
            let gdi_name = String::from_utf16_lossy(&src_name.viewGdiDeviceName[..src_end]);

            let mut tgt_name = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt_name.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt_name.header.adapterId = path.targetInfo.adapterId;
            tgt_name.header.id = path.targetInfo.id;

            let friendly_name = if DisplayConfigGetDeviceInfo(&mut tgt_name.header) == 0 {
                let end = tgt_name
                    .monitorFriendlyDeviceName
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(tgt_name.monitorFriendlyDeviceName.len());
                String::from_utf16_lossy(&tgt_name.monitorFriendlyDeviceName[..end])
            } else {
                String::new()
            };

            let tech = path.targetInfo.outputTechnology;
            let is_internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS;

            map.entry(gdi_name).or_insert(DisplayConfigMeta {
                friendly_name,
                is_internal,
            });
        }
    }
    map
}

fn connect_wmi() -> windows::core::Result<IWbemServices> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(
            &BSTR::from(r"ROOT\WMI"),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(services)
    }
}

fn get_wmi_brightness(services: &IWbemServices) -> Option<u32> {
    unsafe {
        let enumerator = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness"),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .ok()?;

        let mut objs: [Option<IWbemClassObject>; 1] = [None];
        let mut returned = 0u32;
        if enumerator.Next(WBEM_INFINITE, &mut objs, &mut returned).is_ok() && returned > 0 {
            if let Some(obj) = &objs[0] {
                let mut val = VARIANT::default();
                if obj.Get(w!("CurrentBrightness"), 0, &mut val, None, None).is_ok() {
                    let raw = val.as_raw();
                    let vt = raw.Anonymous.Anonymous.vt;
                    let b = match vt {
                        17 => Some(raw.Anonymous.Anonymous.Anonymous.bVal as u32), // VT_UI1
                        2 => Some(raw.Anonymous.Anonymous.Anonymous.iVal.max(0) as u32), // VT_I2
                        3 => Some(raw.Anonymous.Anonymous.Anonymous.lVal.max(0) as u32), // VT_I4
                        19 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal), // VT_UI4
                        _ => u32::try_from(&val)
                            .ok()
                            .or_else(|| i32::try_from(&val).ok().map(|v| v.max(0) as u32)),
                    };
                    return b;
                }
            }
        }
        None
    }
}

fn prepare_wmi_set_method(services: &IWbemServices) -> Option<(BSTR, IWbemClassObject)> {
    unsafe {
        let enumerator = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT __PATH FROM WmiMonitorBrightnessMethods"),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .ok()?;

        let mut objs: [Option<IWbemClassObject>; 1] = [None];
        let mut returned = 0u32;
        if enumerator.Next(WBEM_INFINITE, &mut objs, &mut returned).is_err() || returned == 0 {
            return None;
        }
        let instance = objs[0].as_ref()?;

        let mut path_var = VARIANT::default();
        if instance.Get(w!("__PATH"), 0, &mut path_var, None, None).is_err() {
            return None;
        }
        let path_bstr = BSTR::try_from(&path_var).ok()?;

        let mut class_obj: Option<IWbemClassObject> = None;
        services
            .GetObject(
                &BSTR::from("WmiMonitorBrightnessMethods"),
                Default::default(),
                None,
                Some(&mut class_obj),
                None,
            )
            .ok()?;
        let class_obj = class_obj?;

        let mut in_params_def: Option<IWbemClassObject> = None;
        class_obj
            .GetMethod(w!("WmiSetBrightness"), 0, &mut in_params_def, std::ptr::null_mut())
            .ok()?;
        let in_params_def = in_params_def?;

        Some((path_bstr, in_params_def))
    }
}

fn set_wmi_brightness_cached(
    services: &IWbemServices,
    path_bstr: &BSTR,
    in_params_def: &IWbemClassObject,
    brightness: u8,
) -> bool {
    unsafe {
        let Ok(in_params) = in_params_def.SpawnInstance(0) else {
            return false;
        };

        let timeout_var = VARIANT::from(0i32);
        let brightness_var = VARIANT::from(brightness as i32);
        if in_params.Put(w!("Timeout"), 0, &timeout_var, 0).is_err() {
            return false;
        }
        if in_params.Put(w!("Brightness"), 0, &brightness_var, 0).is_err() {
            return false;
        }

        services
            .ExecMethod(
                path_bstr,
                &BSTR::from("WmiSetBrightness"),
                Default::default(),
                None,
                Some(&in_params),
                None,
                None,
            )
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    #[test]
    fn test_monitor_discovery() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mgr = MonitorManager::new();
        println!("Detected {} monitors:", mgr.monitors.len());
        for m in &mgr.monitors {
            println!("  [{}] {} -> {}% ({:?})", m.display_number, m.name, m.brightness, m.backend);
        }
        if std::env::var_os("CI").is_none() {
            assert!(!mgr.monitors.is_empty(), "Expected at least one controllable monitor");
        }
        drop(mgr);
        unsafe {
            CoUninitialize();
        }
    }

    #[test]
    fn test_display_label_has_no_number() {
        assert_eq!(display_label(false, Some("MAG 27CQ6F")), "MAG 27CQ6F");
        assert_eq!(display_label(false, Some("  ")), "External display");
        assert_eq!(display_label(false, None), "External display");
        assert_eq!(display_label(true, Some("Generic PnP")), "Built-in display");
    }

    #[test]
    fn test_ddc_rounding_roundtrip() {
        // High-resolution VCP ranges (span >= 100) must round-trip every percentage identically
        for pct in 0..=100 {
            assert_eq!(raw_to_pct(pct_to_raw(pct, 0, 100), 0, 100), pct);
            assert_eq!(raw_to_pct(pct_to_raw(pct, 0, 255), 0, 255), pct);
        }
        // Coarser VCP ranges (span < 100) must round-trip every hardware step without downward drift
        for raw in 10..=90 {
            let pct = raw_to_pct(raw, 10, 90);
            assert_eq!(pct_to_raw(pct, 10, 90), raw);
        }
    }

    #[test]
    fn test_wmi_set_brightness() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut mgr = MonitorManager::new();
        let internal = mgr
            .monitors
            .iter()
            .find(|m| matches!(m.backend, BackendKind::InternalWmi))
            .cloned();
        if let Some(mon) = internal {
            let ok = mgr.set_brightness(&mon.id, mon.brightness);
            println!("set_brightness({}, {}) -> {}", mon.id, mon.brightness, ok);
            assert!(ok, "WMI set_brightness failed");
        } else {
            println!("No internal monitor found");
        }
        drop(mgr);
        unsafe {
            CoUninitialize();
        }
    }
}
