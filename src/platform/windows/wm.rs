//! The private desktop the session's apps live on, and what matinee does
//! as their window manager: find their windows, give each its place, and
//! capture what they draw.
//!
//! Windows has no headless display, so the session makes a desktop of its
//! own, unseen by the user, and launches apps onto it. There is no screen
//! surface to read back from such a desktop, so each window is captured on
//! its own through `PrintWindow` and composited by its position.

use super::proc::Job;
use std::{
    cell::RefCell,
    io, mem, ptr,
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, RECT},
    Graphics::{
        Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, SelectObject,
        },
    },
    Storage::Xps::PrintWindow,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        JobObjects::IsProcessInJob,
        StationsAndDesktops::{
            CloseDesktop, CreateDesktopW, DF_ALLOWOTHERACCOUNTHOOK, EnumDesktopWindows,
            GetThreadDesktop, HDESK, SetThreadDesktop,
        },
        Threading::{
            GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::WindowsAndMessaging::{
        FindWindowExW, GW_OWNER, GWL_STYLE, GetClassNameW, GetWindow, GetWindowLongW,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        PW_RENDERFULLCONTENT, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WS_CAPTION,
        WS_CHILD,
    },
};

/// Window classes that are never windows to target or draw: menu
/// shadows, input method helpers, and the like.
const HELPER_CLASSES: &[&str] = &["SysShadow", "IME", "MSCTFIME UI"];
/// Classes of popups drawn but not targeted: menus and tooltips.
const POPUP_CLASSES: &[&str] = &["#32768", "tooltips_class32"];

/// Sessions this process has made a desktop for, so each gets a new one.
static SESSIONS: AtomicU32 = AtomicU32::new(0);

/// The session this thread runs: the private desktop its apps open on, or
/// `None` on the screen, where they open on the user's own; the job they
/// go in; and the packaged apps Windows started for it outside the job.
struct Current {
    desktop: Option<String>,
    job: HANDLE,
    activated: Vec<u32>,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// Where this thread's session opens apps, by desktop name (`None` on the
/// screen), and the job they go in.
pub fn session_desktop() -> Option<(Option<String>, HANDLE)> {
    CURRENT.with(|c| c.borrow().as_ref().map(|s| (s.desktop.clone(), s.job)))
}

/// Count `pid`, a packaged app Windows started for the session, as one of
/// the session's apps.
pub fn adopt(pid: u32) {
    CURRENT.with(|c| {
        if let Some(s) = c.borrow_mut().as_mut() {
            s.activated.push(pid);
        }
    });
}

/// Whether `hwnd` is one of the session's windows. On a private desktop
/// every window is; on the screen, only those of the session's job and of
/// the packaged apps it activated, so the user's own windows are never
/// placed, drawn, targeted, or given input.
pub fn is_session_window(hwnd: HWND) -> bool {
    CURRENT.with(|c| {
        let current = c.borrow();
        let Some(s) = current.as_ref() else {
            return false;
        };
        if s.desktop.is_some() {
            return true;
        }
        let pid = app_of(hwnd);
        s.activated.contains(&pid) || in_job(pid, s.job)
    })
}

fn in_job(pid: u32, job: HANDLE) -> bool {
    let process: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut result = 0;
    let ok = unsafe { IsProcessInJob(process, job, &mut result) };
    unsafe { CloseHandle(process) };
    ok != 0 && result != 0
}

/// The process whose app `hwnd` shows. A UWP app's window is a frame that
/// `ApplicationFrameHost` draws around the app's own window, so the frame
/// counts as the app inside it.
pub fn app_of(hwnd: HWND) -> u32 {
    if class_name(hwnd) == "ApplicationFrameWindow" {
        let class: Vec<u16> = "Windows.UI.Core.CoreWindow"
            .encode_utf16()
            .chain([0])
            .collect();
        let inner = unsafe { FindWindowExW(hwnd, ptr::null_mut(), class.as_ptr(), ptr::null()) };
        if !inner.is_null() {
            return process_of(inner);
        }
    }
    process_of(hwnd)
}

/// Where the session's apps run, the job they run in, and the thread's way
/// back.
///
/// By default that is a desktop of the session's own, unseen. Windows
/// destroys a desktop once nothing holds it: not matinee, and no process
/// running on it. Each session gets a desktop of its own, so an app that
/// outlives one session never shows in the next, and the job ends the
/// session's apps, and whatever they started, with the session.
///
/// On the screen there is no desktop to make: apps open on the user's own,
/// among the user's windows, and the job and the packaged apps the session
/// activated say which windows are the session's.
pub struct Desktop {
    /// The private desktop; `None` on the screen.
    handle: Option<HDESK>,
    original: HDESK,
    name: String,
    job: Job,
}

impl Desktop {
    /// Make the session's place for apps: on the screen, the user's own
    /// desktop as it is; otherwise a private desktop, with this thread
    /// moved onto it so that every process it starts opens its windows
    /// there.
    pub fn create(screen: bool) -> io::Result<Desktop> {
        let job = Job::new()
            .map_err(|e| io::Error::new(e.kind(), format!("creating the session's job: {e}")))?;
        let original = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
        if screen {
            CURRENT.with(|c| {
                *c.borrow_mut() = Some(Current {
                    desktop: None,
                    job: job.handle(),
                    activated: Vec::new(),
                })
            });
            return Ok(Desktop {
                handle: None,
                original,
                name: "the screen".into(),
                job,
            });
        }
        let session = SESSIONS.fetch_add(1, Ordering::Relaxed);
        let name = format!("matinee-{}-{session}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // Hooks from other accounts are irrelevant; the flag keeps apps
        // that install their own hooks working.
        let handle = unsafe {
            CreateDesktopW(
                wide.as_ptr(),
                ptr::null(),
                ptr::null(),
                DF_ALLOWOTHERACCOUNTHOOK,
                0x1000_0000, /* GENERIC_ALL */
                ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::new(
                io::Error::last_os_error().kind(),
                format!("creating a desktop: {}", io::Error::last_os_error()),
            ));
        }
        if unsafe { SetThreadDesktop(handle) } == 0 {
            let e = io::Error::last_os_error();
            unsafe { CloseDesktop(handle) };
            return Err(io::Error::new(
                e.kind(),
                format!("entering the desktop: {e}"),
            ));
        }
        CURRENT.with(|c| {
            *c.borrow_mut() = Some(Current {
                desktop: Some(name.clone()),
                job: job.handle(),
                activated: Vec::new(),
            })
        });
        Ok(Desktop {
            handle: Some(handle),
            original,
            name,
            job,
        })
    }
}

/// Every one of the session's windows on the desktop, top of the stack
/// first (see [`is_session_window`]).
fn all_windows_on(desktop: HDESK) -> Vec<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, lparam: isize) -> i32 {
        let list = unsafe { &mut *(lparam as *mut Vec<HWND>) };
        list.push(hwnd);
        1
    }
    unsafe { EnumDesktopWindows(desktop, Some(collect), &mut list as *mut _ as isize) };
    list.retain(|&h| is_session_window(h));
    list
}

fn drawn_windows_on(desktop: HDESK) -> Vec<HWND> {
    all_windows_on(desktop)
        .into_iter()
        .filter(|&h| unsafe { IsWindowVisible(h) } != 0)
        .filter(|&h| !HELPER_CLASSES.contains(&class_name(h).as_str()))
        .filter(|&h| {
            let r = window_rect(h);
            r.right > r.left && r.bottom > r.top
        })
        .collect()
}

fn app_windows_on(desktop: HDESK) -> Vec<HWND> {
    drawn_windows_on(desktop)
        .into_iter()
        .filter(|&h| !POPUP_CLASSES.contains(&class_name(h).as_str()))
        .filter(|&h| unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & WS_CHILD == 0)
        .filter(|&h| {
            // Owned windows count when they are real dialogs.
            unsafe { GetWindow(h, GW_OWNER) }.is_null()
                || unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & WS_CAPTION == WS_CAPTION
        })
        .collect()
}

/// The windows worth drawing on the desktop this thread is on, top first:
/// apps' windows and their menus and tooltips.
pub fn drawn_windows() -> Vec<HWND> {
    drawn_windows_on(unsafe { GetThreadDesktop(GetCurrentThreadId()) })
}

/// The app windows on the desktop this thread is on, top first.
pub fn app_windows_here() -> Vec<HWND> {
    app_windows_on(unsafe { GetThreadDesktop(GetCurrentThreadId()) })
}

/// App windows on `desktop`, counting hidden ones, for what an app left
/// behind when its session ended.
fn hidden_app_windows_on(desktop: HDESK) -> Vec<HWND> {
    all_windows_on(desktop)
        .into_iter()
        .filter(|&h| !HELPER_CLASSES.contains(&class_name(h).as_str()))
        .filter(|&h| !POPUP_CLASSES.contains(&class_name(h).as_str()))
        .filter(|&h| {
            let r = window_rect(h);
            r.right > r.left && r.bottom > r.top
        })
        .filter(|&h| unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & WS_CHILD == 0)
        .filter(|&h| {
            !is_owned(h)
                || unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & WS_CAPTION == WS_CAPTION
        })
        .collect()
}

impl Drop for Desktop {
    /// End the session's apps, then name any window still open: a program
    /// Windows started on the app's behalf, outside the job, such as a
    /// packaged app. It keeps the desktop alive until it exits.
    fn drop(&mut self) {
        self.job.kill();
        // On the screen, the session's packaged apps are its children and
        // end with it; anything else there is the user's.
        let Some(handle) = self.handle else {
            CURRENT.with(|c| c.borrow_mut().take());
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut left = hidden_app_windows_on(handle);
        while !left.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
            left = hidden_app_windows_on(handle);
        }
        CURRENT.with(|c| c.borrow_mut().take());
        if !left.is_empty() {
            let mut programs: Vec<String> =
                left.iter().map(|&h| program_name(process_of(h))).collect();
            programs.sort();
            programs.dedup();
            eprintln!(
                "matinee: warning: {} still running on desktop {} after the session ended; \
                 Windows started it outside the session, so close it yourself",
                programs.join(", "),
                self.name
            );
        }
        unsafe {
            SetThreadDesktop(self.original);
            CloseDesktop(handle);
        }
    }
}

/// The window's title.
pub fn title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The window's class name.
pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The process that owns the window.
pub fn process_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

/// The thread that owns the window.
pub fn thread_of(hwnd: HWND) -> u32 {
    unsafe { GetWindowThreadProcessId(hwnd, ptr::null_mut()) }
}

/// Whether the window is owned by another, as dialogs are.
pub fn is_owned(hwnd: HWND) -> bool {
    !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null()
}

/// The window's whole rectangle, including the frame Windows draws
/// outside the visible edge.
pub fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// The rectangle a person sees: without the invisible resize borders.
pub fn visible_rect(hwnd: HWND) -> RECT {
    let mut r = window_rect(hwnd);
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut _ as *mut _,
            mem::size_of::<RECT>() as u32,
        )
    };
    r
}

/// Move the window's visible rectangle to `(x, y)` and, unless `size` is
/// `None`, make it `size` wide and high.
pub fn place(hwnd: HWND, x: i32, y: i32, size: Option<(i32, i32)>) {
    let whole = window_rect(hwnd);
    let visible = visible_rect(hwnd);
    // The invisible border on each side, so the visible edge lands where
    // it is asked to.
    let (dl, dt) = (visible.left - whole.left, visible.top - whole.top);
    let (dr, db) = (whole.right - visible.right, whole.bottom - visible.bottom);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    match size {
        Some((w, h)) => unsafe {
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                x - dl,
                y - dt,
                w + dl + dr,
                h + dt + db,
                flags,
            );
        },
        None => unsafe {
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                x - dl,
                y - dt,
                0,
                0,
                flags | SWP_NOSIZE,
            );
        },
    }
}

/// The file name of the program running as `pid`, without `.exe`.
pub fn program_name(pid: u32) -> String {
    let process: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(process) };
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The parent of process `pid`, if it is running.
pub fn parent_of(pid: u32) -> Option<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return None;
    }
    let mut entry: PROCESSENTRY32W = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut parent = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            if entry.th32ProcessID == pid {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    parent
}

/// What a window draws, as rows of 0xAARRGGBB pixels of its visible
/// rectangle, or `None` when it could not be captured.
pub fn capture(hwnd: HWND) -> Option<(usize, usize, Vec<u32>)> {
    let whole = window_rect(hwnd);
    let visible = visible_rect(hwnd);
    let (w, h) = (
        (whole.right - whole.left) as usize,
        (whole.bottom - whole.top) as usize,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let dc: HDC = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut info: BITMAPINFO = unsafe { mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w as i32,
        // Negative: rows from the top.
        biHeight: -(h as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { mem::zeroed() }
    };
    let mut bits: *mut core::ffi::c_void = ptr::null_mut();
    let bitmap: HBITMAP =
        unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe { DeleteDC(dc) };
        return None;
    }
    let previous = unsafe { SelectObject(dc, bitmap) };
    let ok = unsafe { PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) };
    let pixels = (ok != 0).then(|| {
        let all = unsafe { std::slice::from_raw_parts(bits as *const u32, w * h) };
        // Keep the visible part, opaque.
        let (vx, vy) = (
            (visible.left - whole.left).clamp(0, w as i32) as usize,
            (visible.top - whole.top).clamp(0, h as i32) as usize,
        );
        let (vw, vh) = (
            ((visible.right - visible.left) as usize).min(w - vx),
            ((visible.bottom - visible.top) as usize).min(h - vy),
        );
        let mut out = Vec::with_capacity(vw * vh);
        for y in vy..vy + vh {
            out.extend(
                all[y * w + vx..y * w + vx + vw]
                    .iter()
                    .map(|p| p | 0xff00_0000),
            );
        }
        (vw, vh, out)
    });
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    pixels
}
