//! The headless Wayland compositor that apps run under.
//!
//! matinee is the compositor, so it renders frames straight into memory and
//! delivers input directly to the seat; no capture or virtual-input protocol
//! is involved. It also decides when frames happen: apps are told a frame
//! was shown only when [`Compositor::render`] runs, at the time it is given,
//! so a recording can run on its own frame timeline.
//!
//! Every toplevel window is laid out to fill the window area, marked as
//! tiled so toolkits drop shadows and rounded corners. A toplevel with a
//! parent, such as a dialog, keeps its own size and is centered on the
//! area. Rendering is done on the CPU with pixman.

use smithay::{
    backend::renderer::{
        Bind, damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
        pixman::PixmanRenderer, utils::on_commit_buffer_handler,
    },
    delegate_compositor, delegate_data_device, delegate_output, delegate_primary_selection,
    delegate_seat, delegate_shm, delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy, Space,
        Window, WindowSurfaceType, find_popup_root_surface,
    },
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::{FilterResult, KeyboardHandle, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, Focus, MotionEvent, PointerHandle},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        pixman,
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel::State as ToplevelState,
        },
        wayland_server::{
            Client, Display, DisplayHandle, ListeningSocket, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer::WlBuffer, wl_seat::WlSeat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface, with_states,
        },
        output::{OutputHandler, OutputManagerState},
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
            primary_selection::{
                PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
            decoration::{XdgDecorationHandler, XdgDecorationState},
        },
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};
use std::{io, os::fd::AsRawFd, path::Path, sync::Arc, time::Duration};

/// Linux input event codes for pointer buttons.
pub const BUTTON_LEFT: u32 = 0x110;
pub const BUTTON_RIGHT: u32 = 0x111;
pub const BUTTON_MIDDLE: u32 = 0x112;

/// The desktop background behind windows, as RGBA.
const BACKGROUND: [f32; 4] = [0.5, 0.55, 0.55, 1.0];

/// One rendered frame of the whole output, in output pixels.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Rows of 0xAARRGGBB pixels.
    pub pixels: Vec<u32>,
}

impl Frame {
    pub fn pixel(&self, x: usize, y: usize) -> u32 {
        self.pixels[y * self.width + x]
    }

    /// Save the frame as an opaque PNG.
    pub fn write_png(&self, path: &std::path::Path) -> io::Result<()> {
        let file = io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let rgb: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect();
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&rgb))
            .map_err(io::Error::other)
    }
}

/// A mapped toplevel window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    pub title: String,
    pub app_id: String,
    /// The window's position and size in logical points.
    pub rect: Rectangle<i32, Logical>,
    /// Whether it has keyboard focus.
    pub focused: bool,
}

pub struct Compositor {
    display: Display<State>,
    socket: ListeningSocket,
    state: State,
    renderer: PixmanRenderer,
    damage: OutputDamageTracker,
    target: pixman::Image<'static, 'static>,
    size: Size<i32, smithay::utils::Physical>,
}

impl Compositor {
    /// Start a compositor listening at `socket` with an output of `size`
    /// logical points, rendered at `scale` device pixels per point.
    pub fn new(socket: &Path, size: (u32, u32), scale: u32) -> io::Result<Compositor> {
        let display = Display::<State>::new().map_err(io::Error::other)?;
        let dh = display.handle();
        let listener = ListeningSocket::bind_absolute(socket.to_owned())
            .map_err(|e| io::Error::other(format!("{}: {e}", socket.display())))?;

        let physical = Size::from(((size.0 * scale) as i32, (size.1 * scale) as i32));
        let output = Output::new(
            "HEADLESS-1".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "matinee".into(),
                model: "headless".into(),
            },
        );
        let mode = Mode {
            size: physical,
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(scale as i32)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);
        output.create_global::<State>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(XkbConfig::default(), 600, 25)
            .map_err(|e| io::Error::other(format!("keyboard: {e:?}")))?;
        seat.add_pointer();

        let mut space = Space::default();
        space.map_output(&output, (0, 0));

        let state = State {
            compositor: CompositorState::new::<State>(&dh),
            xdg_shell: XdgShellState::new::<State>(&dh),
            shm: ShmState::new::<State>(&dh, vec![]),
            output_manager: OutputManagerState::new_with_xdg_output::<State>(&dh),
            data_device: DataDeviceState::new::<State>(&dh),
            primary_selection: PrimarySelectionState::new::<State>(&dh),
            decoration: XdgDecorationState::new::<State>(&dh),
            viewporter: ViewporterState::new::<State>(&dh),
            seat_state,
            seat,
            space,
            popups: PopupManager::default(),
            area: Rectangle::from_size((size.0 as i32, size.1 as i32).into()),
            output,
            dh,
        };

        let target = pixman::Image::new(
            pixman::FormatCode::A8R8G8B8,
            physical.w as usize,
            physical.h as usize,
            true,
        )
        .map_err(|_| io::Error::other("could not allocate the frame"))?;
        let damage = OutputDamageTracker::from_output(&state.output);
        let renderer = PixmanRenderer::new().map_err(io::Error::other)?;

        Ok(Compositor {
            display,
            socket: listener,
            state,
            renderer,
            damage,
            target,
            size: physical,
        })
    }

    /// The part of the output windows are laid out in: x, y, width,
    /// height in logical points.
    pub fn window_area(&self) -> (i32, i32, i32, i32) {
        let a = self.state.area;
        (a.loc.x, a.loc.y, a.size.w, a.size.h)
    }

    /// Lay out toplevel windows in this part of the output, in logical
    /// points, such as everything above a terminal pane.
    pub fn set_window_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.state.area = Rectangle::new((x, y).into(), (w, h).into());
        let windows: Vec<Window> = self.state.space.elements().cloned().collect();
        for window in windows {
            self.state.place(&window);
        }
    }

    /// Accept new clients and handle their requests, waiting up to
    /// `timeout` for something to happen.
    pub fn dispatch(&mut self, timeout: Duration) -> io::Result<()> {
        let mut fds = [
            libc::pollfd {
                fd: self.socket.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.display.backend().poll_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
        if ready < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        while let Some(stream) = self.socket.accept()? {
            self.display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
                .map_err(io::Error::other)?;
        }
        self.display.dispatch_clients(&mut self.state)?;
        self.state.space.refresh();
        self.state.popups.cleanup();
        self.display.flush_clients()?;
        Ok(())
    }

    /// Render the output as shown at `time` on the recording's timeline,
    /// then tell apps that frame was shown so they draw the next one.
    pub fn render(&mut self, time: Duration) -> io::Result<Frame> {
        let state = &mut self.state;
        {
            let mut target = self
                .renderer
                .bind(&mut self.target)
                .map_err(io::Error::other)?;
            smithay::desktop::space::render_output::<
                _,
                WaylandSurfaceRenderElement<PixmanRenderer>,
                _,
                _,
            >(
                &state.output,
                &mut self.renderer,
                &mut target,
                1.0,
                0,
                [&state.space],
                &[],
                &mut self.damage,
                BACKGROUND,
            )
            .map_err(|e| io::Error::other(format!("render: {e:?}")))?;
        }
        let (width, height) = (self.size.w as usize, self.size.h as usize);
        let stride = self.target.stride() / 4;
        let data = unsafe { std::slice::from_raw_parts(self.target.data(), stride * height) };
        let mut pixels = Vec::with_capacity(width * height);
        for row in data.chunks(stride) {
            pixels.extend_from_slice(&row[..width]);
        }

        for window in state.space.elements() {
            window.send_frame(&state.output, time, Some(Duration::ZERO), |_, _| {
                Some(state.output.clone())
            });
        }
        self.display.flush_clients()?;
        Ok(Frame {
            width,
            height,
            pixels,
        })
    }

    /// Mapped toplevel windows, bottom to top.
    pub fn windows(&self) -> Vec<WindowInfo> {
        let focus = self
            .state
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus());
        self.state
            .space
            .elements()
            .filter_map(|window| {
                let toplevel = window.toplevel()?;
                let (title, app_id) = with_states(toplevel.wl_surface(), |states| {
                    let data = states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()?
                        .lock()
                        .unwrap();
                    Some((
                        data.title.clone().unwrap_or_default(),
                        data.app_id.clone().unwrap_or_default(),
                    ))
                })?;
                let loc = self.state.space.element_location(window)?;
                Some(WindowInfo {
                    title,
                    app_id,
                    rect: Rectangle::new(loc, window.geometry().size),
                    focused: focus.as_ref() == Some(toplevel.wl_surface()),
                })
            })
            .collect()
    }

    /// Where the pointer is, in logical points.
    pub fn pointer_location(&self) -> (f64, f64) {
        let at = self.state.pointer().current_location();
        (at.x, at.y)
    }

    /// Move the pointer to `(x, y)` in logical points.
    pub fn pointer_move(&mut self, x: f64, y: f64, time: Duration) {
        let location = Point::from((x, y));
        let under = self.state.surface_under(location);
        let pointer = self.state.pointer();
        pointer.motion(
            &mut self.state,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time: millis(time),
            },
        );
        pointer.frame(&mut self.state);
    }

    /// Press or release a pointer button, such as [`BUTTON_LEFT`]. Pressing
    /// focuses and raises the window under the pointer.
    pub fn pointer_button(&mut self, button: u32, pressed: bool, time: Duration) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.state.pointer();
        if pressed && !pointer.is_grabbed() {
            let location = pointer.current_location();
            let window = self
                .state
                .space
                .element_under(location)
                .map(|(w, _)| w.clone());
            if let Some(window) = window {
                self.state.focus(&window, serial);
            }
        }
        pointer.button(
            &mut self.state,
            &ButtonEvent {
                serial,
                time: millis(time),
                button,
                state: if pressed {
                    smithay::backend::input::ButtonState::Pressed
                } else {
                    smithay::backend::input::ButtonState::Released
                },
            },
        );
        pointer.frame(&mut self.state);
    }

    /// Scroll by `dy` logical points; positive scrolls down.
    pub fn pointer_scroll(&mut self, dy: f64, time: Duration) {
        let pointer = self.state.pointer();
        pointer.axis(
            &mut self.state,
            AxisFrame::new(millis(time))
                .source(smithay::backend::input::AxisSource::Wheel)
                .value(smithay::backend::input::Axis::Vertical, dy)
                .v120(
                    smithay::backend::input::Axis::Vertical,
                    (dy / 15.0 * 120.0) as i32,
                ),
        );
        pointer.frame(&mut self.state);
    }

    /// Press or release a key by its Linux keycode (`KEY_A` is 30).
    pub fn key(&mut self, keycode: u32, pressed: bool, time: Duration) {
        let keyboard = self.state.keyboard();
        keyboard.input::<(), _>(
            &mut self.state,
            // xkb keycodes are offset by 8 from Linux input codes.
            (keycode + 8).into(),
            if pressed {
                smithay::backend::input::KeyState::Pressed
            } else {
                smithay::backend::input::KeyState::Released
            },
            SERIAL_COUNTER.next_serial(),
            millis(time),
            |_, _, _| FilterResult::Forward,
        );
    }
}

fn millis(time: Duration) -> u32 {
    time.as_millis() as u32
}

#[derive(Default)]
struct ClientState {
    compositor: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

struct State {
    compositor: CompositorState,
    xdg_shell: XdgShellState,
    shm: ShmState,
    #[allow(dead_code)]
    output_manager: OutputManagerState,
    data_device: DataDeviceState,
    primary_selection: PrimarySelectionState,
    #[allow(dead_code)]
    decoration: XdgDecorationState,
    #[allow(dead_code)]
    viewporter: ViewporterState,
    seat_state: SeatState<State>,
    seat: Seat<State>,
    space: Space<Window>,
    popups: PopupManager,
    /// Where toplevel windows go, in logical points.
    area: Rectangle<i32, Logical>,
    output: Output,
    dh: DisplayHandle,
}

/// Marks a dialog that has been centered, so it is not moved again.
struct Centered;

/// Marks a toplevel that has been given the window area's size.
struct Sized;

impl State {
    fn pointer(&self) -> PointerHandle<State> {
        self.seat.get_pointer().expect("the seat has a pointer")
    }

    fn keyboard(&self) -> KeyboardHandle<State> {
        self.seat.get_keyboard().expect("the seat has a keyboard")
    }

    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (window, loc) = self.space.element_under(location)?;
        window
            .surface_under(location - loc.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, offset)| (surface, (offset + loc).to_f64()))
    }

    fn focus(&mut self, window: &Window, serial: Serial) {
        self.space.raise_element(window, true);
        for other in self.space.elements() {
            other.set_activated(other == window);
            if let Some(toplevel) = other.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        let surface = window.toplevel().map(|t| t.wl_surface().clone());
        self.keyboard().set_focus(self, surface, serial);
    }

    /// Put a toplevel where it belongs in the window area.
    fn place(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        if toplevel.parent().is_some() {
            let size = window.geometry().size;
            let centered = window.user_data().get::<Centered>().is_some();
            if size.w > 0 && !centered {
                window.user_data().insert_if_missing(|| Centered);
                let loc = self.area.loc
                    + Point::from((
                        (self.area.size.w - size.w) / 2,
                        (self.area.size.h - size.h) / 2,
                    ));
                self.space.map_element(window.clone(), loc, false);
            }
        } else {
            toplevel.with_pending_state(|state| state.size = Some(self.area.size));
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
            self.space.map_element(window.clone(), self.area.loc, false);
        }
    }

    fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    /// Keep a popup inside the output, flipping or sliding it as its
    /// positioner allows.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.window_for(&root) else {
            return;
        };
        let Some(window_loc) = self.space.element_location(&window) else {
            return;
        };
        let mut target = self.space.output_geometry(&self.output).unwrap_or_default();
        target.loc -= window_loc;
        target.loc -= smithay::desktop::get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for(&root) {
                window.on_commit();
                if let Some(toplevel) = window.toplevel() {
                    if !toplevel.is_initial_configure_sent() {
                        toplevel.send_configure();
                    } else if toplevel.parent().is_some() {
                        self.place(&window);
                    } else if window.geometry().size.w > 0
                        && window.user_data().get::<Sized>().is_none()
                    {
                        // Size it to the area only once it has drawn at its
                        // own size. Some layouts (GNOME Calculator's entry)
                        // measure once against the first size they get and
                        // need a real resize to measure again.
                        window.user_data().insert_if_missing(|| Sized);
                        self.place(&window);
                    }
                }
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                let _ = popup.send_configure();
            }
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            for s in [
                ToplevelState::Activated,
                ToplevelState::TiledLeft,
                ToplevelState::TiledRight,
                ToplevelState::TiledTop,
                ToplevelState::TiledBottom,
            ] {
                state.states.set(s);
            }
        });
        let window = Window::new_wayland_window(surface);
        // Every toplevel first draws at a size of its own choosing; see
        // `commit` for why the area's size comes only after that.
        self.space.map_element(window.clone(), self.area.loc, true);
        self.focus(&window, SERIAL_COUNTER.next_serial());
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|state| state.geometry = positioner.get_geometry());
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, serial: Serial) {
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let seat = self.seat.clone();
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else {
            return;
        };
        let keyboard = self.keyboard();
        if keyboard.is_grabbed()
            && !(keyboard.has_grab(serial)
                || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
        {
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }
        keyboard.set_focus(self, grab.current_grab(), serial);
        keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        let pointer = self.pointer();
        if pointer.is_grabbed()
            && !(pointer.has_grab(serial)
                || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
        {
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }
        pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // Hand focus back to the window now on top.
        let top = self
            .space
            .elements()
            .rev()
            .find(|w| w.toplevel().is_some_and(|t| t != &surface))
            .cloned();
        if let Some(window) = top {
            self.focus(&window, SERIAL_COUNTER.next_serial());
        }
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel
            .with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel
            .with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.request_mode(toplevel, DecorationMode::ServerSide);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client.clone());
        set_primary_focus(&self.dh, seat, client);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection
    }
}

impl OutputHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_output!(State);
delegate_viewporter!(State);
