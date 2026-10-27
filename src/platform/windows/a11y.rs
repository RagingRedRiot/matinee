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
            Ok(A11y { automation })
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
