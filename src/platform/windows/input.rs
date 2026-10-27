//! Pointer and keyboard input for apps on the private desktop.
//!
//! The desktop is never the one taking real input, so events cannot go
//! through the input queue. They are posted to windows as the messages the
//! queue would have delivered: a pointer event to the deepest child window
//! under the pointer, in its client coordinates, and a key event to the
//! window with keyboard focus. Modifier state is set on the app's thread
//! through an attached input queue, since a posted key carries none. Many
//! apps ignore posted input, so the display first acts on the control
//! through UI Automation, and posts only what that cannot do.
//!
//! On the screen (`--screen`) apps take real input, and many modern
//! ones read nothing else, so the `send_` functions put events through the
//! input queue as a mouse and keyboard would. A press only goes out when
//! what it lands on is one of the session's windows, so a user's window
//! that comes to the front never receives a click or a keystroke meant for
//! the app; releases always go out, so nothing is left held down.

use super::wm;
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::ScreenToClient,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetKeyboardState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
            MOUSEINPUT, MapVirtualKeyW, SendInput, SetActiveWindow, SetKeyboardState, ToUnicode,
            VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
        },
        WindowsAndMessaging::{
            BringWindowToTop, CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT,
            ChildWindowFromPointEx, GA_ROOT, GUITHREADINFO, GetAncestor, GetForegroundWindow,
            GetGUIThreadInfo, GetSystemMetrics, GetWindowThreadProcessId, HWND_TOP, PostMessageW,
            SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos, WM_CHAR,
            WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            WindowFromPoint,
        },
    },
};

/// Linux input codes for the modifier keys, as [`crate::keys`] uses them.
const SHIFT: u32 = 42;
const CTRL: u32 = 29;
const ALT: u32 = 56;
const SUPER: u32 = 125;

/// Mouse-message key state flags.
const MK_LBUTTON: usize = 0x0001;
const MK_RBUTTON: usize = 0x0002;
const MK_MBUTTON: usize = 0x0010;

/// A window under the pointer: the top-level window, the deepest child at
/// the point, and the point in that child's client coordinates.
pub struct Under {
    pub top: HWND,
    pub child: HWND,
    pub x: i32,
    pub y: i32,
}

/// What is under `(x, y)` on the desktop, in points.
pub fn under(x: f64, y: f64) -> Option<Under> {
    let (px, py) = (x.round() as i32, y.round() as i32);
    let top = wm::drawn_windows().into_iter().find(|&h| {
        let r = wm::visible_rect(h);
        px >= r.left && px < r.right && py >= r.top && py < r.bottom
    })?;
    let mut child = top;
    loop {
        let mut p = POINT { x: px, y: py };
        unsafe { ScreenToClient(child, &mut p) };
        let next = unsafe {
            ChildWindowFromPointEx(
                child,
                p,
                CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT,
            )
        };
        if next.is_null() || next == child {
            return Some(Under {
                top,
                child,
                x: p.x,
                y: p.y,
            });
        }
        child = next;
    }
}

fn lparam(x: i32, y: i32) -> isize {
    ((y as u16 as isize) << 16) | (x as u16 as isize)
}

/// The pointer moved to `(x, y)` with `buttons` held.
pub fn pointer_move(x: f64, y: f64, buttons: usize) {
    if let Some(u) = under(x, y) {
        unsafe { PostMessageW(u.child, WM_MOUSEMOVE, buttons, lparam(u.x, u.y)) };
    }
}

/// A button went down or up at `(x, y)`; `buttons` is the state after.
pub fn pointer_button(
    x: f64,
    y: f64,
    button: crate::platform::Button,
    pressed: bool,
    buttons: usize,
) {
    use crate::platform::Button;
    let Some(u) = under(x, y) else { return };
    if pressed {
        activate(u.top);
    }
    let message = match (button, pressed) {
        (Button::Left, true) => WM_LBUTTONDOWN,
        (Button::Left, false) => WM_LBUTTONUP,
        (Button::Right, true) => WM_RBUTTONDOWN,
        (Button::Right, false) => WM_RBUTTONUP,
        (Button::Middle, true) => WM_MBUTTONDOWN,
        (Button::Middle, false) => WM_MBUTTONUP,
    };
    unsafe { PostMessageW(u.child, message, buttons, lparam(u.x, u.y)) };
}

/// The flag a mouse message carries for `button`.
pub fn button_flag(button: crate::platform::Button) -> usize {
    use crate::platform::Button;
    match button {
        Button::Left => MK_LBUTTON,
        Button::Right => MK_RBUTTON,
        Button::Middle => MK_MBUTTON,
    }
}

/// Scroll by `dy` points at `(x, y)`; positive scrolls down.
pub fn pointer_scroll(x: f64, y: f64, dy: f64, buttons: usize) {
    let Some(u) = under(x, y) else { return };
    let delta = (-dy / 15.0 * 120.0).round() as i16;
    let wparam = ((delta as u16 as usize) << 16) | buttons;
    // Wheel messages carry screen coordinates.
    unsafe {
        PostMessageW(
            u.child,
            WM_MOUSEWHEEL,
            wparam,
            lparam(x.round() as i32, y.round() as i32),
        )
    };
}

/// Bring `top` to the front and make it the active window, as a click on
/// it would.
pub fn activate(top: HWND) {
    unsafe {
        SetWindowPos(
            top,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let thread = wm::thread_of(top);
        AttachThreadInput(GetCurrentThreadId(), thread, 1);
        SetActiveWindow(top);
        AttachThreadInput(GetCurrentThreadId(), thread, 0);
    }
}

/// The window with keyboard focus in `top`'s thread, or `top` itself.
pub fn focus_in(top: HWND) -> HWND {
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(wm::thread_of(top), &mut info) } != 0 && !info.hwndFocus.is_null()
    {
        info.hwndFocus
    } else {
        top
    }
}

/// Move the real pointer to `(x, y)`, in device pixels on the screen.
pub fn send_pointer_move(x: f64, y: f64) {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2),
        )
    };
    // Absolute positions run from 0 to 65535 across the virtual screen.
    let nx = ((x - vx as f64) * 65535.0 / (vw - 1) as f64).round() as i32;
    let ny = ((y - vy as f64) * 65535.0 / (vh - 1) as f64).round() as i32;
    send_mouse(
        nx,
        ny,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    );
}

/// Press or release `button` at `(x, y)` on the screen.
pub fn send_pointer_button(x: f64, y: f64, button: crate::platform::Button, pressed: bool) {
    use crate::platform::Button;
    if pressed {
        let p = POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        };
        let top = unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) };
        if top.is_null() || !wm::is_session_window(top) {
            eprintln!(
                "matinee: warning: something not of this story is at {},{} on the screen, so \
                 the press was not sent",
                p.x, p.y
            );
            return;
        }
    }
    send_pointer_move(x, y);
    let flag = match (button, pressed) {
        (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (Button::Left, false) => MOUSEEVENTF_LEFTUP,
        (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
        (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
        (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
    };
    send_mouse(0, 0, 0, flag);
}

/// Scroll by `dy` points with the real wheel; positive scrolls down. Only
/// over the session's windows.
pub fn send_pointer_scroll(x: f64, y: f64, dy: f64) {
    let p = POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    };
    let top = unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) };
    if top.is_null() || !wm::is_session_window(top) {
        return;
    }
    let delta = (-dy / 15.0 * 120.0).round() as i32;
    send_mouse(0, 0, delta as u32, MOUSEEVENTF_WHEEL);
}

/// Press or release the key with Linux input code `code` on the real
/// keyboard, into `top` (the session's top window), brought to the front
/// first if it is not already.
pub fn send_key(top: Option<HWND>, code: u32, pressed: bool) {
    let Some((vk, extended)) = virtual_key(code) else {
        return;
    };
    if pressed {
        let foreground = |h: HWND| !h.is_null() && wm::is_session_window(h);
        let mut current = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) };
        if let (false, Some(top)) = (foreground(current), top) {
            bring_forward(top);
            current = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) };
        }
        if !foreground(current) {
            eprintln!(
                "matinee: warning: none of this story's windows has the keyboard, so the key \
                 was not sent"
            );
            return;
        }
    }
    let mut flags = if pressed { 0 } else { KEYEVENTF_KEYUP };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk as u32, 0) } as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
}

fn send_mouse(dx: i32, dy: i32, data: u32, flags: u32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
}

/// Bring `top` to the front of the screen and give it the keyboard.
/// Windows lets only the foreground's own input queue change the
/// foreground, so matinee joins that queue while it asks.
pub fn bring_forward(top: HWND) {
    unsafe {
        let current = GetForegroundWindow();
        let thread = if current.is_null() {
            0
        } else {
            GetWindowThreadProcessId(current, std::ptr::null_mut())
        };
        let me = GetCurrentThreadId();
        let attach = thread != 0 && thread != me;
        if attach {
            AttachThreadInput(me, thread, 1);
        }
        BringWindowToTop(top);
        SetForegroundWindow(top);
        if attach {
            AttachThreadInput(me, thread, 0);
        }
    }
}

/// Which modifiers are held.
#[derive(Default, Clone, Copy)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    /// Record `code` going down or up, if it is a modifier.
    pub fn apply(&mut self, code: u32, pressed: bool) -> bool {
        match code {
            SHIFT => self.shift = pressed,
            CTRL => self.ctrl = pressed,
            ALT => self.alt = pressed,
            SUPER => self.meta = pressed,
            _ => return false,
        }
        true
    }
}

/// Press or release the key with Linux input code `code` in `top`'s
/// focused window, with `modifiers` held.
pub fn key(top: HWND, code: u32, pressed: bool, modifiers: Modifiers) {
    let Some((vk, extended)) = virtual_key(code) else {
        return;
    };
    let target = focus_in(top);
    let thread = wm::thread_of(top);
    let scan = unsafe { MapVirtualKeyW(vk as u32, 0) };
    let mut lparam = 1 | ((scan as isize) << 16);
    if extended {
        lparam |= 1 << 24;
    }
    if modifiers.alt {
        lparam |= 1 << 29;
    }
    if !pressed {
        lparam |= (1 << 30) | (1 << 31);
    }
    let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.meta;
    unsafe {
        AttachThreadInput(GetCurrentThreadId(), thread, 1);
        let mut state = [0u8; 256];
        GetKeyboardState(state.as_mut_ptr());
        for (held, key) in [
            (modifiers.shift, VK_SHIFT),
            (modifiers.ctrl, VK_CONTROL),
            (modifiers.alt, VK_MENU),
            (modifiers.meta, VK_LWIN),
        ] {
            state[key as usize] = if held { 0x80 } else { 0 };
        }
        state[vk as usize] = if pressed { 0x80 } else { 0 };
        SetKeyboardState(state.as_ptr());

        // The character this key makes with the modifiers held, if any.
        let mut buf = [0u16; 8];
        let n = ToUnicode(
            vk as u32,
            scan,
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            4, // do not disturb the real keyboard state
        );
        let printable = plain && n == 1 && buf[0] >= 0x20;

        if printable {
            // Ordinary typing: post the character itself, once, on the press.
            // Posting a key message instead would make the app's own message
            // loop translate it to a character too, doubling every letter,
            // and that translation misses the held Shift.
            if pressed {
                PostMessageW(target, WM_CHAR, buf[0] as usize, lparam);
            }
        } else {
            // Named keys (Enter, Tab, arrows, function keys), whose character
            // the app's message loop produces, and shortcut chords the app
            // reads from the key message directly.
            let message = match (modifiers.alt, pressed) {
                (true, true) => WM_SYSKEYDOWN,
                (true, false) => WM_SYSKEYUP,
                (false, true) => WM_KEYDOWN,
                (false, false) => WM_KEYUP,
            };
            PostMessageW(target, message, vk as usize, lparam);
        }
        AttachThreadInput(GetCurrentThreadId(), thread, 0);
    }
}

/// The character the key with Linux input code `code` types with
/// `modifiers` held, if it types one: none with Ctrl, Alt, or the logo key.
pub fn char_for(code: u32, modifiers: Modifiers) -> Option<char> {
    if modifiers.ctrl || modifiers.alt || modifiers.meta {
        return None;
    }
    let (vk, _) = virtual_key(code)?;
    let mut state = [0u8; 256];
    if modifiers.shift {
        state[VK_SHIFT as usize] = 0x80;
    }
    let mut buf = [0u16; 8];
    let n = unsafe {
        ToUnicode(
            vk as u32,
            MapVirtualKeyW(vk as u32, 0),
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            4, // do not disturb the real keyboard state
        )
    };
    (n == 1 && buf[0] >= 0x20)
        .then(|| char::from_u32(buf[0] as u32))
        .flatten()
}

/// The virtual key for a Linux input code, and whether Windows calls it
/// an extended key.
fn virtual_key(code: u32) -> Option<(u16, bool)> {
    const ROWS: [(&[u8], u32); 4] = [
        (b"1234567890", 2),
        (b"QWERTYUIOP", 16),
        (b"ASDFGHJKL", 30),
        (b"ZXCVBNM", 44),
    ];
    for (keys, first) in ROWS {
        if code >= first && code < first + keys.len() as u32 {
            return Some((keys[(code - first) as usize] as u16, false));
        }
    }
    let plain = |vk: u16| Some((vk, false));
    let extended = |vk: u16| Some((vk, true));
    match code {
        1 => plain(0x1B),                            // Escape
        12 => plain(0xBD),                           // -
        13 => plain(0xBB),                           // =
        14 => plain(0x08),                           // Backspace
        15 => plain(0x09),                           // Tab
        26 => plain(0xDB),                           // [
        27 => plain(0xDD),                           // ]
        28 => plain(0x0D),                           // Enter
        29 => plain(0x11),                           // Ctrl
        39 => plain(0xBA),                           // ;
        40 => plain(0xDE),                           // '
        41 => plain(0xC0),                           // `
        42 => plain(0x10),                           // Shift
        43 => plain(0xDC),                           // \
        51 => plain(0xBC),                           // ,
        52 => plain(0xBE),                           // .
        53 => plain(0xBF),                           // /
        56 => plain(0x12),                           // Alt
        57 => plain(0x20),                           // Space
        59..=68 => plain(0x70 + (code - 59) as u16), // F1 to F10
        87 => plain(0x7A),                           // F11
        88 => plain(0x7B),                           // F12
        102 => extended(0x24),                       // Home
        103 => extended(0x26),                       // Up
        104 => extended(0x21),                       // Page Up
        105 => extended(0x25),                       // Left
        106 => extended(0x27),                       // Right
        107 => extended(0x23),                       // End
        108 => extended(0x28),                       // Down
        109 => extended(0x22),                       // Page Down
        110 => extended(0x2D),                       // Insert
        111 => extended(0x2E),                       // Delete
        125 => extended(0x5B),                       // Super
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::virtual_key;

    #[test]
    fn letters_digits_and_named_keys_map() {
        assert_eq!(virtual_key(30), Some((b'A' as u16, false)));
        assert_eq!(virtual_key(25), Some((b'P' as u16, false)));
        assert_eq!(virtual_key(2), Some((b'1' as u16, false)));
        assert_eq!(virtual_key(11), Some((b'0' as u16, false)));
        assert_eq!(virtual_key(28), Some((0x0D, false)));
        assert_eq!(virtual_key(103), Some((0x26, true)));
        assert_eq!(virtual_key(88), Some((0x7B, false)));
        assert_eq!(virtual_key(999), None);
    }
}
