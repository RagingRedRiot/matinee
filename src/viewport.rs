//! A live window onto a running story, for watching it work.
//!
//! `--view` opens one window on the caller's desktop that shows the whole
//! session as it runs, with the camera's view outlined and the rest of the
//! screen dimmed, so each move the camera makes can be seen.
//!
//! The window never reaches into the boundary. Stories run where the
//! desktop's display is out of sight, and opening it to them would let
//! every app inside reach the real desktop. The viewer runs as its own
//! process instead, forked before any boundary, and reads frames from
//! memory it shares with the session: each tick the session overwrites the
//! latest frame, and the viewer shows whichever frame is there when it is
//! ready for one. The session never waits for the viewer, so watching a
//! story does not change its recording.
//!
//! The viewer is a process rather than a thread because a boundary can
//! only be created by a single-threaded process, and `record` creates one
//! per theme.

use crate::{camera::Rect, compositor::Frame};
use std::{
    io,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    sync::{
        OnceLock,
        atomic::{AtomicU32, AtomicU64, Ordering, fence},
    },
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The camera outline: the same blue as click ripples.
const OUTLINE: u32 = 0xff3b82f6;
const OUTLINE_WIDTH: usize = 2;
/// How bright the screen outside the camera's view stays, out of 256.
const DIM: u32 = 110;
/// Around the session when the window's shape differs from it.
const BACKGROUND: u32 = 0xff111827;
/// The largest window the viewer asks for; the desktop may resize it.
const MAX_WINDOW: (u32, u32) = (1280, 800);
/// How long the viewer waits for display events before looking for a new
/// frame, in milliseconds.
const POLL_MS: i32 = 16;

/// The start of the shared memory. Written only by the session.
#[repr(C)]
struct Header {
    /// Odd while a frame is being written; two more for each frame.
    seq: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    scale: AtomicU32,
    /// The camera's view in logical points, as `f64` bits.
    view: [AtomicU64; 4],
    /// The pointer in logical points, as `f64` bits; NaN while hidden.
    pointer: [AtomicU64; 2],
}

/// Where the pixels start, after the header.
const PIXELS_AT: usize = size_of::<Header>().next_multiple_of(64);

/// Memory shared by the session and the viewer: a header, then room for
/// one frame of `capacity` pixels. Every process forked after it is made
/// shares it.
#[derive(Clone, Copy)]
struct Shared {
    base: *mut u8,
    capacity: usize,
}

// The header is atomic, and pixel reads are checked against `seq`.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn new(capacity: usize) -> io::Result<Shared> {
        let len = PIXELS_AT + capacity * 4;
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Shared {
            base: base.cast(),
            capacity,
        })
    }

    fn header(&self) -> &Header {
        unsafe { &*self.base.cast::<Header>() }
    }

    fn pixels(&self) -> *mut u32 {
        unsafe { self.base.add(PIXELS_AT).cast() }
    }

    /// Replace the latest frame. Never blocks; a frame too big for the
    /// memory is skipped.
    fn write(&self, frame: &Frame, view: Rect, pointer: Option<(f64, f64)>, scale: u32) {
        let n = frame.width * frame.height;
        if n > self.capacity || frame.pixels.len() < n {
            return;
        }
        let h = self.header();
        let seq = h.seq.load(Ordering::Relaxed);
        h.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        h.width.store(frame.width as u32, Ordering::Relaxed);
        h.height.store(frame.height as u32, Ordering::Relaxed);
        h.scale.store(scale, Ordering::Relaxed);
        for (slot, value) in h.view.iter().zip([view.x, view.y, view.w, view.h]) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        let (px, py) = pointer.unwrap_or((f64::NAN, f64::NAN));
        h.pointer[0].store(px.to_bits(), Ordering::Relaxed);
        h.pointer[1].store(py.to_bits(), Ordering::Relaxed);
        unsafe { std::ptr::copy_nonoverlapping(frame.pixels.as_ptr(), self.pixels(), n) };
        h.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// Copy the latest frame into `snap` if it is newer than the one there
    /// and was not being written meanwhile. Returns whether it copied.
    fn read(&self, snap: &mut Snapshot) -> bool {
        let h = self.header();
        let seq = h.seq.load(Ordering::Acquire);
        if seq & 1 == 1 || seq == snap.seq {
            return false;
        }
        let width = h.width.load(Ordering::Relaxed) as usize;
        let height = h.height.load(Ordering::Relaxed) as usize;
        let scale = h.scale.load(Ordering::Relaxed).max(1) as f64;
        let [x, y, w, hh] = [0, 1, 2, 3].map(|i| f64::from_bits(h.view[i].load(Ordering::Relaxed)));
        let [px, py] = [0, 1].map(|i| f64::from_bits(h.pointer[i].load(Ordering::Relaxed)));
        let n = width * height;
        if n > self.capacity {
            return false;
        }
        snap.pixels.resize(n, 0);
        unsafe { std::ptr::copy_nonoverlapping(self.pixels(), snap.pixels.as_mut_ptr(), n) };
        fence(Ordering::Acquire);
        // A frame written while copying is torn; skip it for the next.
        if h.seq.load(Ordering::Relaxed) != seq {
            return false;
        }
        snap.seq = seq;
        snap.width = width;
        snap.height = height;
        snap.view = Rect::new(x * scale, y * scale, w * scale, hh * scale);
        snap.pointer = (!px.is_nan() && !py.is_nan()).then_some((px * scale, py * scale));
        true
    }
}

/// The viewer's copy of a frame.
struct Snapshot {
    seq: u64,
    width: usize,
    height: usize,
    /// The camera's view in the frame's pixels.
    view: Rect,
    /// Where the pointer is in the frame's pixels, while it is shown.
    pointer: Option<(f64, f64)>,
    pixels: Vec<u32>,
}

impl Default for Snapshot {
    fn default() -> Snapshot {
        Snapshot {
            seq: 0,
            width: 0,
            height: 0,
            view: Rect::new(0.0, 0.0, 0.0, 0.0),
            pointer: None,
            pixels: vec![],
        }
    }
}

struct Viewport {
    shared: Shared,
    viewer: libc::pid_t,
}

static VIEWPORT: OnceLock<Viewport> = OnceLock::new();

/// Open the viewport window for frames of up to `capacity` pixels, sized
/// at first for a screen of `screen` points. Call it before any boundary
/// is created, while the process has one thread; it returns once the
/// window's display is connected, or says why it could not be.
pub fn open(capacity: usize, screen: (u32, u32)) -> io::Result<()> {
    if VIEWPORT.get().is_some() {
        return Err(io::Error::other("the viewport is already open"));
    }
    let shared = Shared::new(capacity)?;
    let parent = unsafe { libc::getpid() };
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let [read_end, write_end] = fds;
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            unsafe { libc::close(read_end) };
            let ready = unsafe { OwnedFd::from_raw_fd(write_end) };
            // Go when matinee does, however it ends.
            unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
            if unsafe { libc::getppid() } != parent {
                unsafe { libc::_exit(0) };
            }
            let code = match Viewer::connect(shared, screen) {
                Ok((mut viewer, mut queue)) => {
                    drop(ready);
                    match viewer.run(&mut queue) {
                        Ok(()) => 0,
                        Err(e) => {
                            eprintln!("matinee: viewport: {e}");
                            1
                        }
                    }
                }
                Err(e) => {
                    let message = format!("--view needs a Wayland desktop: {e}");
                    unsafe {
                        libc::write(ready.as_raw_fd(), message.as_ptr().cast(), message.len())
                    };
                    1
                }
            };
            unsafe { libc::_exit(code) }
        }
        viewer => {
            unsafe { libc::close(write_end) };
            let message = read_all(read_end);
            if !message.is_empty() {
                let mut status = 0;
                unsafe { libc::waitpid(viewer, &mut status, 0) };
                return Err(io::Error::other(message));
            }
            let _ = VIEWPORT.set(Viewport { shared, viewer });
            Ok(())
        }
    }
}

/// Show `frame` in the viewport, with the camera at `view` and the pointer
/// at `pointer` when it is shown, both in logical points, if one is open.
pub fn publish(frame: &Frame, view: Rect, pointer: Option<(f64, f64)>, scale: u32) {
    if let Some(viewport) = VIEWPORT.get() {
        viewport.shared.write(frame, view, pointer, scale);
    }
}

/// Close the viewport window, if one is open.
pub fn close() {
    if let Some(viewport) = VIEWPORT.get() {
        let mut status = 0;
        unsafe {
            libc::kill(viewport.viewer, libc::SIGTERM);
            libc::waitpid(viewport.viewer, &mut status, 0);
        }
    }
}

fn read_all(fd: libc::c_int) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        } else if n == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break;
        }
    }
    unsafe { libc::close(fd) };
    String::from_utf8_lossy(&out).into_owned()
}

/// The window, as a Wayland client of the caller's desktop.
struct Viewer {
    shared: Shared,
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    _xdg: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
    /// The size the window picks when the desktop leaves it the choice.
    own_size: (i32, i32),
    /// The size from the latest toplevel configure.
    pending: (i32, i32),
    size: (i32, i32),
    configured: bool,
    closed: bool,
    /// A frame callback is outstanding; drawing waits for it.
    waiting: bool,
    /// Something other than a new frame needs a redraw, like a resize.
    dirty: bool,
    buffers: Vec<Buffer>,
    /// Bumped when the buffers are replaced, so releases of old ones are
    /// ignored.
    generation: u32,
    snapshot: Snapshot,
}

struct Buffer {
    buffer: wl_buffer::WlBuffer,
    pixels: *mut u32,
    size: (i32, i32),
    busy: bool,
    _fd: OwnedFd,
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        let len = (self.size.0 * self.size.1 * 4) as usize;
        unsafe { libc::munmap(self.pixels.cast(), len) };
    }
}

fn wayland(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Accept a Wayland error that only says to try again later.
fn would_block(e: wayland_client::backend::WaylandError) -> io::Result<()> {
    match e {
        wayland_client::backend::WaylandError::Io(e) if e.kind() == io::ErrorKind::WouldBlock => {
            Ok(())
        }
        e => Err(wayland(e)),
    }
}

impl Viewer {
    fn connect(shared: Shared, screen: (u32, u32)) -> io::Result<(Viewer, EventQueue<Viewer>)> {
        let conn = Connection::connect_to_env().map_err(wayland)?;
        let (globals, queue) = registry_queue_init::<Viewer>(&conn).map_err(wayland)?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor =
            globals.bind(&qh, 1..=6, ()).map_err(wayland)?;
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).map_err(wayland)?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(wayland)?;
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title("matinee".into());
        toplevel.set_app_id("matinee".into());
        surface.commit();
        let own_size = fit(screen, MAX_WINDOW);
        let viewer = Viewer {
            shared,
            shm,
            surface,
            _xdg: xdg,
            _toplevel: toplevel,
            own_size: (own_size.0 as i32, own_size.1 as i32),
            pending: (0, 0),
            size: (0, 0),
            configured: false,
            closed: false,
            waiting: false,
            dirty: true,
            buffers: vec![],
            generation: 0,
            snapshot: Snapshot::default(),
        };
        conn.flush().map_err(wayland)?;
        Ok((viewer, queue))
    }

    /// Show frames until the window is closed or the display goes away.
    fn run(&mut self, queue: &mut EventQueue<Viewer>) -> io::Result<()> {
        let qh = queue.handle();
        while !self.closed {
            // A full socket, or a wakeup before a whole message arrived,
            // only means waiting for the next turn.
            queue.flush().or_else(would_block)?;
            if let Some(guard) = queue.prepare_read() {
                let mut fd = [libc::pollfd {
                    fd: guard.connection_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                }];
                unsafe { libc::poll(fd.as_mut_ptr(), 1, POLL_MS) };
                if fd[0].revents & libc::POLLIN != 0 {
                    guard.read().map(drop).or_else(would_block)?;
                }
            }
            queue.dispatch_pending(self).map_err(wayland)?;
            self.draw(&qh)?;
        }
        Ok(())
    }

    /// Draw the latest frame, if there is a new one or the window changed,
    /// and the desktop is ready for it.
    fn draw(&mut self, qh: &QueueHandle<Viewer>) -> io::Result<()> {
        if !self.configured || self.waiting {
            return Ok(());
        }
        let fresh = self.shared.read(&mut self.snapshot);
        if !fresh && !self.dirty {
            return Ok(());
        }
        if self.buffers.first().is_none_or(|b| b.size != self.size) {
            self.generation += 1;
            self.buffers = (0..2)
                .map(|i| self.buffer(i, qh))
                .collect::<io::Result<_>>()?;
        }
        let Some(index) = self.buffers.iter().position(|b| !b.busy) else {
            // Both are still on screen; draw this frame once one is free.
            self.dirty = true;
            return Ok(());
        };
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        let buffer = &mut self.buffers[index];
        let pixels = unsafe { std::slice::from_raw_parts_mut(buffer.pixels, w * h) };
        compose(&self.snapshot, pixels, (w, h));
        buffer.busy = true;
        self.surface.attach(Some(&buffer.buffer), 0, 0);
        self.surface.damage_buffer(0, 0, self.size.0, self.size.1);
        self.surface.frame(qh, ());
        self.surface.commit();
        self.waiting = true;
        self.dirty = false;
        Ok(())
    }

    /// A buffer the window's size, in shared memory the desktop maps.
    fn buffer(&self, index: usize, qh: &QueueHandle<Viewer>) -> io::Result<Buffer> {
        let (w, h) = self.size;
        let len = (w * h * 4) as usize;
        let fd = unsafe { libc::memfd_create(c"matinee-viewport".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let pixels = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if pixels == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let pool = self.shm.create_pool(fd.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            w,
            h,
            w * 4,
            wl_shm::Format::Argb8888,
            qh,
            (self.generation, index),
        );
        pool.destroy();
        Ok(Buffer {
            buffer,
            pixels: pixels.cast(),
            size: self.size,
            busy: false,
            _fd: fd,
        })
    }
}

/// The largest size with `size`'s shape that fits in `bounds`, never
/// larger than `size` itself.
fn fit(size: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    let f = (bounds.0 as f64 / w).min(bounds.1 as f64 / h).min(1.0);
    (
        ((w * f).round() as u32).max(1),
        ((h * f).round() as u32).max(1),
    )
}

/// Draw `snap` into `dst`, `size` pixels: the frame scaled to fit and
/// centered, the screen outside the camera's view dimmed, the view
/// outlined, and the pointer, at the size recordings draw it.
fn compose(snap: &Snapshot, dst: &mut [u32], (dw, dh): (usize, usize)) {
    dst.fill(BACKGROUND);
    let (sw, sh) = (snap.width, snap.height);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return;
    }
    let f = (dw as f64 / sw as f64).min(dh as f64 / sh as f64);
    let ow = ((sw as f64 * f).round() as usize).clamp(1, dw);
    let oh = ((sh as f64 * f).round() as usize).clamp(1, dh);
    let (ox, oy) = ((dw - ow) / 2, (dh - oh) / 2);

    // The view in the window's pixels.
    let v = snap.view;
    let vx0 = ox as f64 + v.x * ow as f64 / sw as f64;
    let vy0 = oy as f64 + v.y * oh as f64 / sh as f64;
    let vx1 = vx0 + v.w * ow as f64 / sw as f64;
    let vy1 = vy0 + v.h * oh as f64 / sh as f64;

    for y in 0..oh {
        // Each window pixel averages the frame pixels it covers.
        let sy0 = y * sh / oh;
        let sy1 = ((y + 1) * sh / oh).max(sy0 + 1);
        let cy = (oy + y) as f64 + 0.5;
        for x in 0..ow {
            let sx0 = x * sw / ow;
            let sx1 = ((x + 1) * sw / ow).max(sx0 + 1);
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for sy in sy0..sy1 {
                for &p in &snap.pixels[sy * sw + sx0..sy * sw + sx1] {
                    r += (p >> 16) & 0xff;
                    g += (p >> 8) & 0xff;
                    b += p & 0xff;
                }
            }
            let count = ((sy1 - sy0) * (sx1 - sx0)) as u32;
            let (mut r, mut g, mut b) = (r / count, g / count, b / count);
            let cx = (ox + x) as f64 + 0.5;
            if cx < vx0 || cx > vx1 || cy < vy0 || cy > vy1 {
                (r, g, b) = (r * DIM / 256, g * DIM / 256, b * DIM / 256);
            }
            dst[(oy + y) * dw + ox + x] = 0xff00_0000 | r << 16 | g << 8 | b;
        }
    }

    // The outline, just inside the view's edges and within the frame.
    let clamp_x = |v: f64| (v.round().max(ox as f64) as usize).min(ox + ow);
    let clamp_y = |v: f64| (v.round().max(oy as f64) as usize).min(oy + oh);
    let (x0, x1, y0, y1) = (clamp_x(vx0), clamp_x(vx1), clamp_y(vy0), clamp_y(vy1));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let t = OUTLINE_WIDTH.min((x1 - x0) / 2).min((y1 - y0) / 2).max(1);
    for y in y0..y1 {
        let row = &mut dst[y * dw..(y + 1) * dw];
        if y < y0 + t || y >= y1 - t {
            row[x0..x1].fill(OUTLINE);
        } else {
            row[x0..x0 + t].fill(OUTLINE);
            row[x1 - t..x1].fill(OUTLINE);
        }
    }

    if let Some((px, py)) = snap.pointer {
        let at = (
            (ox as f64 + px * ow as f64 / sw as f64).round() as i32,
            (oy as f64 + py * oh as f64 / sh as f64).round() as i32,
        );
        crate::record::draw_pointer(dst, (dw, dh), at, 0xff00_0000);
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Viewer {
    fn event(
        _: &mut Self,
        wm: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Viewer {
    fn event(
        viewer: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            let (w, h) = viewer.pending;
            viewer.size = if w > 0 && h > 0 {
                (w, h)
            } else {
                viewer.own_size
            };
            viewer.configured = true;
            viewer.dirty = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Viewer {
    fn event(
        viewer: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                viewer.pending = (width, height);
            }
            xdg_toplevel::Event::Close => viewer.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Viewer {
    fn event(
        viewer: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            viewer.waiting = false;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, (u32, usize)> for Viewer {
    fn event(
        viewer: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        &(generation, index): &(u32, usize),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if generation == viewer.generation {
                if let Some(buffer) = viewer.buffers.get_mut(index) {
                    buffer.busy = false;
                }
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Viewer {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Viewer: ignore wl_compositor::WlCompositor);
delegate_noop!(Viewer: ignore wl_shm::WlShm);
delegate_noop!(Viewer: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Viewer: ignore wl_surface::WlSurface);

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: usize, height: usize, color: u32) -> Frame {
        Frame {
            width,
            height,
            pixels: vec![color; width * height],
        }
    }

    #[test]
    fn a_published_frame_is_read_once() {
        let shared = Shared::new(16).unwrap();
        let mut snap = Snapshot::default();
        assert!(!shared.read(&mut snap), "nothing published yet");
        shared.write(
            &frame(4, 2, 0xff112233),
            Rect::new(1.0, 0.0, 1.0, 1.0),
            None,
            2,
        );
        assert!(shared.read(&mut snap));
        assert_eq!((snap.width, snap.height), (4, 2));
        assert_eq!(snap.pixels, vec![0xff112233; 8]);
        assert_eq!(
            snap.view,
            Rect::new(2.0, 0.0, 2.0, 2.0),
            "view in frame pixels"
        );
        assert!(!shared.read(&mut snap), "the same frame is not read twice");
    }

    #[test]
    fn a_frame_too_big_for_the_memory_is_skipped() {
        let shared = Shared::new(4).unwrap();
        shared.write(
            &frame(4, 2, 0xffffffff),
            Rect::new(0.0, 0.0, 4.0, 2.0),
            None,
            1,
        );
        assert!(!shared.read(&mut Snapshot::default()));
    }

    #[test]
    fn compose_dims_outside_the_view_and_outlines_it() {
        let snap = Snapshot {
            seq: 2,
            width: 40,
            height: 20,
            view: Rect::new(10.0, 5.0, 20.0, 10.0),
            pointer: None,
            pixels: vec![0xffffffff; 800],
        };
        let mut dst = vec![0; 40 * 20];
        compose(&snap, &mut dst, (40, 20));
        assert_eq!(
            dst[0],
            0xff00_0000 | ((255 * DIM / 256) * 0x010101),
            "outside is dimmed"
        );
        assert_eq!(dst[10 * 40 + 20], 0xffffffff, "inside is untouched");
        assert_eq!(dst[5 * 40 + 20], OUTLINE, "top edge");
        assert_eq!(dst[10 * 40 + 10], OUTLINE, "left edge");
        assert_eq!(dst[10 * 40 + 29], OUTLINE, "right edge");
    }

    #[test]
    fn compose_fits_and_centers_the_frame() {
        let snap = Snapshot {
            seq: 2,
            width: 40,
            height: 20,
            view: Rect::new(0.0, 0.0, 40.0, 20.0),
            pointer: None,
            pixels: vec![0xff000000; 800],
        };
        // Twice the frame's height: the frame is 20x10, centered, with
        // background above and below.
        let mut dst = vec![0; 20 * 20];
        compose(&snap, &mut dst, (20, 20));
        assert_eq!(dst[2 * 20 + 10], BACKGROUND, "above");
        assert_eq!(dst[17 * 20 + 10], BACKGROUND, "below");
        assert_eq!(dst[5 * 20 + 10], OUTLINE, "the frame's top edge");
        assert_eq!(dst[10 * 20 + 10], 0xff000000, "the frame itself");
    }

    #[test]
    fn the_pointer_is_read_in_frame_pixels() {
        let shared = Shared::new(16).unwrap();
        let mut snap = Snapshot::default();
        let view = Rect::new(0.0, 0.0, 2.0, 1.0);
        shared.write(&frame(4, 2, 0xff000000), view, Some((1.5, 0.5)), 2);
        assert!(shared.read(&mut snap));
        assert_eq!(snap.pointer, Some((3.0, 1.0)));
    }

    #[test]
    fn compose_draws_the_pointer_where_it_is() {
        let snap = Snapshot {
            seq: 2,
            width: 80,
            height: 80,
            view: Rect::new(0.0, 0.0, 80.0, 80.0),
            pointer: Some((40.0, 20.0)),
            pixels: vec![0xff808080; 6400],
        };
        // Half size: the pointer's tip lands at 20,10.
        let mut dst = vec![0; 40 * 40];
        compose(&snap, &mut dst, (40, 40));
        assert_eq!(dst[10 * 40 + 20], 0xff00_0000, "the tip's outline");
        assert_eq!(dst[15 * 40 + 22], 0xffff_ffff, "its fill");
    }

    #[test]
    fn fit_keeps_the_shape_and_never_grows() {
        assert_eq!(fit((2560, 1440), (1280, 800)), (1280, 720));
        assert_eq!(fit((800, 400), (1280, 800)), (800, 400));
    }
}
