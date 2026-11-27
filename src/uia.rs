use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::core::{implement, Error, IUnknown, IUnknownImpl, Result, BSTR, HRESULT, VARIANT};
use windows::Win32::Foundation::{
    BOOL, E_INVALIDARG, E_OUTOFMEMORY, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Accessibility::{
    IInvokeProvider, IInvokeProvider_Impl, IRangeValueProvider, IRangeValueProvider_Impl,
    IRawElementProviderFragment, IRawElementProviderFragmentRoot,
    IRawElementProviderFragmentRoot_Impl, IRawElementProviderFragment_Impl,
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, IToggleProvider,
    IToggleProvider_Impl, NavigateDirection, NavigateDirection_FirstChild,
    NavigateDirection_LastChild, NavigateDirection_NextSibling, NavigateDirection_Parent,
    NavigateDirection_PreviousSibling, NotificationKind_ActionAborted, NotificationKind_Other,
    NotificationProcessing_ImportantMostRecent, NotificationProcessing_MostRecent,
    OrientationType_Horizontal, ProviderOptions, ProviderOptions_ServerSideProvider,
    ProviderOptions_UseComThreading, StructureChangeType_ChildrenInvalidated, ToggleState,
    ToggleState_Off, ToggleState_On, UIA_AcceleratorKeyPropertyId, UIA_AutomationFocusChangedEventId,
    UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId, UIA_ClassNamePropertyId,
    UIA_ControlTypePropertyId, UIA_E_ELEMENTNOTAVAILABLE, UIA_HasKeyboardFocusPropertyId,
    UIA_HelpTextPropertyId, UIA_InvokePatternId, UIA_IsContentElementPropertyId,
    UIA_IsControlElementPropertyId, UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_IsOffscreenPropertyId, UIA_MenuClosedEventId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_MenuOpenedEventId, UIA_NamePropertyId,
    UIA_OrientationPropertyId, UIA_PaneControlTypeId, UIA_RangeValueIsReadOnlyPropertyId,
    UIA_RangeValueLargeChangePropertyId, UIA_RangeValueMaximumPropertyId,
    UIA_RangeValueMinimumPropertyId, UIA_RangeValuePatternId, UIA_RangeValueSmallChangePropertyId,
    UIA_RangeValueValuePropertyId, UIA_SliderControlTypeId, UIA_TextControlTypeId,
    UIA_TogglePatternId, UIA_ToggleToggleStatePropertyId, UiaAppendRuntimeId,
    UiaClientsAreListening, UiaDisconnectAllProviders, UiaDisconnectProvider,
    UiaHostProviderFromHwnd, UiaRaiseAutomationEvent, UiaRaiseAutomationPropertyChangedEvent,
    UiaRaiseNotificationEvent, UiaRaiseStructureChangedEvent, UiaRect,
    UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageTimeoutW, SystemParametersInfoW, SMTO_ABORTIFHUNG,
    SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_APP,
};

use crate::menu::MenuEntry;
use crate::ui::{
    button_rects, compute_flyout_height_dip, hit_test, slider_hit_rect, HoverTarget, UiMonitor,
    FLYOUT_WIDTH_DIP,
};

pub const WM_UIA_ACTION: u32 = WM_APP + 6;
pub const WM_UIA_MENU_INVOKE: u32 = WM_APP + 7;
pub const WM_UIA_MENU_FOCUS: u32 = WM_APP + 8;

const ACT_SET_VALUE: usize = 1;
const ACT_FOCUS_SLIDER: usize = 2;
const ACT_FOCUS_BUTTON: usize = 3;
const ACT_INVOKE_BUTTON: usize = 4;
const ACT_TOGGLE_LINK: usize = 5;

const EMPTY_STATE_TEXT: &str =
    "No controllable displays found. Turn on DDC/CI in your monitor's menu, then Refresh";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiaAction {
    SetSliderValue { idx: usize, pct_x100: u32 },
    FocusSlider(usize),
    FocusButton(HoverTarget),
    InvokeButton(HoverTarget),
    ToggleLink,
}

fn button_code(btn: HoverTarget) -> Option<usize> {
    match btn {
        HoverTarget::AutoButton => Some(1),
        HoverTarget::LinkButton => Some(2),
        HoverTarget::RefreshButton => Some(3),
        HoverTarget::SettingsButton => Some(4),
        _ => None,
    }
}

fn code_to_button(code: usize) -> Option<HoverTarget> {
    match code {
        1 => Some(HoverTarget::AutoButton),
        2 => Some(HoverTarget::LinkButton),
        3 => Some(HoverTarget::RefreshButton),
        4 => Some(HoverTarget::SettingsButton),
        _ => None,
    }
}

pub fn encode_action(action: UiaAction) -> (usize, isize) {
    match action {
        UiaAction::SetSliderValue { idx, pct_x100 } => {
            let clamped = pct_x100.min(10_000) as usize;
            (ACT_SET_VALUE, (((idx & 0xFFFF) << 16) | clamped) as isize)
        }
        UiaAction::FocusSlider(idx) => (ACT_FOCUS_SLIDER, idx as isize),
        UiaAction::FocusButton(btn) => (ACT_FOCUS_BUTTON, button_code(btn).unwrap_or(0) as isize),
        UiaAction::InvokeButton(btn) => (ACT_INVOKE_BUTTON, button_code(btn).unwrap_or(0) as isize),
        UiaAction::ToggleLink => (ACT_TOGGLE_LINK, 0),
    }
}

pub fn decode_action(wparam: usize, lparam: isize) -> Option<UiaAction> {
    match wparam {
        ACT_SET_VALUE => {
            let raw = lparam as usize;
            let idx = raw >> 16;
            let pct_x100 = (raw & 0xFFFF) as u32;
            (pct_x100 <= 10_000).then_some(UiaAction::SetSliderValue { idx, pct_x100 })
        }
        ACT_FOCUS_SLIDER => (lparam >= 0).then_some(UiaAction::FocusSlider(lparam as usize)),
        ACT_FOCUS_BUTTON => code_to_button(lparam as usize).map(UiaAction::FocusButton),
        ACT_INVOKE_BUTTON => code_to_button(lparam as usize).map(UiaAction::InvokeButton),
        ACT_TOGGLE_LINK => Some(UiaAction::ToggleLink),
        _ => None,
    }
}

pub fn is_screen_reader_active() -> bool {
    let mut active: BOOL = BOOL(0);
    unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENREADER,
            0,
            Some(&mut active as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && active.as_bool()
    }
}

pub fn is_uia_getobject(lparam: LPARAM) -> bool {
    lparam.0 as i32 == UiaRootObjectId
}

pub unsafe fn return_raw_provider(
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    root: Option<&IRawElementProviderSimple>,
) -> LRESULT {
    UiaReturnRawElementProvider(hwnd, wparam, lparam, root)
}

pub unsafe fn disconnect_all_providers() {
    let _ = UiaDisconnectAllProviders();
}

pub fn slider_accessible_name(monitors: &[UiMonitor], idx: usize) -> String {
    let Some(m) = monitors.get(idx) else {
        return String::new();
    };
    if monitors.iter().filter(|o| o.name == m.name).count() > 1 {
        format!("{}, display {}", m.name, m.display_number)
    } else {
        m.name.clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Slider(usize),
    EmptyText,
    Button(HoverTarget),
}

pub fn ordered_nodes(slider_count: usize, show_auto: bool) -> Vec<NodeKind> {
    let mut out = Vec::with_capacity(slider_count.max(1) + 4);
    if slider_count == 0 {
        out.push(NodeKind::EmptyText);
    } else {
        for i in 0..slider_count {
            out.push(NodeKind::Slider(i));
        }
    }
    if show_auto {
        out.push(NodeKind::Button(HoverTarget::AutoButton));
    }
    out.push(NodeKind::Button(HoverTarget::LinkButton));
    out.push(NodeKind::Button(HoverTarget::RefreshButton));
    out.push(NodeKind::Button(HoverTarget::SettingsButton));
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UiaSlider {
    id: String,
    name: String,
    value: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FocusTarget {
    Slider(usize),
    Button(HoverTarget),
}

#[derive(Default)]
struct UiaModel {
    hwnd: isize,
    visible: bool,
    dpi: u32,
    sliders: Vec<UiaSlider>,
    show_auto: bool,
    linked: bool,
    measuring_ambient: bool,
    focus: Option<FocusTarget>,
}

impl UiaModel {
    fn nodes(&self) -> Vec<NodeKind> {
        ordered_nodes(self.sliders.len(), self.show_auto)
    }

    fn contains_node(&self, kind: NodeKind) -> bool {
        match kind {
            NodeKind::Slider(i) => i < self.sliders.len(),
            NodeKind::EmptyText => self.sliders.is_empty(),
            NodeKind::Button(HoverTarget::AutoButton) => self.show_auto,
            NodeKind::Button(
                HoverTarget::LinkButton | HoverTarget::RefreshButton | HoverTarget::SettingsButton,
            ) => true,
            NodeKind::Button(_) => false,
        }
    }

    fn button_enabled(&self, btn: HoverTarget) -> bool {
        match btn {
            HoverTarget::AutoButton => !self.sliders.is_empty() && !self.measuring_ambient,
            HoverTarget::LinkButton => self.sliders.len() > 1,
            HoverTarget::RefreshButton | HoverTarget::SettingsButton => true,
            _ => false,
        }
    }
}

pub struct FlyoutUia {
    shared: Arc<Mutex<UiaModel>>,
    root: Option<IRawElementProviderSimple>,
}

impl FlyoutUia {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            shared: Arc::new(Mutex::new(UiaModel {
                hwnd: hwnd.0 as isize,
                dpi: 96,
                ..Default::default()
            })),
            root: None,
        }
    }

    pub fn root_for_hwnd(&mut self, hwnd: HWND) -> IRawElementProviderSimple {
        if let Ok(mut model) = self.shared.lock() {
            model.hwnd = hwnd.0 as isize;
        }
        self.ensure_root().clone()
    }

    fn ensure_root(&mut self) -> &IRawElementProviderSimple {
        self.root.get_or_insert_with(|| {
            RootProvider {
                shared: self.shared.clone(),
            }
            .into()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        hwnd: HWND,
        visible: bool,
        dpi: u32,
        monitors: &[UiMonitor],
        show_auto: bool,
        linked: bool,
        measuring_ambient: bool,
        focused_idx: Option<usize>,
        focused_btn: Option<HoverTarget>,
    ) {
        let new_sliders: Vec<UiaSlider> = (0..monitors.len())
            .map(|i| UiaSlider {
                id: monitors[i].id.clone(),
                name: slider_accessible_name(monitors, i),
                value: monitors[i].brightness,
            })
            .collect();

        let new_focus = if let Some(b) = focused_btn {
            Some(FocusTarget::Button(b))
        } else {
            focused_idx
                .filter(|&i| i < new_sliders.len())
                .map(FocusTarget::Slider)
        };

        let mut model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let old_visible = model.visible;
        let structure_changed = model.sliders.len() != new_sliders.len()
            || model.show_auto != show_auto
            || model
                .sliders
                .iter()
                .zip(&new_sliders)
                .any(|(a, b)| a.id != b.id || a.name != b.name);

        let value_changes: Vec<(usize, u32, u32)> = if !structure_changed {
            model
                .sliders
                .iter()
                .zip(&new_sliders)
                .enumerate()
                .filter_map(|(i, (a, b))| (a.value != b.value).then_some((i, a.value, b.value)))
                .collect()
        } else {
            Vec::new()
        };

        let link_changed = model.linked != linked;
        let old_linked = model.linked;
        let focus_changed = visible && (new_focus != model.focus || !old_visible);

        model.hwnd = hwnd.0 as isize;
        model.visible = visible;
        model.dpi = dpi.max(96);
        model.sliders = new_sliders;
        model.show_auto = show_auto;
        model.linked = linked;
        model.measuring_ambient = measuring_ambient;
        model.focus = if visible { new_focus } else { None };
        drop(model);

        if !visible || !unsafe { UiaClientsAreListening().as_bool() } {
            return;
        }
        let root = self.root_for_hwnd(hwnd);

        unsafe {
            if structure_changed {
                let _ = UiaRaiseStructureChangedEvent(
                    &root,
                    StructureChangeType_ChildrenInvalidated,
                    std::ptr::null_mut(),
                    0,
                );
            }
            for (idx, old_v, new_v) in value_changes {
                let elem = child_simple(&self.shared, NodeKind::Slider(idx));
                let _ = UiaRaiseAutomationPropertyChangedEvent(
                    &elem,
                    UIA_RangeValueValuePropertyId,
                    &VARIANT::from(old_v as f64),
                    &VARIANT::from(new_v as f64),
                );
            }
            if link_changed {
                let elem = child_simple(&self.shared, NodeKind::Button(HoverTarget::LinkButton));
                let old_t = if old_linked { ToggleState_On.0 } else { ToggleState_Off.0 };
                let new_t = if linked { ToggleState_On.0 } else { ToggleState_Off.0 };
                let _ = UiaRaiseAutomationPropertyChangedEvent(
                    &elem,
                    UIA_ToggleToggleStatePropertyId,
                    &VARIANT::from(old_t),
                    &VARIANT::from(new_t),
                );
            }
            if focus_changed {
                if let Some(target) = new_focus {
                    let kind = match target {
                        FocusTarget::Slider(i) => NodeKind::Slider(i),
                        FocusTarget::Button(b) => NodeKind::Button(b),
                    };
                    let elem = child_simple(&self.shared, kind);
                    let _ = UiaRaiseAutomationEvent(&elem, UIA_AutomationFocusChangedEventId);
                }
            }
        }
    }

    pub fn notify(&mut self, msg: &str, is_error: bool) {
        let (visible, hwnd_raw) = self
            .shared
            .lock()
            .map(|m| (m.visible, m.hwnd))
            .unwrap_or((false, 0));
        if !visible || !unsafe { UiaClientsAreListening().as_bool() } {
            return;
        }
        let root = self.root_for_hwnd(hwnd_from_raw(hwnd_raw));
        let (kind, proc_kind) = if is_error {
            (
                NotificationKind_ActionAborted,
                NotificationProcessing_ImportantMostRecent,
            )
        } else {
            (NotificationKind_Other, NotificationProcessing_MostRecent)
        };
        unsafe {
            let _ = UiaRaiseNotificationEvent(
                &root,
                kind,
                proc_kind,
                &BSTR::from(msg),
                &BSTR::from("brightness-tray-status"),
            );
        }
    }
}

unsafe fn make_runtime_id(parts: &[i32]) -> Result<*mut SAFEARRAY> {
    let psa = SafeArrayCreateVector(VT_I4, 0, parts.len() as u32);
    if psa.is_null() {
        return Err(Error::from(E_OUTOFMEMORY));
    }
    for (idx, val) in parts.iter().enumerate() {
        let i = idx as i32;
        if let Err(e) = SafeArrayPutElement(psa, &i, val as *const i32 as *const c_void) {
            let _ = SafeArrayDestroy(psa);
            return Err(e);
        }
    }
    Ok(psa)
}

fn hwnd_from_raw(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

fn dip_rect_to_screen(hwnd: HWND, dpi: u32, left: f32, top: f32, right: f32, bottom: f32) -> UiaRect {
    let mut origin = POINT { x: 0, y: 0 };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut origin);
    }
    let scale = dpi as f64 / 96.0;
    UiaRect {
        left: origin.x as f64 + left as f64 * scale,
        top: origin.y as f64 + top as f64 * scale,
        width: (right - left).max(0.0) as f64 * scale,
        height: (bottom - top).max(0.0) as f64 * scale,
    }
}

fn send_uia_action_sync(hwnd: HWND, action: UiaAction) {
    let (w, l) = encode_action(action);
    let mut res = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(
            hwnd,
            WM_UIA_ACTION,
            WPARAM(w),
            LPARAM(l),
            SMTO_ABORTIFHUNG,
            250,
            Some(&mut res),
        );
    }
}

fn post_uia_action(hwnd: HWND, action: UiaAction) {
    let (w, l) = encode_action(action);
    unsafe {
        let _ = PostMessageW(hwnd, WM_UIA_ACTION, WPARAM(w), LPARAM(l));
    }
}

fn child_fragment(shared: &Arc<Mutex<UiaModel>>, kind: NodeKind) -> IRawElementProviderFragment {
    match kind {
        NodeKind::Slider(idx) => SliderProvider {
            shared: shared.clone(),
            idx,
        }
        .into(),
        NodeKind::EmptyText => TextProvider {
            shared: shared.clone(),
        }
        .into(),
        NodeKind::Button(HoverTarget::LinkButton) => ToggleButtonProvider {
            shared: shared.clone(),
        }
        .into(),
        NodeKind::Button(btn) => ActionButtonProvider {
            shared: shared.clone(),
            btn,
        }
        .into(),
    }
}

fn child_simple(shared: &Arc<Mutex<UiaModel>>, kind: NodeKind) -> IRawElementProviderSimple {
    match kind {
        NodeKind::Slider(idx) => SliderProvider {
            shared: shared.clone(),
            idx,
        }
        .into(),
        NodeKind::EmptyText => TextProvider {
            shared: shared.clone(),
        }
        .into(),
        NodeKind::Button(HoverTarget::LinkButton) => ToggleButtonProvider {
            shared: shared.clone(),
        }
        .into(),
        NodeKind::Button(btn) => ActionButtonProvider {
            shared: shared.clone(),
            btn,
        }
        .into(),
    }
}

fn navigate_child(
    shared: &Arc<Mutex<UiaModel>>,
    kind: NodeKind,
    dir: NavigateDirection,
) -> Result<IRawElementProviderFragment> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.contains_node(kind) {
        return Err(Error::empty());
    }
    let nodes = model.nodes();
    let Some(pos) = nodes.iter().position(|&n| n == kind) else {
        return Err(Error::empty());
    };
    let next = match dir {
        d if d == NavigateDirection_Parent => {
            drop(model);
            return Ok(RootProvider {
                shared: shared.clone(),
            }
            .into());
        }
        d if d == NavigateDirection_PreviousSibling => pos.checked_sub(1).and_then(|p| nodes.get(p).copied()),
        d if d == NavigateDirection_NextSibling => nodes.get(pos + 1).copied(),
        _ => None,
    };
    drop(model);
    match next {
        Some(target) => Ok(child_fragment(shared, target)),
        None => Err(Error::empty()),
    }
}

fn child_runtime_id(kind: NodeKind) -> Result<*mut SAFEARRAY> {
    let (tag, sub) = match kind {
        NodeKind::Slider(idx) => (1, idx as i32),
        NodeKind::Button(btn) => (2, button_code(btn).unwrap_or(0) as i32),
        NodeKind::EmptyText => (3, 0),
    };
    unsafe { make_runtime_id(&[UiaAppendRuntimeId as i32, tag, sub]) }
}

fn child_bounds(shared: &Arc<Mutex<UiaModel>>, kind: NodeKind) -> Result<UiaRect> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.contains_node(kind) {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    }
    if !model.visible {
        return Ok(UiaRect::default());
    }
    let hwnd = hwnd_from_raw(model.hwnd);
    let dpi = model.dpi;
    let total_h = compute_flyout_height_dip(model.sliders.len());
    let rect_dip = match kind {
        NodeKind::Slider(i) => slider_hit_rect(i),
        NodeKind::EmptyText => windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: 12.0,
            top: 12.0,
            right: FLYOUT_WIDTH_DIP - 12.0,
            bottom: 100.0,
        },
        NodeKind::Button(btn) => button_rects(total_h, model.show_auto)
            .into_iter()
            .find(|(t, _)| *t == btn)
            .map(|(_, r)| r)
            .ok_or_else(|| Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)))?,
    };
    drop(model);
    Ok(dip_rect_to_screen(
        hwnd,
        dpi,
        rect_dip.left,
        rect_dip.top,
        rect_dip.right,
        rect_dip.bottom,
    ))
}

fn child_property(
    shared: &Arc<Mutex<UiaModel>>,
    kind: NodeKind,
    property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
) -> Result<VARIANT> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.contains_node(kind) {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    }
    let visible = model.visible;
    let is_focused = visible
        && match (kind, model.focus) {
            (NodeKind::Slider(i), Some(FocusTarget::Slider(fi))) => i == fi,
            (NodeKind::Button(b), Some(FocusTarget::Button(fb))) => b == fb,
            _ => false,
        };

    let val = match kind {
        NodeKind::Slider(i) => {
            let mon = &model.sliders[i];
            match property_id {
                id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_SliderControlTypeId.0),
                id if id == UIA_NamePropertyId => VARIANT::from(mon.name.as_str()),
                id if id == UIA_AutomationIdPropertyId => {
                    VARIANT::from(format!("slider-{i}").as_str())
                }
                id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
                id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
                id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
                id if id == UIA_HasKeyboardFocusPropertyId => VARIANT::from(is_focused),
                id if id == UIA_IsEnabledPropertyId => VARIANT::from(true),
                id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!visible),
                id if id == UIA_OrientationPropertyId => {
                    VARIANT::from(OrientationType_Horizontal.0)
                }
                id if id == UIA_RangeValueValuePropertyId => VARIANT::from(mon.value as f64),
                id if id == UIA_RangeValueMinimumPropertyId => VARIANT::from(0.0_f64),
                id if id == UIA_RangeValueMaximumPropertyId => VARIANT::from(100.0_f64),
                id if id == UIA_RangeValueSmallChangePropertyId => VARIANT::from(5.0_f64),
                id if id == UIA_RangeValueLargeChangePropertyId => VARIANT::from(10.0_f64),
                id if id == UIA_RangeValueIsReadOnlyPropertyId => VARIANT::from(false),
                _ => VARIANT::default(),
            }
        }
        NodeKind::EmptyText => match property_id {
            id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_TextControlTypeId.0),
            id if id == UIA_NamePropertyId => VARIANT::from(EMPTY_STATE_TEXT),
            id if id == UIA_AutomationIdPropertyId => VARIANT::from("empty-state"),
            id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(false),
            id if id == UIA_HasKeyboardFocusPropertyId => VARIANT::from(false),
            id if id == UIA_IsEnabledPropertyId => VARIANT::from(true),
            id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!visible),
            _ => VARIANT::default(),
        },
        NodeKind::Button(btn) => {
            let enabled = model.button_enabled(btn);
            let (name, auto_id, help, accel): (&str, &str, Option<&str>, Option<&str>) = match btn {
                HoverTarget::AutoButton => ("Auto", "btn-auto", Some("Match room light"), Some("A")),
                HoverTarget::LinkButton => ("Link displays", "btn-link", None, Some("L")),
                HoverTarget::RefreshButton => {
                    ("Refresh displays", "btn-refresh", None, Some("R"))
                }
                HoverTarget::SettingsButton => ("Settings", "btn-settings", None, Some("Menu")),
                _ => ("", "", None, None),
            };
            match property_id {
                id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_ButtonControlTypeId.0),
                id if id == UIA_NamePropertyId => VARIANT::from(name),
                id if id == UIA_AutomationIdPropertyId => VARIANT::from(auto_id),
                id if id == UIA_HelpTextPropertyId => help.map(VARIANT::from).unwrap_or_default(),
                id if id == UIA_AcceleratorKeyPropertyId => {
                    accel.map(VARIANT::from).unwrap_or_default()
                }
                id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
                id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
                id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(enabled),
                id if id == UIA_HasKeyboardFocusPropertyId => VARIANT::from(enabled && is_focused),
                id if id == UIA_IsEnabledPropertyId => VARIANT::from(enabled),
                id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!visible),
                id if id == UIA_ToggleToggleStatePropertyId && btn == HoverTarget::LinkButton => {
                    VARIANT::from(if model.linked {
                        ToggleState_On.0
                    } else {
                        ToggleState_Off.0
                    })
                }
                _ => VARIANT::default(),
            }
        }
    };
    Ok(val)
}

fn child_set_focus(shared: &Arc<Mutex<UiaModel>>, kind: NodeKind) -> Result<()> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.contains_node(kind) {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    }
    let hwnd = hwnd_from_raw(model.hwnd);
    drop(model);
    match kind {
        NodeKind::Slider(i) => send_uia_action_sync(hwnd, UiaAction::FocusSlider(i)),
        NodeKind::Button(b) => send_uia_action_sync(hwnd, UiaAction::FocusButton(b)),
        NodeKind::EmptyText => {}
    }
    Ok(())
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct RootProvider {
    shared: Arc<Mutex<UiaModel>>,
}

impl IRawElementProviderSimple_Impl for RootProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        _pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        Err(Error::empty())
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let visible = model.visible;
        let has_focus = visible && model.focus.is_none();
        let val = match property_id {
            id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_PaneControlTypeId.0),
            id if id == UIA_NamePropertyId => VARIANT::from("Display Brightness"),
            id if id == UIA_ClassNamePropertyId => VARIANT::from("BrightnessTrayFlyoutClass"),
            id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
            id if id == UIA_HasKeyboardFocusPropertyId => VARIANT::from(has_focus),
            id if id == UIA_IsEnabledPropertyId => VARIANT::from(true),
            id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!visible),
            _ => VARIANT::default(),
        };
        Ok(val)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let hwnd = hwnd_from_raw(self.shared.lock().unwrap_or_else(|e| e.into_inner()).hwnd);
        unsafe { UiaHostProviderFromHwnd(hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for RootProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let nodes = model.nodes();
        let target = match direction {
            d if d == NavigateDirection_FirstChild => nodes.first().copied(),
            d if d == NavigateDirection_LastChild => nodes.last().copied(),
            _ => None,
        };
        drop(model);
        match target {
            Some(kind) => Ok(child_fragment(&self.shared, kind)),
            None => Err(Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // HostRawElementProvider delegates to the HWND provider, which supplies the root runtime ID.
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Ok(UiaRect::default());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        let dpi = model.dpi;
        let total_h = compute_flyout_height_dip(model.sliders.len());
        drop(model);
        Ok(dip_rect_to_screen(hwnd, dpi, 0.0, 0.0, FLYOUT_WIDTH_DIP, total_h))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
        }
        let first = model.nodes().into_iter().find(|&n| match n {
            NodeKind::Slider(_) => true,
            NodeKind::Button(b) => model.button_enabled(b),
            NodeKind::EmptyText => false,
        });
        drop(model);
        if let Some(kind) = first {
            return child_set_focus(&self.shared, kind);
        }
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_object().to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for RootProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Err(Error::empty());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        let dpi = model.dpi;
        let slider_count = model.sliders.len();
        let show_auto = model.show_auto;
        drop(model);

        let mut pt = POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        };
        unsafe {
            let _ = ScreenToClient(hwnd, &mut pt);
        }
        let scale = dpi as f32 / 96.0;
        let dip_x = pt.x as f32 / scale;
        let dip_y = pt.y as f32 / scale;
        let total_h = compute_flyout_height_dip(slider_count);
        if dip_x < 0.0 || dip_y < 0.0 || dip_x > FLYOUT_WIDTH_DIP || dip_y > total_h {
            return Err(Error::empty());
        }
        match hit_test(dip_x, dip_y, slider_count, total_h, show_auto) {
            HoverTarget::Slider(i) => Ok(child_fragment(&self.shared, NodeKind::Slider(i))),
            HoverTarget::None
                if slider_count == 0
                    && (12.0..=FLYOUT_WIDTH_DIP - 12.0).contains(&dip_x)
                    && (12.0..=100.0).contains(&dip_y) =>
            {
                Ok(child_fragment(&self.shared, NodeKind::EmptyText))
            }
            HoverTarget::None => Ok(self.to_object().to_interface()),
            btn => Ok(child_fragment(&self.shared, NodeKind::Button(btn))),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Err(Error::empty());
        }
        let focus = model.focus;
        drop(model);
        match focus {
            Some(FocusTarget::Slider(i)) => Ok(child_fragment(&self.shared, NodeKind::Slider(i))),
            Some(FocusTarget::Button(b)) => Ok(child_fragment(&self.shared, NodeKind::Button(b))),
            None => Ok(self.to_object().to_interface()),
        }
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRangeValueProvider)]
struct SliderProvider {
    shared: Arc<Mutex<UiaModel>>,
    idx: usize,
}

impl IRawElementProviderSimple_Impl for SliderProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        if pattern_id == UIA_RangeValuePatternId {
            let pattern: IRangeValueProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        child_property(&self.shared, NodeKind::Slider(self.idx), property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for SliderProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_child(&self.shared, NodeKind::Slider(self.idx), direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        child_runtime_id(NodeKind::Slider(self.idx))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        child_bounds(&self.shared, NodeKind::Slider(self.idx))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        child_set_focus(&self.shared, NodeKind::Slider(self.idx))
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(RootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

impl IRangeValueProvider_Impl for SliderProvider_Impl {
    fn SetValue(&self, val: f64) -> Result<()> {
        if !val.is_finite() || !(0.0..=100.0).contains(&val) {
            return Err(Error::from(E_INVALIDARG));
        }
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if self.idx >= model.sliders.len() {
            return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        drop(model);
        let pct_x100 = (val * 100.0).round().clamp(0.0, 10_000.0) as u32;
        send_uia_action_sync(
            hwnd,
            UiaAction::SetSliderValue {
                idx: self.idx,
                pct_x100,
            },
        );
        Ok(())
    }

    fn Value(&self) -> Result<f64> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        model
            .sliders
            .get(self.idx)
            .map(|s| s.value as f64)
            .ok_or_else(|| Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn Maximum(&self) -> Result<f64> {
        Ok(100.0)
    }

    fn Minimum(&self) -> Result<f64> {
        Ok(0.0)
    }

    fn LargeChange(&self) -> Result<f64> {
        Ok(10.0)
    }

    fn SmallChange(&self) -> Result<f64> {
        Ok(5.0)
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IInvokeProvider)]
struct ActionButtonProvider {
    shared: Arc<Mutex<UiaModel>>,
    btn: HoverTarget,
}

impl IRawElementProviderSimple_Impl for ActionButtonProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        if pattern_id == UIA_InvokePatternId {
            let pattern: IInvokeProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        child_property(&self.shared, NodeKind::Button(self.btn), property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for ActionButtonProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_child(&self.shared, NodeKind::Button(self.btn), direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        child_runtime_id(NodeKind::Button(self.btn))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        child_bounds(&self.shared, NodeKind::Button(self.btn))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        child_set_focus(&self.shared, NodeKind::Button(self.btn))
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(RootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

impl IInvokeProvider_Impl for ActionButtonProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible || !model.contains_node(NodeKind::Button(self.btn)) {
            return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
        }
        if !model.button_enabled(self.btn) {
            return Err(Error::empty());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        drop(model);
        post_uia_action(hwnd, UiaAction::InvokeButton(self.btn));
        Ok(())
    }
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider,
    IToggleProvider
)]
struct ToggleButtonProvider {
    shared: Arc<Mutex<UiaModel>>,
}

impl IRawElementProviderSimple_Impl for ToggleButtonProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        if pattern_id == UIA_InvokePatternId {
            let pattern: IInvokeProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else if pattern_id == UIA_TogglePatternId {
            let pattern: IToggleProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        child_property(
            &self.shared,
            NodeKind::Button(HoverTarget::LinkButton),
            property_id,
        )
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for ToggleButtonProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_child(
            &self.shared,
            NodeKind::Button(HoverTarget::LinkButton),
            direction,
        )
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        child_runtime_id(NodeKind::Button(HoverTarget::LinkButton))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        child_bounds(&self.shared, NodeKind::Button(HoverTarget::LinkButton))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        child_set_focus(&self.shared, NodeKind::Button(HoverTarget::LinkButton))
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(RootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

impl IInvokeProvider_Impl for ToggleButtonProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible || !model.contains_node(NodeKind::Button(HoverTarget::LinkButton)) {
            return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
        }
        if !model.button_enabled(HoverTarget::LinkButton) {
            return Err(Error::empty());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        drop(model);
        post_uia_action(hwnd, UiaAction::InvokeButton(HoverTarget::LinkButton));
        Ok(())
    }
}

impl IToggleProvider_Impl for ToggleButtonProvider_Impl {
    fn Toggle(&self) -> Result<()> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible || !model.contains_node(NodeKind::Button(HoverTarget::LinkButton)) {
            return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
        }
        if !model.button_enabled(HoverTarget::LinkButton) {
            return Err(Error::empty());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        drop(model);
        post_uia_action(hwnd, UiaAction::ToggleLink);
        Ok(())
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        let linked = self.shared.lock().unwrap_or_else(|e| e.into_inner()).linked;
        Ok(if linked { ToggleState_On } else { ToggleState_Off })
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment)]
struct TextProvider {
    shared: Arc<Mutex<UiaModel>>,
}

impl IRawElementProviderSimple_Impl for TextProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        _pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        Err(Error::empty())
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        child_property(&self.shared, NodeKind::EmptyText, property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for TextProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_child(&self.shared, NodeKind::EmptyText, direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        child_runtime_id(NodeKind::EmptyText)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        child_bounds(&self.shared, NodeKind::EmptyText)
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(RootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

// ============================================================================
// Settings / Tray Fluent Menu UIA
// ============================================================================

#[derive(Clone, Debug)]
pub struct MenuUiaItem {
    pub entry_idx: usize,
    pub id: usize,
    pub label: String,
    pub checked: Option<bool>,
    pub rect_dip: (f32, f32, f32, f32),
}

pub fn build_menu_items(
    entries: &[MenuEntry],
    rect_fn: impl Fn(usize) -> Option<(f32, f32, f32, f32)>,
) -> Vec<MenuUiaItem> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            MenuEntry::Item {
                id,
                label,
                checked,
                ..
            } => rect_fn(i).map(|rect_dip| MenuUiaItem {
                entry_idx: i,
                id: *id,
                label: label.clone(),
                checked: *checked,
                rect_dip,
            }),
            MenuEntry::Separator => None,
        })
        .collect()
}

struct MenuUiaModel {
    hwnd: isize,
    dpi: u32,
    width_dip: f32,
    height_dip: f32,
    items: Vec<MenuUiaItem>,
    hover_entry_idx: Option<usize>,
    visible: bool,
}

pub struct MenuUia {
    shared: Arc<Mutex<MenuUiaModel>>,
    root: Option<IRawElementProviderSimple>,
}

impl MenuUia {
    pub fn new(hwnd: HWND, dpi: u32, width_dip: f32, height_dip: f32, items: Vec<MenuUiaItem>) -> Self {
        Self {
            shared: Arc::new(Mutex::new(MenuUiaModel {
                hwnd: hwnd.0 as isize,
                dpi: dpi.max(96),
                width_dip,
                height_dip,
                items,
                hover_entry_idx: None,
                visible: true,
            })),
            root: None,
        }
    }

    pub fn root_for_hwnd(&mut self) -> &IRawElementProviderSimple {
        self.root.get_or_insert_with(|| {
            MenuRootProvider {
                shared: self.shared.clone(),
            }
            .into()
        })
    }

    pub fn sync_focus(&mut self, dpi: u32, hover_entry_idx: Option<usize>) {
        let mut model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let changed = model.hover_entry_idx != hover_entry_idx;
        model.dpi = dpi.max(96);
        model.hover_entry_idx = hover_entry_idx;
        let item_pos = hover_entry_idx.and_then(|eidx| model.items.iter().position(|it| it.entry_idx == eidx));
        drop(model);

        if !changed || !unsafe { UiaClientsAreListening().as_bool() } {
            return;
        }
        if let Some(pos) = item_pos {
            let elem = menu_item_simple(&self.shared, pos);
            unsafe {
                let _ = UiaRaiseAutomationEvent(&elem, UIA_AutomationFocusChangedEventId);
            }
        }
    }

    pub fn raise_opened(&mut self) {
        if !unsafe { UiaClientsAreListening().as_bool() } {
            return;
        }
        let root = self.root_for_hwnd().clone();
        unsafe {
            let _ = UiaRaiseAutomationEvent(&root, UIA_MenuOpenedEventId);
        }
    }

    pub fn raise_closed(&mut self) {
        if let Ok(mut model) = self.shared.lock() {
            model.visible = false;
        }
        if let Some(root) = &self.root {
            if unsafe { UiaClientsAreListening().as_bool() } {
                unsafe {
                    let _ = UiaRaiseAutomationEvent(root, UIA_MenuClosedEventId);
                }
            }
        }
    }

    pub fn disconnect(&mut self) {
        if let Some(root) = self.root.take() {
            unsafe {
                let _ = UiaDisconnectProvider(&root);
            }
        }
    }
}

fn menu_item_fragment(shared: &Arc<Mutex<MenuUiaModel>>, pos: usize) -> IRawElementProviderFragment {
    let has_toggle = shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .items
        .get(pos)
        .is_some_and(|it| it.checked.is_some());
    if has_toggle {
        MenuToggleItemProvider {
            shared: shared.clone(),
            pos,
        }
        .into()
    } else {
        MenuItemProvider {
            shared: shared.clone(),
            pos,
        }
        .into()
    }
}

fn menu_item_simple(shared: &Arc<Mutex<MenuUiaModel>>, pos: usize) -> IRawElementProviderSimple {
    let has_toggle = shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .items
        .get(pos)
        .is_some_and(|it| it.checked.is_some());
    if has_toggle {
        MenuToggleItemProvider {
            shared: shared.clone(),
            pos,
        }
        .into()
    } else {
        MenuItemProvider {
            shared: shared.clone(),
            pos,
        }
        .into()
    }
}

fn menu_item_property(
    shared: &Arc<Mutex<MenuUiaModel>>,
    pos: usize,
    property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
) -> Result<VARIANT> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    let Some(item) = model.items.get(pos) else {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    };
    let focused = model.visible && model.hover_entry_idx == Some(item.entry_idx);
    let val = match property_id {
        id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_MenuItemControlTypeId.0),
        id if id == UIA_NamePropertyId => VARIANT::from(item.label.as_str()),
        id if id == UIA_AutomationIdPropertyId => VARIANT::from(format!("menu-{}", item.id).as_str()),
        id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
        id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
        id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
        id if id == UIA_HasKeyboardFocusPropertyId => VARIANT::from(focused),
        id if id == UIA_IsEnabledPropertyId => VARIANT::from(true),
        id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!model.visible),
        id if id == UIA_ToggleToggleStatePropertyId => {
            if let Some(c) = item.checked {
                VARIANT::from(if c { ToggleState_On.0 } else { ToggleState_Off.0 })
            } else {
                VARIANT::default()
            }
        }
        _ => VARIANT::default(),
    };
    Ok(val)
}

fn menu_item_navigate(
    shared: &Arc<Mutex<MenuUiaModel>>,
    pos: usize,
    dir: NavigateDirection,
) -> Result<IRawElementProviderFragment> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if pos >= model.items.len() {
        return Err(Error::empty());
    }
    let next = match dir {
        d if d == NavigateDirection_Parent => {
            drop(model);
            return Ok(MenuRootProvider {
                shared: shared.clone(),
            }
            .into());
        }
        d if d == NavigateDirection_PreviousSibling => pos.checked_sub(1),
        d if d == NavigateDirection_NextSibling => {
            (pos + 1 < model.items.len()).then_some(pos + 1)
        }
        _ => None,
    };
    drop(model);
    match next {
        Some(p) => Ok(menu_item_fragment(shared, p)),
        None => Err(Error::empty()),
    }
}

fn menu_item_bounds(shared: &Arc<Mutex<MenuUiaModel>>, pos: usize) -> Result<UiaRect> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    let Some(item) = model.items.get(pos) else {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    };
    if !model.visible {
        return Ok(UiaRect::default());
    }
    let hwnd = hwnd_from_raw(model.hwnd);
    let dpi = model.dpi;
    let (l, t, r, b) = item.rect_dip;
    drop(model);
    Ok(dip_rect_to_screen(hwnd, dpi, l, t, r, b))
}

fn post_menu_invoke(shared: &Arc<Mutex<MenuUiaModel>>, pos: usize) -> Result<()> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.visible {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    }
    let Some(item) = model.items.get(pos) else {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    };
    let hwnd = hwnd_from_raw(model.hwnd);
    let cmd_id = item.id;
    drop(model);
    unsafe {
        let _ = PostMessageW(hwnd, WM_UIA_MENU_INVOKE, WPARAM(cmd_id), LPARAM(0));
    }
    Ok(())
}

fn post_menu_focus(shared: &Arc<Mutex<MenuUiaModel>>, pos: usize) -> Result<()> {
    let model = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !model.visible {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    }
    let Some(item) = model.items.get(pos) else {
        return Err(Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)));
    };
    let hwnd = hwnd_from_raw(model.hwnd);
    let entry_idx = item.entry_idx;
    drop(model);
    unsafe {
        let _ = PostMessageW(hwnd, WM_UIA_MENU_FOCUS, WPARAM(entry_idx), LPARAM(0));
    }
    Ok(())
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct MenuRootProvider {
    shared: Arc<Mutex<MenuUiaModel>>,
}

impl IRawElementProviderSimple_Impl for MenuRootProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        _pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        Err(Error::empty())
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let visible = model.visible;
        let val = match property_id {
            id if id == UIA_ControlTypePropertyId => VARIANT::from(UIA_MenuControlTypeId.0),
            id if id == UIA_NamePropertyId => VARIANT::from("Settings"),
            id if id == UIA_ClassNamePropertyId => VARIANT::from("BrightnessTrayFluentMenuClass"),
            id if id == UIA_IsControlElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsContentElementPropertyId => VARIANT::from(true),
            id if id == UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
            id if id == UIA_HasKeyboardFocusPropertyId => {
                VARIANT::from(visible && model.hover_entry_idx.is_none())
            }
            id if id == UIA_IsEnabledPropertyId => VARIANT::from(true),
            id if id == UIA_IsOffscreenPropertyId => VARIANT::from(!visible),
            _ => VARIANT::default(),
        };
        Ok(val)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let hwnd = hwnd_from_raw(self.shared.lock().unwrap_or_else(|e| e.into_inner()).hwnd);
        unsafe { UiaHostProviderFromHwnd(hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for MenuRootProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let len = model.items.len();
        let target = match direction {
            d if d == NavigateDirection_FirstChild && len > 0 => Some(0),
            d if d == NavigateDirection_LastChild && len > 0 => Some(len - 1),
            _ => None,
        };
        drop(model);
        match target {
            Some(pos) => Ok(menu_item_fragment(&self.shared, pos)),
            None => Err(Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Ok(UiaRect::default());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        let dpi = model.dpi;
        let w = model.width_dip;
        let h = model.height_dip;
        drop(model);
        Ok(dip_rect_to_screen(hwnd, dpi, 0.0, 0.0, w, h))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        post_menu_focus(&self.shared, 0)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_object().to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for MenuRootProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Err(Error::empty());
        }
        let hwnd = hwnd_from_raw(model.hwnd);
        let dpi = model.dpi;
        let mut pt = POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        };
        unsafe {
            let _ = ScreenToClient(hwnd, &mut pt);
        }
        let scale = dpi as f32 / 96.0;
        let dip_x = pt.x as f32 / scale;
        let dip_y = pt.y as f32 / scale;
        let hit = model.items.iter().position(|it| {
            let (l, t, r, b) = it.rect_dip;
            dip_x >= l && dip_x <= r && dip_y >= t && dip_y <= b
        });
        let inside = dip_x >= 0.0 && dip_y >= 0.0 && dip_x <= model.width_dip && dip_y <= model.height_dip;
        drop(model);
        match hit {
            Some(pos) => Ok(menu_item_fragment(&self.shared, pos)),
            None if inside => Ok(self.to_object().to_interface()),
            None => Err(Error::empty()),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let model = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !model.visible {
            return Err(Error::empty());
        }
        let pos = model
            .hover_entry_idx
            .and_then(|eidx| model.items.iter().position(|it| it.entry_idx == eidx));
        drop(model);
        match pos {
            Some(p) => Ok(menu_item_fragment(&self.shared, p)),
            None => Ok(self.to_object().to_interface()),
        }
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IInvokeProvider)]
struct MenuItemProvider {
    shared: Arc<Mutex<MenuUiaModel>>,
    pos: usize,
}

impl IRawElementProviderSimple_Impl for MenuItemProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        if pattern_id == UIA_InvokePatternId {
            let pattern: IInvokeProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        menu_item_property(&self.shared, self.pos, property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for MenuItemProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        menu_item_navigate(&self.shared, self.pos, direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        unsafe { make_runtime_id(&[UiaAppendRuntimeId as i32, 10, self.pos as i32]) }
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        menu_item_bounds(&self.shared, self.pos)
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        post_menu_focus(&self.shared, self.pos)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(MenuRootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

impl IInvokeProvider_Impl for MenuItemProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        post_menu_invoke(&self.shared, self.pos)
    }
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider,
    IToggleProvider
)]
struct MenuToggleItemProvider {
    shared: Arc<Mutex<MenuUiaModel>>,
    pos: usize,
}

impl IRawElementProviderSimple_Impl for MenuToggleItemProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(
        &self,
        pattern_id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> Result<IUnknown> {
        if pattern_id == UIA_InvokePatternId {
            let pattern: IInvokeProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else if pattern_id == UIA_TogglePatternId {
            let pattern: IToggleProvider = self.to_object().to_interface();
            Ok(pattern.into())
        } else {
            Err(Error::empty())
        }
    }

    fn GetPropertyValue(
        &self,
        property_id: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> Result<VARIANT> {
        menu_item_property(&self.shared, self.pos, property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for MenuToggleItemProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        menu_item_navigate(&self.shared, self.pos, direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        unsafe { make_runtime_id(&[UiaAppendRuntimeId as i32, 10, self.pos as i32]) }
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        menu_item_bounds(&self.shared, self.pos)
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        post_menu_focus(&self.shared, self.pos)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(MenuRootProvider {
            shared: self.shared.clone(),
        }
        .into())
    }
}

impl IInvokeProvider_Impl for MenuToggleItemProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        post_menu_invoke(&self.shared, self.pos)
    }
}

impl IToggleProvider_Impl for MenuToggleItemProvider_Impl {
    fn Toggle(&self) -> Result<()> {
        post_menu_invoke(&self.shared, self.pos)
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        let checked = self
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .get(self.pos)
            .and_then(|it| it.checked)
            .unwrap_or(false);
        Ok(if checked { ToggleState_On } else { ToggleState_Off })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn action_encode_decode_roundtrip() {
        let cases = [
            UiaAction::SetSliderValue {
                idx: 0,
                pct_x100: 5125,
            },
            UiaAction::SetSliderValue {
                idx: 3,
                pct_x100: 10_000,
            },
            UiaAction::FocusSlider(2),
            UiaAction::FocusButton(HoverTarget::AutoButton),
            UiaAction::FocusButton(HoverTarget::LinkButton),
            UiaAction::InvokeButton(HoverTarget::RefreshButton),
            UiaAction::InvokeButton(HoverTarget::SettingsButton),
            UiaAction::ToggleLink,
        ];
        for a in cases {
            let (w, l) = encode_action(a);
            assert_eq!(decode_action(w, l), Some(a));
        }
        assert_eq!(decode_action(99, 0), None);
        assert_eq!(decode_action(ACT_SET_VALUE, 10_001), None);
        assert_eq!(decode_action(ACT_FOCUS_SLIDER, -1), None);
        assert_eq!(decode_action(ACT_INVOKE_BUTTON, 9), None);
    }

    #[test]
    fn ordered_nodes_with_and_without_auto_and_empty() {
        assert_eq!(
            ordered_nodes(2, true),
            vec![
                NodeKind::Slider(0),
                NodeKind::Slider(1),
                NodeKind::Button(HoverTarget::AutoButton),
                NodeKind::Button(HoverTarget::LinkButton),
                NodeKind::Button(HoverTarget::RefreshButton),
                NodeKind::Button(HoverTarget::SettingsButton),
            ]
        );
        assert_eq!(
            ordered_nodes(1, false),
            vec![
                NodeKind::Slider(0),
                NodeKind::Button(HoverTarget::LinkButton),
                NodeKind::Button(HoverTarget::RefreshButton),
                NodeKind::Button(HoverTarget::SettingsButton),
            ]
        );
        assert_eq!(
            ordered_nodes(0, false),
            vec![
                NodeKind::EmptyText,
                NodeKind::Button(HoverTarget::LinkButton),
                NodeKind::Button(HoverTarget::RefreshButton),
                NodeKind::Button(HoverTarget::SettingsButton),
            ]
        );
    }

    #[test]
    fn slider_name_adds_display_number_only_on_duplicates() {
        let unique = vec![mon(1, "MAG 27CQ6F", 51), mon(2, "Built-in display", 40)];
        assert_eq!(slider_accessible_name(&unique, 0), "MAG 27CQ6F");
        assert_eq!(slider_accessible_name(&unique, 1), "Built-in display");

        let twins = vec![mon(2, "DELL U2419H", 30), mon(3, "DELL U2419H", 70)];
        assert_eq!(slider_accessible_name(&twins, 0), "DELL U2419H, display 2");
        assert_eq!(slider_accessible_name(&twins, 1), "DELL U2419H, display 3");
    }

    #[test]
    fn menu_item_order_skips_separators() {
        let entries = vec![
            MenuEntry::Item {
                id: 1,
                label: "One".into(),
                glyph: 0,
                checked: Some(true),
            },
            MenuEntry::Separator,
            MenuEntry::Item {
                id: 2,
                label: "Two".into(),
                glyph: 0,
                checked: None,
            },
        ];
        let items = build_menu_items(&entries, |idx| Some((0.0, idx as f32 * 37.0, 250.0, 37.0)));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].entry_idx, 0);
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].entry_idx, 2);
        assert_eq!(items[1].checked, None);
    }
}
