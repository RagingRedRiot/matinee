//! Reading apps' accessibility trees through UI Automation.
//!
//! UI Automation is Windows' assistive-technology interface, the same one a
//! screen reader uses to find controls by name and role. matinee reads each
//! app window's control view: the meaningful controls, named and typed, with
//! the positions and sizes the compositor needs to aim at them.
//!
//! Control types become the AT-SPI role names the rest of matinee already
//! speaks, so one storyboard names controls the same way on either platform.
//! Positions come back in device pixels relative to the window, which
//! [`crate::platform::in_points`] divides by the scale, as on Linux.

use super::wm;
use crate::platform::Node;
use crate::platform::TextRect;
use windows::Win32::UI::Accessibility::{
    IUIAutomationInvokePattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
    UIA_InvokePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
};
use windows::Win32::{
    Foundation::RPC_E_CHANGED_MODE,
    System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationRangeValuePattern,
        IUIAutomationTextPattern, IUIAutomationTreeWalker, IUIAutomationValuePattern,
        UIA_ButtonControlTypeId, UIA_CONTROLTYPE_ID, UIA_CalendarControlTypeId,
        UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_DataGridControlTypeId,
        UIA_DataItemControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
        UIA_GroupControlTypeId, UIA_HeaderControlTypeId, UIA_HeaderItemControlTypeId,
        UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId, UIA_ListControlTypeId,
        UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
        UIA_MenuItemControlTypeId, UIA_PaneControlTypeId, UIA_ProgressBarControlTypeId,
        UIA_RadioButtonControlTypeId, UIA_RangeValuePatternId, UIA_ScrollBarControlTypeId,
        UIA_SeparatorControlTypeId, UIA_SliderControlTypeId, UIA_SpinnerControlTypeId,
        UIA_SplitButtonControlTypeId, UIA_StatusBarControlTypeId, UIA_TabControlTypeId,
        UIA_TabItemControlTypeId, UIA_TableControlTypeId, UIA_TextControlTypeId, UIA_TextPatternId,
        UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId, UIA_ToolTipControlTypeId,
        UIA_TreeControlTypeId, UIA_TreeItemControlTypeId, UIA_ValuePatternId,
        UIA_WindowControlTypeId,
    },
};
use windows::core::Interface;

/// Stop walking a window's tree after this many nodes.
const MAX_NODES: usize = 5000;

pub struct A11y {
    automation: IUIAutomation,
    /// The field the last press focused, which typing goes into.
    field: std::cell::RefCell<Option<IUIAutomationElement>>,
}

/// An app with windows on the session's desktop.
#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub pid: u32,
}

impl A11y {
    /// Connect to UI Automation on this thread.
    pub fn connect() -> std::io::Result<A11y> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            // Already initialized in another mode is fine; we only read.
            if hr.is_err() && hr != RPC_E_CHANGED_MODE {
                return Err(std::io::Error::other(format!(
                    "starting COM for accessibility: {hr:?}"
                )));
            }
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| std::io::Error::other(format!("UI Automation: {e}")))?;
            Ok(A11y {
                automation,
                field: Default::default(),
            })
        }
    }

    /// Apps with a window on the desktop, one entry per process.
    pub fn apps(&self) -> Vec<App> {
        let mut apps: Vec<App> = Vec::new();
        for hwnd in wm::app_windows_here() {
            let pid = wm::app_of(hwnd);
            if apps.iter().any(|a| a.pid == pid) {
                continue;
            }
            apps.push(App {
                name: wm::program_name(pid),
                pid,
            });
        }
        apps
    }

    /// Act on the control at `(x, y)`, in device pixels on the desktop, as
    /// a click on it would: press a button, select an item, toggle a box,
    /// or focus a field for typing. The smallest control there that can do
    /// one of these acts, found through the same tree the session's steps
    /// target, which reaches controls a lookup by point does not, such as
    /// a search box hosted in a window of its own. Whether one did.
    pub fn press_at(&self, x: i32, y: i32) -> bool {
        let Ok(walker) = (unsafe { self.automation.ControlViewWalker() }) else {
            return false;
        };
        let mut found: Vec<(i64, IUIAutomationElement)> = Vec::new();
        for hwnd in wm::app_windows_here() {
            let handle = windows::Win32::Foundation::HWND(hwnd);
            if let Ok(root) = unsafe { self.automation.ElementFromHandle(handle) } {
                let mut seen = 0;
                under(&walker, &root, (x, y), &mut seen, &mut found);
            }
        }
        found.sort_by_key(|(area, _)| *area);
        found.iter().any(|(_, element)| self.act(element))
    }

    fn act(&self, element: &IUIAutomationElement) -> bool {
        unsafe {
            let editable = element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .is_ok_and(|value| {
                    !value
                        .CurrentIsReadOnly()
                        .map(|b| b.as_bool())
                        .unwrap_or(true)
                });
            if editable {
                let _ = element.SetFocus();
                *self.field.borrow_mut() = Some(element.clone());
                return true;
            }
            if let Ok(invoke) =
                element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                return invoke.Invoke().is_ok();
            }
            if let Ok(item) = element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                UIA_SelectionItemPatternId,
            ) {
                return item.Select().is_ok();
            }
            if let Ok(toggle) =
                element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            {
                return toggle.Toggle().is_ok();
            }
        }
        false
    }

    /// The latest `text` in an app's text, such as a console's buffer:
    /// where it shows, and where the text holding it shows, each as x, y,
    /// width, height in device pixels on the desktop.
    pub fn text_bounds(&self, text: &str) -> Option<(TextRect, TextRect)> {
        let walker = unsafe { self.automation.ControlViewWalker() }.ok()?;
        for hwnd in wm::app_windows_here() {
            let handle = windows::Win32::Foundation::HWND(hwnd);
            let Ok(root) = (unsafe { self.automation.ElementFromHandle(handle) }) else {
                continue;
            };
            let mut seen = 0;
            if let Some(found) = find_text(&walker, &root, text, &mut seen) {
                return Some(found);
            }
        }
        None
    }

    /// Type `c` at the end of the field the last press focused. Whether
    /// there was one to type into.
    pub fn type_char(&self, c: char) -> bool {
        let field = self.field.borrow();
        let Some(element) = field.as_ref() else {
            return false;
        };
        unsafe {
            let Ok(value) =
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            else {
                return false;
            };
            let mut text = value
                .CurrentValue()
                .map(|b| b.to_string())
                .unwrap_or_default();
            text.push(c);
            value
                .SetValue(&windows::core::BSTR::from(text.as_str()))
                .is_ok()
        }
    }

    /// The control-view nodes of every window the app owns, depth-first.
    pub fn nodes(&self, app: &App, all: bool) -> Vec<Node> {
        let walker = match unsafe { self.automation.ControlViewWalker() } {
            Ok(w) => w,
            Err(_) => return vec![],
        };
        let mut out = Vec::new();
        for hwnd in wm::app_windows_here() {
            if wm::app_of(hwnd) != app.pid {
                continue;
            }
            let handle = windows::Win32::Foundation::HWND(hwnd);
            let element = match unsafe { self.automation.ElementFromHandle(handle) } {
                Ok(e) => e,
                Err(_) => continue,
            };
            let window = wm::title(hwnd);
            let origin = {
                let r = wm::visible_rect(hwnd);
                (r.left, r.top)
            };
            walk(&walker, &element, 0, None, &window, origin, all, &mut out);
            if out.len() >= MAX_NODES {
                break;
            }
        }
        out
    }
}

/// The latest `text` in the text of `element` or a control-view
/// descendant: where it shows, and where that text shows.
fn find_text(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    text: &str,
    seen: &mut usize,
) -> Option<(TextRect, TextRect)> {
    *seen += 1;
    if *seen > MAX_NODES {
        return None;
    }
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
    {
        let found = unsafe {
            pattern
                .DocumentRange()
                .and_then(|range| range.FindText(&windows::core::BSTR::from(text), true, false))
        };
        if let Ok(range) = found {
            let shown = unsafe { range.GetBoundingRectangles() }
                .ok()
                .and_then(|array| unsafe { rects(array) });
            let holder = unsafe { element.CurrentBoundingRectangle() }.ok();
            if let (Some(shown), Some(r)) = (shown, holder) {
                let holder = (
                    r.left as f64,
                    r.top as f64,
                    (r.right - r.left) as f64,
                    (r.bottom - r.top) as f64,
                );
                return Some((shown, holder));
            }
        }
    }
    let mut child = unsafe { walker.GetFirstChildElement(element) };
    while let Ok(node) = child {
        if let Some(found) = find_text(walker, &node, text, seen) {
            return Some(found);
        }
        child = unsafe { walker.GetNextSiblingElement(&node) };
    }
    None
}

/// The rectangle around every rectangle in `array`, UI Automation's list of
/// left, top, width, and height, which this frees. `None` when it is empty,
/// as for text scrolled out of view.
unsafe fn rects(array: *mut windows::Win32::System::Com::SAFEARRAY) -> Option<TextRect> {
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData,
    };
    if array.is_null() {
        return None;
    }
    let mut out: Option<TextRect> = None;
    unsafe {
        if let Ok(upper) = SafeArrayGetUBound(array, 1) {
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            if SafeArrayAccessData(array, &mut data).is_ok() {
                let values = std::slice::from_raw_parts(data as *const f64, (upper + 1) as usize);
                for r in values.chunks_exact(4) {
                    let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
                    if w <= 0.0 || h <= 0.0 {
                        continue;
                    }
                    out = Some(match out {
                        None => (x, y, w, h),
                        Some((ox, oy, ow, oh)) => {
                            let (x0, y0) = (ox.min(x), oy.min(y));
                            let x1 = (ox + ow).max(x + w);
                            let y1 = (oy + oh).max(y + h);
                            (x0, y0, x1 - x0, y1 - y0)
                        }
                    });
                }
                let _ = SafeArrayUnaccessData(array);
            }
        }
        let _ = SafeArrayDestroy(array);
    }
    out
}

/// Add `element` and every control-view descendant whose bounds hold
/// `point` to `found`, each with its area, smallest first once sorted.
/// Bounds that miss the point prune their subtree; empty ones do not, since
/// some containers report none.
fn under(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    point: (i32, i32),
    seen: &mut usize,
    found: &mut Vec<(i64, IUIAutomationElement)>,
) {
    *seen += 1;
    if *seen > MAX_NODES {
        return;
    }
    if let Ok(r) = unsafe { element.CurrentBoundingRectangle() } {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w > 0 && h > 0 {
            let inside =
                (r.left..r.right).contains(&point.0) && (r.top..r.bottom).contains(&point.1);
            if !inside {
                return;
            }
            found.push((w as i64 * h as i64, element.clone()));
        }
    }
    let mut child = unsafe { walker.GetFirstChildElement(element) };
    while let Ok(node) = child {
        under(walker, &node, point, seen, found);
        child = unsafe { walker.GetNextSiblingElement(&node) };
    }
}

/// Add `element` and its control-view descendants to `out`, depth-first and
/// in order, as the Linux tree walk does. `origin` is the window's top-left
/// on the desktop, in device pixels, so extents come back relative to it.
#[allow(clippy::too_many_arguments)]
fn walk(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    depth: usize,
    parent: Option<usize>,
    window: &str,
    origin: (i32, i32),
    all: bool,
    out: &mut Vec<Node>,
) {
    if out.len() >= MAX_NODES {
        return;
    }
    let showing = !unsafe { element.CurrentIsOffscreen() }
        .map(|b| b.as_bool())
        .unwrap_or(false);
    // A hidden control and its subtree are left out unless everything is
    // asked for, matching the Linux walk.
    if !showing && !all {
        return;
    }
    let name = unsafe { element.CurrentName() }
        .map(|b| b.to_string())
        .unwrap_or_default();
    let control = unsafe { element.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
    let role = role_name(control).to_string();
    let focused = unsafe { element.CurrentHasKeyboardFocus() }
        .map(|b| b.as_bool())
        .unwrap_or(false);
    let extents = unsafe { element.CurrentBoundingRectangle() }
        .ok()
        .and_then(|r| {
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            (w > 0 && h > 0).then_some((r.left - origin.0, r.top - origin.1, w, h))
        });
    let password = unsafe { element.CurrentIsPassword() }
        .map(|b| b.as_bool())
        .unwrap_or(false);
    let text = (!password).then(|| read_text(element, control)).flatten();
    let value = (control == UIA_SliderControlTypeId || control == UIA_SpinnerControlTypeId)
        .then(|| read_value(element))
        .flatten();

    let index = out.len();
    out.push(Node {
        name,
        role,
        depth,
        parent,
        window: window.to_string(),
        extents,
        showing,
        focused,
        text,
        value,
    });

    let mut child = unsafe { walker.GetFirstChildElement(element) };
    while let Ok(node) = child {
        walk(
            walker,
            &node,
            depth + 1,
            Some(index),
            window,
            origin,
            all,
            out,
        );
        if out.len() >= MAX_NODES {
            return;
        }
        child = unsafe { walker.GetNextSiblingElement(&node) };
    }
}

/// The text a field or document holds, through whichever pattern it offers.
fn read_text(element: &IUIAutomationElement, control: UIA_CONTROLTYPE_ID) -> Option<String> {
    if control == UIA_EditControlTypeId || control == UIA_ComboBoxControlTypeId {
        let pattern: IUIAutomationValuePattern =
            unsafe { element.GetCurrentPattern(UIA_ValuePatternId) }
                .ok()?
                .cast()
                .ok()?;
        return unsafe { pattern.CurrentValue() }
            .ok()
            .map(|b| b.to_string());
    }
    if control == UIA_DocumentControlTypeId || control == UIA_TextControlTypeId {
        let pattern: IUIAutomationTextPattern =
            unsafe { element.GetCurrentPattern(UIA_TextPatternId) }
                .ok()?
                .cast()
                .ok()?;
        let range = unsafe { pattern.DocumentRange() }.ok()?;
        return unsafe { range.GetText(-1) }.ok().map(|b| b.to_string());
    }
    None
}

/// A slider's or spinner's current, minimum, and maximum value.
fn read_value(element: &IUIAutomationElement) -> Option<(f64, f64, f64)> {
    let pattern: IUIAutomationRangeValuePattern =
        unsafe { element.GetCurrentPattern(UIA_RangeValuePatternId) }
            .ok()?
            .cast()
            .ok()?;
    Some((
        unsafe { pattern.CurrentValue() }.ok()?,
        unsafe { pattern.CurrentMinimum() }.ok()?,
        unsafe { pattern.CurrentMaximum() }.ok()?,
    ))
}

/// The AT-SPI role name matinee uses for a UI Automation control type, so a
/// storyboard names a control the same way on Windows and Linux.
fn role_name(control: UIA_CONTROLTYPE_ID) -> &'static str {
    let pairs: &[(UIA_CONTROLTYPE_ID, &str)] = &[
        (UIA_ButtonControlTypeId, "push button"),
        (UIA_SplitButtonControlTypeId, "push button"),
        (UIA_EditControlTypeId, "text box"),
        (UIA_TextControlTypeId, "label"),
        (UIA_DocumentControlTypeId, "document text"),
        (UIA_CheckBoxControlTypeId, "check box"),
        (UIA_RadioButtonControlTypeId, "radio button"),
        (UIA_SliderControlTypeId, "slider"),
        (UIA_SpinnerControlTypeId, "spin button"),
        (UIA_ComboBoxControlTypeId, "combo box"),
        (UIA_ListControlTypeId, "list box"),
        (UIA_ListItemControlTypeId, "list item"),
        (UIA_MenuControlTypeId, "menu"),
        (UIA_MenuBarControlTypeId, "menu bar"),
        (UIA_MenuItemControlTypeId, "menu item"),
        (UIA_TabControlTypeId, "page tab list"),
        (UIA_TabItemControlTypeId, "page tab"),
        (UIA_TreeControlTypeId, "tree"),
        (UIA_TreeItemControlTypeId, "tree item"),
        (UIA_TableControlTypeId, "table"),
        (UIA_DataGridControlTypeId, "table"),
        (UIA_DataItemControlTypeId, "table cell"),
        (UIA_HeaderControlTypeId, "header"),
        (UIA_HeaderItemControlTypeId, "column header"),
        (UIA_HyperlinkControlTypeId, "link"),
        (UIA_ImageControlTypeId, "image"),
        (UIA_WindowControlTypeId, "frame"),
        (UIA_PaneControlTypeId, "panel"),
        (UIA_GroupControlTypeId, "panel"),
        (UIA_ToolBarControlTypeId, "tool bar"),
        (UIA_TitleBarControlTypeId, "title bar"),
        (UIA_StatusBarControlTypeId, "status bar"),
        (UIA_ProgressBarControlTypeId, "progress bar"),
        (UIA_ScrollBarControlTypeId, "scroll bar"),
        (UIA_SeparatorControlTypeId, "separator"),
        (UIA_ToolTipControlTypeId, "tool tip"),
        (UIA_CalendarControlTypeId, "calendar"),
    ];
    pairs
        .iter()
        .find(|(id, _)| *id == control)
        .map(|(_, name)| *name)
        .unwrap_or("unknown")
}
