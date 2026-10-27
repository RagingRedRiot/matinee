//! The built-in compositor, driven by a small Wayland client in a thread.

use matinee::platform::{Button, Window, compositor::Compositor};
use std::{
    os::{fd::AsFd, unix::net::UnixStream},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager,
        wl_data_offer, wl_data_source, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const SCALE: u32 = 2;
/// The size a window picks when the compositor leaves it the choice.
const OWN_SIZE: (i32, i32) = (120, 90);
const SIZE: (u32, u32) = (400, 300);
const BLUE: u32 = 0xff3366cc;
const ORANGE: u32 = 0xffee8833;
const GREEN: u32 = 0xff33aa55;
/// A drag icon's logical size.
const ICON: i32 = 20;

/// What the client asks of its windows.
struct Spec {
    title: &'static str,
    color: u32,
    /// A dialog of this logical size, parented to the first window.
    dialog: Option<(i32, i32)>,
    /// Pressing in this window starts a drag whose icon has its hotspot
    /// this many points in from the icon's top-left corner.
    drag_icon: Option<(i32, i32)>,
}

/// What the client reports back.
#[derive(Debug)]
enum Report {
    /// A frame callback arrived, with the time the compositor gave it.
    Frame(u32),
    /// The keyboard entered the window with this title.
    Focus(&'static str),
}

struct Client {
    shm: wl_shm::WlShm,
    compositor: wl_compositor::WlCompositor,
    data: wl_data_device_manager::WlDataDeviceManager,
    device: wl_data_device::WlDataDevice,
    surfaces: Vec<Probe>,
    reports: mpsc::Sender<Report>,
}

struct Probe {
    spec: Spec,
    surface: wl_surface::WlSurface,
    _xdg: xdg_surface::XdgSurface,
    size: (i32, i32),
}

fn run_client(
    socket: PathBuf,
    specs: Vec<Spec>,
    reports: mpsc::Sender<Report>,
    stop: mpsc::Receiver<()>,
) {
    let conn = Connection::from_socket(UnixStream::connect(socket).unwrap()).unwrap();
    let (globals, mut queue) = registry_queue_init::<Client>(&conn).unwrap();
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=6, ()).unwrap();
    let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ()).unwrap();
    seat.get_keyboard(&qh, ());
    seat.get_pointer(&qh, ());
    let data: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).unwrap();
    let mut client = Client {
        shm: globals.bind(&qh, 1..=1, ()).unwrap(),
        device: data.get_data_device(&seat, &qh, ()),
        data,
        compositor: compositor.clone(),
        surfaces: vec![],
        reports,
    };
    let mut parent: Option<xdg_toplevel::XdgToplevel> = None;
    for spec in specs {
        let surface = compositor.create_surface(&qh, ());
        surface.set_buffer_scale(SCALE as i32);
        let index = client.surfaces.len();
        let xdg = wm.get_xdg_surface(&surface, &qh, index);
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title(spec.title.into());
        if spec.dialog.is_some() {
            toplevel.set_parent(parent.as_ref());
        } else {
            parent = Some(toplevel);
        }
        surface.commit();
        client.surfaces.push(Probe {
            size: spec.dialog.unwrap_or((0, 0)),
            spec,
            surface,
            _xdg: xdg,
        });
    }
    while matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
        queue.flush().unwrap();
        if let Some(guard) = queue.prepare_read() {
            let mut fd = [libc::pollfd {
                fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd()),
                events: libc::POLLIN,
                revents: 0,
            }];
            unsafe { libc::poll(fd.as_mut_ptr(), 1, 10) };
            if fd[0].revents & libc::POLLIN != 0 {
                let _ = guard.read();
            }
        }
        queue.dispatch_pending(&mut client).unwrap();
    }
}

impl Client {
    /// Draw window `index` in its color at its current size.
    fn draw(&mut self, index: usize, qh: &QueueHandle<Client>) {
        let probe = &self.surfaces[index];
        let buffer = self.buffer(probe.size, probe.spec.color, qh);
        let probe = &self.surfaces[index];
        let (w, h) = (probe.size.0 * SCALE as i32, probe.size.1 * SCALE as i32);
        probe.surface.attach(Some(&buffer), 0, 0);
        probe.surface.damage_buffer(0, 0, w, h);
        probe.surface.frame(qh, ());
        probe.surface.commit();
    }

    /// A buffer of one color, `size` points at the test's scale.
    fn buffer(
        &self,
        size: (i32, i32),
        color: u32,
        qh: &QueueHandle<Client>,
    ) -> wl_buffer::WlBuffer {
        let (w, h) = (size.0 * SCALE as i32, size.1 * SCALE as i32);
        let stride = w * 4;
        let len = (stride * h) as usize;
        let file = tempfile();
        let pixels: Vec<u8> = std::iter::repeat_n(color.to_le_bytes(), (w * h) as usize)
            .flatten()
            .collect();
        std::io::Write::write_all(&mut &file, &pixels).unwrap();
        let pool = self.shm.create_pool(file.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, qh, ());
        pool.destroy();
        buffer
    }

    /// Start dragging from the first window with a drag icon, the way
    /// GTK does: the icon's buffer is offset by minus its hotspot.
    fn start_drag(&mut self, serial: u32, qh: &QueueHandle<Client>) {
        let Some(probe) = self.surfaces.first() else {
            return;
        };
        let Some((hx, hy)) = probe.spec.drag_icon else {
            return;
        };
        let origin = probe.surface.clone();
        let source = self.data.create_data_source(qh, ());
        source.offer("text/plain".into());
        let icon = self.compositor.create_surface(qh, ());
        self.device
            .start_drag(Some(&source), &origin, Some(&icon), serial);
        icon.set_buffer_scale(SCALE as i32);
        let buffer = self.buffer((ICON, ICON), GREEN, qh);
        if icon.version() >= 5 {
            icon.attach(Some(&buffer), 0, 0);
            icon.offset(-hx, -hy);
        } else {
            icon.attach(Some(&buffer), -hx, -hy);
        }
        icon.damage_buffer(0, 0, ICON * SCALE as i32, ICON * SCALE as i32);
        icon.commit();
    }
}

fn tempfile() -> std::fs::File {
    let path = std::env::temp_dir().join(format!(
        "matinee-shm-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    std::fs::remove_file(path).unwrap();
    file
}

impl Dispatch<xdg_surface::XdgSurface, usize> for Client {
    fn event(
        client: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            let probe = &mut client.surfaces[*index];
            // Dialogs keep their own size; other windows take the size
            // they are given, or pick one when given none.
            if probe.spec.dialog.is_none() {
                let (w, h) = PENDING.with(|p| p.get());
                probe.size = if w > 0 && h > 0 { (w, h) } else { OWN_SIZE };
            }
            client.draw(*index, qh);
        }
    }
}

thread_local! {
    /// The size from the latest toplevel configure, applied on the
    /// xdg_surface configure that follows it.
    static PENDING: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Client {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            PENDING.with(|p| p.set((width, height)));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { callback_data } = event {
            let _ = client.reports.send(Report::Frame(callback_data));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { surface, .. } = event {
            if let Some(probe) = client.surfaces.iter().find(|p| p.surface == surface) {
                let _ = client.reports.send(Report::Focus(probe.spec.title));
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
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

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
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

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            client.start_drag(serial, qh);
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_data_device::WlDataDevice,
        _: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(Client, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(Client: ignore wl_data_source::WlDataSource);
delegate_noop!(Client: ignore wl_data_offer::WlDataOffer);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);

/// A compositor with a client connected in another thread.
struct Harness {
    compositor: Compositor,
    reports: mpsc::Receiver<Report>,
    stop: Option<mpsc::Sender<()>>,
    client: Option<std::thread::JoinHandle<()>>,
    dir: PathBuf,
}

impl Harness {
    fn start(name: &str, specs: Vec<Spec>) -> Harness {
        let dir = std::env::temp_dir().join(format!("matinee-wl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("wayland-1");
        let compositor = Compositor::new(&socket, SIZE, SCALE).unwrap();
        let (report_tx, reports) = mpsc::channel();
        let (stop_tx, stop) = mpsc::channel();
        let client = std::thread::spawn(move || run_client(socket, specs, report_tx, stop));
        Harness {
            compositor,
            reports,
            stop: Some(stop_tx),
            client: Some(client),
            dir,
        }
    }

    /// Dispatch until `done` holds, or fail after five seconds.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Compositor) -> bool) {
        let start = Instant::now();
        while !done(&mut self.compositor) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out waiting for {what}"
            );
            self.compositor.dispatch(Duration::from_millis(10)).unwrap();
        }
    }

    fn windows(&self) -> Vec<Window> {
        self.compositor.windows()
    }

    /// Reports received so far.
    fn drain(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(client) = self.client.take() {
            let _ = self.compositor.dispatch(Duration::ZERO);
            let _ = client.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn mapped(titles: &'static [&'static str]) -> impl FnMut(&mut Compositor) -> bool {
    move |c| {
        let windows = c.windows();
        titles
            .iter()
            .all(|t| windows.iter().any(|w| w.title == *t && w.width > 0))
    }
}

#[test]
fn a_window_fills_the_output_and_renders() {
    let mut h = Harness::start(
        "fill",
        vec![Spec {
            title: "Probe",
            color: BLUE,
            dialog: None,
            drag_icon: None,
        }],
    );
    h.until("the window", mapped(&["Probe"]));
    // It first draws at its own size, then is resized to fill the area.
    h.until("the window at full size", |c| {
        c.windows()
            .iter()
            .any(|w| (w.width, w.height) == (SIZE.0 as i32, SIZE.1 as i32))
    });
    h.compositor.dispatch(Duration::from_millis(50)).unwrap();
    let window = &h.windows()[0];
    assert_eq!((window.x, window.y), (0, 0));
    assert_eq!(
        (window.width, window.height),
        (SIZE.0 as i32, SIZE.1 as i32)
    );
    assert!(window.focused);

    let frame = h.compositor.render(Duration::from_millis(1234)).unwrap();
    assert_eq!((frame.width, frame.height), (800, 600));
    assert_eq!(frame.pixel(0, 0), BLUE);
    assert_eq!(frame.pixel(799, 599), BLUE);
}

#[test]
fn frame_callbacks_carry_the_recording_time() {
    let mut h = Harness::start(
        "time",
        vec![Spec {
            title: "Probe",
            color: BLUE,
            dialog: None,
            drag_icon: None,
        }],
    );
    h.until("the window", mapped(&["Probe"]));
    h.compositor.render(Duration::from_millis(1234)).unwrap();
    let mut times = vec![];
    let start = Instant::now();
    while times.is_empty() && start.elapsed() < Duration::from_secs(5) {
        h.compositor.dispatch(Duration::from_millis(10)).unwrap();
        times.extend(h.drain().into_iter().filter_map(|r| match r {
            Report::Frame(t) => Some(t),
            _ => None,
        }));
    }
    assert_eq!(times, [1234]);
}

#[test]
fn a_dialog_is_centered_at_its_own_size_and_takes_focus() {
    let mut h = Harness::start(
        "dialog",
        vec![
            Spec {
                title: "Main",
                color: BLUE,
                dialog: None,
                drag_icon: None,
            },
            Spec {
                title: "Dialog",
                color: ORANGE,
                dialog: Some((200, 100)),
                drag_icon: None,
            },
        ],
    );
    h.until("both windows", mapped(&["Main", "Dialog"]));
    h.until("the dialog centered", |c| {
        c.windows()
            .iter()
            .any(|w| w.title == "Dialog" && (w.x, w.y) == (100, 100))
    });
    let dialog = h
        .windows()
        .into_iter()
        .find(|w| w.title == "Dialog")
        .unwrap();
    assert_eq!((dialog.width, dialog.height), (200, 100));
    assert!(dialog.focused);

    let frame = h.compositor.render(Duration::ZERO).unwrap();
    assert_eq!(frame.pixel(400, 300), ORANGE, "dialog in the middle");
    assert_eq!(frame.pixel(10, 10), BLUE, "main window around it");

    // Clicking the main window outside the dialog focuses it.
    h.compositor.dispatch(Duration::from_millis(50)).unwrap();
    h.drain();
    h.compositor.pointer_move(20.0, 20.0, Duration::ZERO);
    h.compositor
        .pointer_button(Button::Left, true, Duration::ZERO);
    h.compositor
        .pointer_button(Button::Left, false, Duration::ZERO);
    h.until("main focused", |c| {
        c.windows().iter().any(|w| w.title == "Main" && w.focused)
    });
    let start = Instant::now();
    let mut focus = vec![];
    while !focus.contains(&"Main") && start.elapsed() < Duration::from_secs(5) {
        h.compositor.dispatch(Duration::from_millis(10)).unwrap();
        focus.extend(h.drain().into_iter().filter_map(|r| match r {
            Report::Focus(t) => Some(t),
            _ => None,
        }));
    }
    assert_eq!(focus.last(), Some(&"Main"), "{focus:?}");
}

#[test]
fn window_area_leaves_room_for_a_terminal() {
    let mut h = Harness::start(
        "area",
        vec![Spec {
            title: "Probe",
            color: BLUE,
            dialog: None,
            drag_icon: None,
        }],
    );
    h.compositor.set_window_area(0, 0, 400, 200);
    h.until("the window at the area's size", |c| {
        c.windows()
            .iter()
            .any(|w| (w.width, w.height) == (400, 200))
    });
    // The client redraws at the new size after the configure.
    let start = Instant::now();
    loop {
        h.compositor.dispatch(Duration::from_millis(10)).unwrap();
        let frame = h.compositor.render(Duration::ZERO).unwrap();
        if frame.pixel(10, 390) == BLUE && frame.pixel(10, 410) != BLUE {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "window never filled only the area"
        );
    }
}

#[test]
fn a_drag_icon_sits_at_its_hotspot() {
    let mut h = Harness::start(
        "dragicon",
        vec![Spec {
            title: "Probe",
            color: BLUE,
            dialog: None,
            drag_icon: Some((12, 8)),
        }],
    );
    h.until("the window at full size", |c| {
        c.windows()
            .iter()
            .any(|w| (w.width, w.height) == (SIZE.0 as i32, SIZE.1 as i32))
    });
    h.compositor.dispatch(Duration::from_millis(50)).unwrap();
    h.compositor.pointer_move(100.0, 100.0, Duration::ZERO);
    h.compositor
        .pointer_button(Button::Left, true, Duration::ZERO);
    // The icon's top-left corner is 12 points left of and 8 above the
    // pointer, so it covers 88..108 by 92..112, in device pixels twice that.
    let start = Instant::now();
    let frame = loop {
        h.compositor.dispatch(Duration::from_millis(10)).unwrap();
        let frame = h.compositor.render(Duration::ZERO).unwrap();
        if frame.pixel(2 * 100, 2 * 100) == GREEN {
            break frame;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no drag icon under the pointer"
        );
    };
    assert_eq!(frame.pixel(2 * 89, 2 * 93), GREEN, "top-left of the icon");
    assert_eq!(
        frame.pixel(2 * 107, 2 * 111),
        GREEN,
        "bottom-right of the icon"
    );
    assert_eq!(frame.pixel(2 * 110, 2 * 114), BLUE, "past the icon");
}
