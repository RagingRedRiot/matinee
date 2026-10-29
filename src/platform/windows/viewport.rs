//! A live window onto a running story, for watching it work.
//!
//! `--view` opens one window on the user's desktop that shows the whole
//! session as it runs, with the camera's view outlined and the rest of the
//! screen dimmed, as on Linux.
//!
//! The window belongs to a thread of its own, which starts on the user's
//! desktop even though the session's thread moves onto a private one, so
//! the window is seen and the session's apps stay where they are. Each
//! tick the session leaves the latest frame for it, and the window shows
//! whichever frame is there when it next looks. The session never waits
//! for the window: when the window is reading the frame, the session skips
//! that tick's copy, so watching a story does not change its recording.

use crate::platform::view::{MAX_WINDOW, Snapshot, compose, fit};
use crate::{camera::Rect, frame::Frame};
use std::{
    cell::RefCell,
    io, mem, ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, InvalidateRect,
        PAINTSTRUCT, SetDIBitsToDevice,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        WindowsAndMessaging::{
            AdjustWindowRect, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW,
            DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, IDC_ARROW,
            LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE,
            SetTimer, ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_SIZE,
            WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        },
    },
};

/// How often the window looks for a new frame, in milliseconds.
const POLL_MS: u32 = 16;

/// The latest frame the session left, and how many it has left so far.
struct Latest {
    seq: u64,
    snap: Snapshot,
}

struct Viewport {
    latest: Mutex<Latest>,
    /// The window, once made, so `close` can close it.
    window: AtomicIsize,
    thread: Mutex<Option<JoinHandle<()>>>,
}

static VIEWPORT: OnceLock<Viewport> = OnceLock::new();

thread_local! {
    /// The window thread's copy of the frame it shows, and the picture of
    /// it at the window's size.
    static SHOWN: RefCell<(Snapshot, Vec<u32>, (usize, usize))> =
        RefCell::new((Snapshot::default(), vec![], (0, 0)));
}

/// Open the viewport window, sized at first for a screen of `screen`
/// points. It returns once the window is made, or says why it could not
/// be. Frames of any size are taken, so `_capacity` is not needed here.
pub fn open(_capacity: usize, screen: (u32, u32)) -> io::Result<()> {
    if VIEWPORT.get().is_some() {
        return Err(io::Error::other("the viewport is already open"));
    }
    let _ = VIEWPORT.set(Viewport {
        latest: Mutex::new(Latest {
            seq: 0,
            snap: Snapshot::default(),
        }),
        window: AtomicIsize::new(0),
        thread: Mutex::new(None),
    });
    let (ready, made) = mpsc::channel();
    let thread = thread::spawn(move || match make_window(screen) {
        Ok(hwnd) => {
            if let Some(viewport) = VIEWPORT.get() {
                viewport.window.store(hwnd as isize, Ordering::Release);
            }
            let _ = ready.send(Ok(()));
            run();
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    });
    let made = made
        .recv()
        .unwrap_or_else(|_| Err(io::Error::other("the viewport's thread ended")));
    match made {
        Ok(()) => {
            if let Some(viewport) = VIEWPORT.get() {
                *viewport.thread.lock().unwrap() = Some(thread);
            }
            Ok(())
        }
        Err(e) => {
            let _ = thread.join();
            Err(e)
        }
    }
}

/// Show `frame` in the viewport, with the camera at `view` and the pointer
/// at `pointer` when it is shown, both in logical points, if one is open.
pub fn publish(frame: &Frame, view: Rect, pointer: Option<(f64, f64)>, scale: u32) {
    let Some(viewport) = VIEWPORT.get() else {
        return;
    };
    // The window is reading the last one; skip this tick rather than wait.
    let Ok(mut latest) = viewport.latest.try_lock() else {
        return;
    };
    let s = scale.max(1) as f64;
    latest.seq += 1;
    let seq = latest.seq;
    let snap = &mut latest.snap;
    snap.seq = seq;
    snap.width = frame.width;
    snap.height = frame.height;
    snap.view = Rect::new(view.x * s, view.y * s, view.w * s, view.h * s);
    snap.pointer = pointer.map(|(x, y)| (x * s, y * s));
    snap.pixels.clear();
    snap.pixels.extend_from_slice(&frame.pixels);
}

/// Close the viewport window, if one is open, and wait for its thread.
pub fn close() {
    let Some(viewport) = VIEWPORT.get() else {
        return;
    };
    let hwnd = viewport.window.load(Ordering::Acquire) as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    }
    if let Some(thread) = viewport.thread.lock().unwrap().take() {
        let _ = thread.join();
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Make the window, shown without taking focus from the user, with a
/// client area of the screen's shape no larger than `MAX_WINDOW`, in the
/// display's points, as a scaled desktop sizes windows.
fn make_window(screen: (u32, u32)) -> io::Result<HWND> {
    // Crisp on a scaled display, rather than stretched by Windows.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let class = wide("matinee-viewport");
    let title = wide("matinee");
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        lpszClassName: class.as_ptr(),
        ..unsafe { mem::zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let (w, h) = fit(screen, MAX_WINDOW);
    let percent = super::display::display_scale().unwrap_or(100) as i32;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: w as i32 * percent / 100,
        bottom: h as i32 * percent / 100,
    };
    unsafe { AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0) };
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        let e = io::Error::last_os_error();
        return Err(io::Error::new(
            e.kind(),
            format!("--view could not open its window: {e}"),
        ));
    }
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 1, POLL_MS, None);
    }
    Ok(hwnd)
}

/// Run the window until it is closed.
fn run() {
    let mut msg: MSG = unsafe { mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The window's size inside its frame, in pixels.
fn client_size(hwnd: HWND) -> (usize, usize) {
    let mut r: RECT = unsafe { mem::zeroed() };
    unsafe { GetClientRect(hwnd, &mut r) };
    (
        (r.right - r.left).max(0) as usize,
        (r.bottom - r.top).max(0) as usize,
    )
}

/// Draw the window's picture again, at its current size, from its copy of
/// the frame.
fn redraw(hwnd: HWND) {
    let size = client_size(hwnd);
    SHOWN.with(|shown| {
        let (snap, picture, at) = &mut *shown.borrow_mut();
        picture.resize(size.0 * size.1, 0);
        compose(snap, picture, size);
        *at = size;
    });
    unsafe { InvalidateRect(hwnd, ptr::null(), 0) };
}

/// Take the session's latest frame into the window thread's copy, if it is
/// newer than the one there. Whether it was.
fn take_latest() -> bool {
    let Some(viewport) = VIEWPORT.get() else {
        return false;
    };
    let latest = viewport.latest.lock().unwrap();
    SHOWN.with(|shown| {
        let snap = &mut shown.borrow_mut().0;
        if latest.seq == snap.seq || latest.snap.pixels.is_empty() {
            return false;
        }
        snap.seq = latest.snap.seq;
        snap.width = latest.snap.width;
        snap.height = latest.snap.height;
        snap.view = latest.snap.view;
        snap.pointer = latest.snap.pointer;
        snap.pixels.clone_from(&latest.snap.pixels);
        true
    })
}

/// Copy the window's picture to the screen.
fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = unsafe { mem::zeroed() };
    let dc = unsafe { BeginPaint(hwnd, &mut ps) };
    SHOWN.with(|shown| {
        let (_, picture, (w, h)) = &*shown.borrow();
        if *w == 0 || *h == 0 || picture.len() != w * h {
            return;
        }
        let mut info: BITMAPINFO = unsafe { mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: *w as i32,
            // Negative: rows from the top.
            biHeight: -(*h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { mem::zeroed() }
        };
        unsafe {
            SetDIBitsToDevice(
                dc,
                0,
                0,
                *w as u32,
                *h as u32,
                0,
                0,
                0,
                *h as u32,
                picture.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
            )
        };
    });
    unsafe { EndPaint(hwnd, &ps) };
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER => {
            if take_latest() {
                redraw(hwnd);
            }
            0
        }
        WM_SIZE => {
            redraw(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        // Closing the window leaves the run going.
        WM_CLOSE => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            if let Some(viewport) = VIEWPORT.get() {
                viewport.window.store(0, Ordering::Release);
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
