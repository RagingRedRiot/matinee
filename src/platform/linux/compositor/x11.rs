//! X11 apps, through Xwayland, for launches that ask for it with
//! `x11 = true`.
//!
//! Xwayland starts only when a story needs it. matinee is its window
//! manager: X11 toplevels are laid out like Wayland ones, filling the
//! window area, or centered at their own size for dialogs. Menus and
//! tooltips (override-redirect windows) stay where the app puts them.

use super::{Centered, Sized, State};
use smithay::{
    backend::input::KeyState,
    desktop::{PopupKind, Window},
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
    wayland::{
        seat::WaylandFocus,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};
use std::borrow::Cow;

/// What has the keyboard or pointer: a Wayland surface, or an X11 window,
/// which also needs X's own input focus set.
#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Surface(WlSurface),
    X11(X11Surface),
}

impl Focus {
    /// The Wayland surface that receives input for this target.
    pub fn surface(&self) -> Option<WlSurface> {
        match self {
            Focus::Surface(s) => Some(s.clone()),
            Focus::X11(w) => w.wl_surface(),
        }
    }
}

impl IsAlive for Focus {
    fn alive(&self) -> bool {
        match self {
            Focus::Surface(s) => s.alive(),
            Focus::X11(w) => w.alive(),
        }
    }
}

impl From<PopupKind> for Focus {
    fn from(popup: PopupKind) -> Focus {
        Focus::Surface(popup.wl_surface().clone())
    }
}

impl WaylandFocus for Focus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Focus::Surface(s) => Some(Cow::Borrowed(s)),
            Focus::X11(w) => w.wl_surface().map(Cow::Owned),
        }
    }
}

/// Call the same method on whichever target this is.
macro_rules! either {
    ($self:ident, $trait:ident :: $method:ident ($($arg:expr),*)) => {
        match $self {
            Focus::Surface(s) => $trait::<State>::$method(s, $($arg),*),
            Focus::X11(w) => $trait::<State>::$method(w, $($arg),*),
        }
    };
}

impl KeyboardTarget<State> for Focus {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        either!(self, KeyboardTarget::enter(seat, data, keys, serial))
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        either!(self, KeyboardTarget::leave(seat, data, serial))
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        either!(
            self,
            KeyboardTarget::key(seat, data, key, state, serial, time)
        )
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        either!(
            self,
            KeyboardTarget::modifiers(seat, data, modifiers, serial)
        )
    }
}

impl PointerTarget<State> for Focus {
    fn enter(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        either!(self, PointerTarget::enter(seat, data, event))
    }

    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        either!(self, PointerTarget::motion(seat, data, event))
    }

    fn relative_motion(&self, seat: &Seat<State>, data: &mut State, event: &RelativeMotionEvent) {
        either!(self, PointerTarget::relative_motion(seat, data, event))
    }

    fn button(&self, seat: &Seat<State>, data: &mut State, event: &ButtonEvent) {
        either!(self, PointerTarget::button(seat, data, event))
    }

    fn axis(&self, seat: &Seat<State>, data: &mut State, frame: AxisFrame) {
        either!(self, PointerTarget::axis(seat, data, frame))
    }

    fn frame(&self, seat: &Seat<State>, data: &mut State) {
        either!(self, PointerTarget::frame(seat, data))
    }

    fn gesture_swipe_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeBeginEvent,
    ) {
        either!(self, PointerTarget::gesture_swipe_begin(seat, data, event))
    }

    fn gesture_swipe_update(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeUpdateEvent,
    ) {
        either!(self, PointerTarget::gesture_swipe_update(seat, data, event))
    }

    fn gesture_swipe_end(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeEndEvent,
    ) {
        either!(self, PointerTarget::gesture_swipe_end(seat, data, event))
    }

    fn gesture_pinch_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchBeginEvent,
    ) {
        either!(self, PointerTarget::gesture_pinch_begin(seat, data, event))
    }

    fn gesture_pinch_update(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchUpdateEvent,
    ) {
        either!(self, PointerTarget::gesture_pinch_update(seat, data, event))
    }

    fn gesture_pinch_end(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchEndEvent,
    ) {
        either!(self, PointerTarget::gesture_pinch_end(seat, data, event))
    }

    fn gesture_hold_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureHoldBeginEvent,
    ) {
        either!(self, PointerTarget::gesture_hold_begin(seat, data, event))
    }

    fn gesture_hold_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldEndEvent) {
        either!(self, PointerTarget::gesture_hold_end(seat, data, event))
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial, time: u32) {
        either!(self, PointerTarget::leave(seat, data, serial, time))
    }
}

impl State {
    fn xwm(&mut self) -> Option<&mut X11Wm> {
        self.xwm.as_mut()
    }

    /// The window in the space showing `surface`.
    fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }

    /// Put an X11 toplevel where it belongs: a dialog centered at its own
    /// size, anything else filling the window area.
    pub(super) fn place_x11(&mut self, window: &Window, surface: &X11Surface) {
        let area = self.area;
        let rect = if is_dialog(surface) {
            let size = surface.geometry().size;
            window.user_data().insert_if_missing(|| Centered);
            Rectangle::new(
                area.loc + Point::from(((area.size.w - size.w) / 2, (area.size.h - size.h) / 2)),
                size,
            )
        } else {
            window.user_data().insert_if_missing(|| Sized);
            area
        };
        let _ = surface.configure(rect);
        self.space.map_element(window.clone(), rect.loc, false);
    }
}

/// A transient window, such as a dialog, keeps its own size.
fn is_dialog(surface: &X11Surface) -> bool {
    surface.is_transient_for().is_some() || surface.is_popup()
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell
    }

    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, window: X11Surface) {
        // A window shown before its surface arrived gets its input now.
        if self.pending_focus.as_ref() == Some(&window) {
            self.pending_focus = None;
            if let Some(w) = self.x11_window(&window) {
                self.focus(&w, SERIAL_COUNTER.next_serial());
            }
        }
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm()
            .expect("X11 events come only from a running window manager")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if surface.set_mapped(true).is_err() {
            return;
        }
        let window = Window::new_x11_window(surface.clone());
        self.place_x11(&window, &surface);
        self.focus(&window, SERIAL_COUNTER.next_serial());
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        // X places menus and tooltips itself, in output coordinates.
        let loc = surface.geometry().loc;
        self.space
            .map_element(Window::new_x11_window(surface), loc, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.space.unmap_elem(&window);
        }
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
            self.focus_top();
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if self.pending_focus.as_ref() == Some(&surface) {
            self.pending_focus = None;
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        match self.x11_window(&surface) {
            // Shown windows stay where matinee put them; a dialog may
            // still change its size and is centered again.
            Some(window) => {
                if is_dialog(&surface) {
                    let mut geometry = surface.geometry();
                    geometry.size = (
                        w.map_or(geometry.size.w, |w| w as i32),
                        h.map_or(geometry.size.h, |h| h as i32),
                    )
                        .into();
                    let _ = surface.configure(geometry);
                }
                self.place_x11(&window, &surface);
            }
            // Before it is shown, a window may choose its own size.
            None => {
                let mut geometry = surface.geometry();
                if let Some(w) = w {
                    geometry.size.w = w as i32;
                }
                if let Some(h) = h {
                    geometry.size.h = h as i32;
                }
                let _ = surface.configure(geometry);
            }
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        if surface.is_override_redirect() {
            if let Some(window) = self.x11_window(&surface) {
                self.space.map_element(window, geometry.loc, false);
            }
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edge: ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}
