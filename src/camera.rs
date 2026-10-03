//! The virtual camera: a rectangle over the screen that each recorded frame
//! shows, scaled to the output size.
//!
//! The screen is rendered at the story's scale, so a camera zoomed in up to
//! that scale crops real detail rather than enlarging pixels. Each move
//! eases in and out over a fixed number of frames, so the same story always
//! makes the same camera path.

/// A rectangle in logical points: x, y, width, height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Rect) -> Rect {
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let x1 = (self.x + self.w).max(other.x + other.w);
        let y1 = (self.y + self.h).max(other.y + other.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    fn center(self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether `other` lies inside the middle `fraction` of this rectangle.
    fn holds(self, other: Rect, fraction: f64) -> bool {
        let mx = self.w * (1.0 - fraction) / 2.0;
        let my = self.h * (1.0 - fraction) / 2.0;
        other.x >= self.x + mx
            && other.y >= self.y + my
            && other.x + other.w <= self.x + self.w - mx
            && other.y + other.h <= self.y + self.h - my
    }
}

/// How much of the view a framed target should fill, at most, along its
/// larger dimension: the rest is context.
const FILL: f64 = 0.4;
/// The middle part of the view a target may sit in without the camera
/// moving.
const COMFORT: f64 = 0.9;
/// How wide a recording is usually shown, in pixels: about the width of a
/// README's column. Text drawn at its normal size stays readable when the
/// output is no wider than this.
pub const VIEWED_WIDTH: f64 = 800.0;
/// A view zoomed at least this close to the wanted zoom is readable
/// enough to stay put.
const ZOOM_SLACK: f64 = 1.25;

#[derive(Debug, Clone)]
pub struct Camera {
    screen: (f64, f64),
    max_zoom: f64,
    /// The zoom that shows text at its normal size when the recording is
    /// viewed `VIEWED_WIDTH` wide. Auto framing zooms no further.
    reading_zoom: f64,
    /// Frames each move takes.
    frames: u32,
    fixed: bool,
    current: Rect,
    /// Where the current move started, and how far into it the view is.
    from: Rect,
    progress: u32,
    target: Rect,
}

impl Camera {
    /// A camera over a screen of `width` by `height` points, zooming in at
    /// most `max_zoom` times, each move taking `frames` frames.
    pub fn new(width: f64, height: f64, max_zoom: f64, frames: u32, fixed: bool) -> Camera {
        let all = Rect::new(0.0, 0.0, width, height);
        Camera {
            screen: (width, height),
            max_zoom: max_zoom.max(1.0),
            reading_zoom: (width / VIEWED_WIDTH).max(1.0),
            frames: frames.max(1),
            fixed,
            current: all,
            from: all,
            progress: frames.max(1),
            target: all,
        }
    }

    /// What the view shows now.
    pub fn view(&self) -> Rect {
        self.current
    }

    /// How far the view is magnified.
    pub fn zoom(&self) -> f64 {
        self.screen.0 / self.current.w
    }

    /// Whether the view has reached its target.
    pub fn settled(&self) -> bool {
        self.progress >= self.frames
    }

    /// Ease back out to the whole screen. Returns whether the camera will
    /// move.
    pub fn overview(&mut self) -> bool {
        self.aim(Rect::new(0.0, 0.0, self.screen.0, self.screen.1))
    }

    /// Start a move toward `next`, from wherever the view is now.
    fn aim(&mut self, next: Rect) -> bool {
        if next == self.target {
            return false;
        }
        self.from = self.current;
        self.progress = 0;
        self.target = next;
        true
    }

    /// Frame `focus` with context around it, unless the view already holds
    /// it comfortably and close enough to read: every move costs a GIF
    /// nearly every pixel, so the camera stays put whenever it can. `zoom`
    /// overrides the zoom. Returns whether the camera will move.
    pub fn frame(&mut self, focus: Rect, zoom: Option<f64>) -> bool {
        if self.fixed {
            return false;
        }
        let wanted = zoom.unwrap_or_else(|| {
            // Enough to read the focus at the usual viewing width, with it
            // filling at most FILL of the view.
            (FILL * self.screen.0 / focus.w.max(1.0))
                .min(FILL * self.screen.1 / focus.h.max(1.0))
                .min(self.reading_zoom)
        });
        let wanted = wanted.clamp(1.0, self.max_zoom);
        let current = self.zoom();
        if zoom.is_none() && self.target.holds(focus, COMFORT) && wanted / current < ZOOM_SLACK {
            return false;
        }
        let (w, h) = (self.screen.0 / wanted, self.screen.1 / wanted);
        let (cx, cy) = focus.center();
        // Centered on the focus, but never past the screen's edges.
        let x = (cx - w / 2.0).clamp(0.0, self.screen.0 - w);
        let y = (cy - h / 2.0).clamp(0.0, self.screen.1 - h);
        self.aim(Rect::new(x, y, w, h))
    }

    /// Advance one frame along the current move.
    pub fn step(&mut self) {
        if self.settled() {
            self.current = self.target;
            return;
        }
        self.progress += 1;
        let t = self.progress as f64 / self.frames as f64;
        // Smoothstep: slow to start, slow to stop.
        let e = t * t * (3.0 - 2.0 * t);
        let (a, b) = (self.from, self.target);
        let mix = |p: f64, q: f64| p + (q - p) * e;
        self.current = Rect::new(mix(a.x, b.x), mix(a.y, b.y), mix(a.w, b.w), mix(a.h, b.h));
    }

    /// Where a point on the screen appears in an output `size` wide and
    /// high, under the current view.
    pub fn to_output(&self, (x, y): (f64, f64), size: (f64, f64)) -> (f64, f64) {
        let v = self.current;
        ((x - v.x) * size.0 / v.w, (y - v.y) * size.1 / v.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1600-point screen: it needs 2x to read at the usual width.
    fn camera() -> Camera {
        Camera::new(1600.0, 1000.0, 2.0, 9, false)
    }

    fn settle(c: &mut Camera) -> usize {
        let mut frames = 0;
        while !c.settled() {
            c.step();
            frames += 1;
            assert!(frames < 200, "never settled");
        }
        frames
    }

    #[test]
    fn a_small_target_is_framed_at_most_at_max_zoom_and_kept_on_screen() {
        let mut c = camera();
        assert!(c.frame(Rect::new(20.0, 10.0, 40.0, 20.0), None));
        let frames = settle(&mut c);
        assert_eq!(frames, 9);
        assert_eq!(c.view(), Rect::new(0.0, 0.0, 800.0, 500.0));
        assert_eq!(c.zoom(), 2.0);
    }

    #[test]
    fn a_screen_no_wider_than_it_is_viewed_needs_no_zoom() {
        let mut c = Camera::new(800.0, 600.0, 2.0, 9, false);
        assert!(!c.frame(Rect::new(20.0, 10.0, 40.0, 20.0), None));
        // An explicit zoom still applies.
        assert!(c.frame(Rect::new(20.0, 10.0, 40.0, 20.0), Some(1.5)));
    }

    #[test]
    fn a_target_already_in_view_does_not_move_the_camera() {
        let mut c = camera();
        c.frame(Rect::new(500.0, 300.0, 40.0, 20.0), None);
        settle(&mut c);
        assert!(!c.frame(Rect::new(560.0, 360.0, 40.0, 20.0), None));
    }

    #[test]
    fn a_closer_view_that_holds_the_next_target_stays() {
        let mut c = camera();
        c.frame(Rect::new(400.0, 300.0, 40.0, 20.0), None);
        settle(&mut c);
        // A large neighbor would want less zoom, but it is in view.
        assert!(!c.frame(Rect::new(250.0, 200.0, 300.0, 120.0), None));
    }

    #[test]
    fn a_large_focus_stays_at_the_overview() {
        let mut c = camera();
        assert!(!c.frame(Rect::new(100.0, 100.0, 1300.0, 700.0), None));
        assert_eq!(c.zoom(), 1.0);
    }

    #[test]
    fn the_same_moves_make_the_same_path() {
        let path = || {
            let mut c = camera();
            c.frame(Rect::new(700.0, 500.0, 50.0, 30.0), None);
            (0..20)
                .map(|_| {
                    c.step();
                    c.view()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(path(), path());
    }

    #[test]
    fn a_fixed_camera_never_moves() {
        let mut c = Camera::new(1200.0, 800.0, 2.0, 9, true);
        assert!(!c.frame(Rect::new(20.0, 10.0, 40.0, 20.0), None));
        c.step();
        assert_eq!(c.zoom(), 1.0);
    }

    #[test]
    fn points_map_into_the_output() {
        let mut c = camera();
        c.frame(Rect::new(20.0, 10.0, 40.0, 20.0), None);
        settle(&mut c);
        // The view is the top-left 800x500, shown at 1600x1000.
        assert_eq!(
            c.to_output((400.0, 250.0), (1600.0, 1000.0)),
            (800.0, 500.0)
        );
    }
}
