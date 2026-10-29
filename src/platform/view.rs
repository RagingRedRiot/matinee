//! Drawing the viewport window's picture of a running story, the same on
//! every platform: the whole session scaled to fit, the screen outside the
//! camera's view dimmed, the view outlined, and the pointer.

use crate::camera::Rect;

/// The camera outline: the same blue as click ripples.
pub const OUTLINE: u32 = 0xff3b82f6;
const OUTLINE_WIDTH: usize = 2;
/// How bright the screen outside the camera's view stays, out of 256.
pub const DIM: u32 = 110;
/// Around the session when the window's shape differs from it.
pub const BACKGROUND: u32 = 0xff111827;
/// The largest window the viewer asks for; the desktop may resize it.
pub const MAX_WINDOW: (u32, u32) = (1280, 800);

/// The viewer's copy of a frame.
pub struct Snapshot {
    pub seq: u64,
    pub width: usize,
    pub height: usize,
    /// The camera's view in the frame's pixels.
    pub view: Rect,
    /// Where the pointer is in the frame's pixels, while it is shown.
    pub pointer: Option<(f64, f64)>,
    pub pixels: Vec<u32>,
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

/// The largest size with `size`'s shape that fits in `bounds`, never
/// larger than `size` itself.
pub fn fit(size: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
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
pub fn compose(snap: &Snapshot, dst: &mut [u32], (dw, dh): (usize, usize)) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
