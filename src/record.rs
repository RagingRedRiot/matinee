//! Capturing a session's frames and writing them as a GIF.
//!
//! Frames are rendered at the story's scale and stored at output size, one
//! per tick of the timeline. An app image identical to the previous one is
//! not stored again. The pointer and the overlays (captions, click
//! ripples, key badges) are drawn afterwards, at output size, so they stay
//! sharp. When encoding, frames that look the same merge into one
//! longer frame, each later frame stores only the rectangle that changed,
//! with what did not change inside it transparent, and the whole clip
//! shares one palette, chosen from how long each color is on screen. Frames
//! are compressed with slight loss (see [`crate::lzw`]), and the size of
//! each part of the recording, such as a scene, is reported.

use crate::{
    camera::Rect,
    frame::Frame,
    lzw,
    overlay::{self, Events, Painter, Shown},
    palette::{self, Histogram, Nearest},
};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::HashMap,
    io::{self, Write},
    path::Path,
    rc::Rc,
};

/// The pointer, drawn with its tip at the top-left: `X` is the outline,
/// `.` the fill.
const ARROW: [&str; 19] = [
    "X",
    "XX",
    "X.X",
    "X..X",
    "X...X",
    "X....X",
    "X.....X",
    "X......X",
    "X.......X",
    "X........X",
    "X.........X",
    "X......XXXXX",
    "X...X..X",
    "X..XX..X",
    "X.X  X..X",
    "XX   X..X",
    "X     X..X",
    "      X..X",
    "       XX",
];

/// One tick of a recording: an index into its images, and the pointer
/// when it is shown.
type Tick = (usize, Option<(i32, i32)>);

pub struct Recording {
    scale: usize,
    pub width: usize,
    pub height: usize,
    /// Distinct app images, at output size, as 0xRRGGBB.
    images: Vec<Vec<u32>>,
    /// For each tick, the image shown and where the pointer was, if shown.
    ticks: Vec<Tick>,
    /// For each tick, whether the camera moved since the tick before.
    moving: Vec<bool>,
    /// For each tick, the part of the screen the camera showed.
    views: Vec<Rect>,
    /// Captions, clicks, and keys, by tick.
    events: Events,
    /// The terminal pane on the screen, which overlays keep clear of.
    pane: Option<Rect>,
    /// How far lossy compression may move a color; 0 is exact.
    lossy: u8,
    /// Where each part of the recording starts, by tick, and its name:
    /// the lead-in, each scene, and the end. Sizes are reported per part.
    segments: Vec<(usize, String)>,
    painter: Painter,
}

/// The size a GIF should stay under, per theme: a moving camera changes
/// nearly every pixel while it moves, so it gets more room.
pub const BUDGET: u64 = 2 * 1024 * 1024;
pub const CAMERA_BUDGET: u64 = 5 * 1024 * 1024;

/// How far, in any channel, lossy compression may move a color unless a
/// story says otherwise. At 6, callboard's demo is about a sixth smaller and
/// looks the same; from about 12, streaks show in flat light areas.
pub const LOSSY: u8 = 6;

/// The palette index left for transparency, after the clip's 255 colors.
const TRANSPARENT: u8 = 255;

/// What was written.
pub struct Written {
    pub bytes: u64,
    /// Frames in the GIF after merging repeats.
    pub frames: usize,
    pub seconds: f64,
    /// Whether the camera moved at any point.
    pub camera_moved: bool,
    /// The size of each part of the recording, in order.
    pub segments: Vec<SegmentSize>,
}

/// What one part of a recording, such as a scene, cost.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentSize {
    pub label: String,
    pub seconds: f64,
    pub bytes: u64,
    /// The part of `bytes` from frames where the camera was moving.
    pub camera_bytes: u64,
}

/// A writer that counts what passes through it.
struct Counting<W> {
    inner: W,
    count: Rc<Cell<u64>>,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count.set(self.count.get() + n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Recording {
    /// A recording of an output `scale` times larger than `width` by
    /// `height`.
    pub fn new(width: usize, height: usize, scale: usize) -> Recording {
        Recording {
            scale,
            width,
            height,
            images: vec![],
            ticks: vec![],
            moving: vec![],
            views: vec![],
            events: Events::default(),
            pane: None,
            lossy: LOSSY,
            segments: vec![(0, "start".into())],
            painter: Painter::new(width, height),
        }
    }

    /// Let lossy compression move a color by up to `tolerance` in any
    /// channel; 0 compresses exactly.
    pub fn set_lossy(&mut self, tolerance: u8) {
        self.lossy = tolerance;
    }

    /// Keep overlays clear of the terminal pane at `pane`, in points.
    pub fn set_pane(&mut self, pane: Option<Rect>) {
        self.pane = pane;
    }

    /// Start a new part of the recording, named `label`, from the next
    /// tick, for size reporting.
    pub fn segment(&mut self, label: &str) {
        self.segments.push((self.ticks.len(), label.into()));
    }

    /// Caption the ticks from the next one on, or stop with `None`.
    pub fn caption(&mut self, text: Option<&str>) {
        self.events.caption(self.ticks.len(), text);
    }

    /// Show a ripple where the pointer is on the next tick.
    pub fn click(&mut self) {
        self.events.click(self.ticks.len());
    }

    /// Show a badge naming `chord` from the next tick.
    pub fn key(&mut self, chord: &str) {
        self.events.key(self.ticks.len(), chord);
    }

    /// Add one tick: the rendered output, the part of the screen the
    /// camera shows (in logical points), and the pointer, already placed in
    /// the output, when it is shown.
    pub fn add(&mut self, frame: &Frame, view: Rect, pointer: Option<(f64, f64)>) {
        let whole = view == Rect::new(0.0, 0.0, self.width as f64, self.height as f64);
        let image = if whole {
            downscale(frame, self.scale, self.width, self.height)
        } else {
            resample(frame, view, self.scale, self.width, self.height)
        };
        if self.images.last() != Some(&image) {
            self.images.push(image);
        }
        let pointer = pointer.map(|(x, y)| (x.round() as i32, y.round() as i32));
        self.ticks.push((self.images.len() - 1, pointer));
        self.moving
            .push(self.views.last().is_some_and(|&v| v != view));
        self.views.push(view);
    }

    /// What the overlays show on each tick.
    fn shown(&self, fps: u32) -> Vec<Shown> {
        let pointer = |i: usize| self.ticks.get(i).and_then(|t| t.1);
        (0..self.ticks.len())
            .map(|i| {
                let area = overlay::free_area(self.views[i], self.pane, self.width, self.height);
                self.events.shown(i, self.ticks.len(), fps, pointer, area)
            })
            .collect()
    }

    /// Write the recording as a looping GIF, at `fps` ticks per second.
    pub fn write_gif(&self, path: &Path, fps: u32) -> io::Result<Written> {
        // Runs of ticks that look the same become one frame.
        let shown = self.shown(fps);
        let mut runs: Vec<((Tick, Shown), usize)> = vec![];
        // For each run, the part of the recording it starts in, and whether
        // the camera moved during it.
        let mut about: Vec<(usize, bool)> = vec![];
        let segment_of = |tick: usize| {
            self.segments
                .iter()
                .rposition(|&(start, _)| start <= tick)
                .unwrap_or(0)
        };
        let mut kept_moving = false;
        for (i, &tick) in self.ticks.iter().enumerate() {
            let moving = self.moving[i];
            let tick = (tick, shown[i].clone());
            match runs.last_mut() {
                Some((last, n)) if *last == tick => *n += 1,
                // While the camera moves, every other frame is kept: a move
                // changes nearly every pixel, and half its frames still read
                // as smooth motion.
                Some((_, n)) if moving && kept_moving => {
                    *n += 1;
                    about.last_mut().unwrap().1 = true;
                    kept_moving = false;
                    continue;
                }
                _ => {
                    runs.push((tick, 1));
                    about.push((segment_of(i), moving));
                }
            }
            kept_moving = moving;
        }
        if runs.is_empty() {
            return Err(io::Error::other("nothing was recorded"));
        }

        let colors = palette::quantize(&self.histogram(&runs), TRANSPARENT as usize);
        let nearest = Nearest::new(&colors);
        let mut palette: Vec<u8> = colors
            .iter()
            .flat_map(|c| c.to_be_bytes()[1..].to_vec())
            .collect();
        // The rest, at least the last entry, is left for transparency.
        palette.resize(3 * (TRANSPARENT as usize + 1), 0);
        let mut lookup: HashMap<u32, u8> = HashMap::new();
        let mut index = |rgb: u32| -> u8 {
            *lookup
                .entry(rgb)
                .or_insert_with(|| nearest.index(rgb) as u8)
        };

        let count = Rc::new(Cell::new(0));
        let file = Counting {
            inner: io::BufWriter::new(std::fs::File::create(path)?),
            count: count.clone(),
        };
        let mut sizes: Vec<SegmentSize> = self
            .segments
            .iter()
            .map(|(_, label)| SegmentSize {
                label: label.clone(),
                seconds: 0.0,
                bytes: 0,
                camera_bytes: 0,
            })
            .collect();
        // Charge what the encoder has written since the last frame to the
        // part and camera state of the frame just written.
        let mut charged = 0;
        let mut charge = |sizes: &mut Vec<SegmentSize>, (segment, moving): (usize, bool)| {
            let bytes = count.get() - charged;
            charged = count.get();
            sizes[segment].bytes += bytes;
            if moving {
                sizes[segment].camera_bytes += bytes;
            }
        };
        let mut encoder = gif::Encoder::new(file, self.width as u16, self.height as u16, &palette)
            .map_err(io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(io::Error::other)?;

        let lossy = lzw::Lossy {
            palette: &colors,
            tolerance: self.lossy,
            transparent: Some(TRANSPARENT),
        };
        // Whether a shown index can stay for a wanted one: the same, or
        // close enough that lossy compression would have allowed it.
        let mut same = vec![false; 256 * 256];
        for a in 0..256 {
            for b in 0..256 {
                same[a * 256 + b] = lossy.close(a as u8, b as u8);
            }
        }
        let same = |a: u8, b: u8| same[a as usize * 256 + b as usize];
        // What a viewer sees after the frames written so far.
        let mut previous: Option<Vec<u8>> = None;
        // Each frame is held until the next one differs, so time from
        // frames that turn out the same can still be added to it.
        let mut pending: Option<(gif::Frame<'static>, (usize, bool))> = None;
        let mut ticks_done = 0usize;
        let mut written_cs = 0u32;
        let mut frames = 0;
        for ((((image, pointer), shown), n), this) in runs.into_iter().zip(about) {
            let pixels = self.compose(image, pointer, &shown);
            let indexed: Vec<u8> = pixels.iter().map(|&p| index(p)).collect();
            ticks_done += n;
            // Delays are whole centiseconds; rounding the running total
            // keeps the clip's length right.
            let end_cs = (ticks_done as f64 * 100.0 / fps as f64).round() as u32;
            let delay = (end_cs - written_cs).max(1);
            written_cs += delay;
            sizes[this.0].seconds += delay as f64 / 100.0;

            let rect = match &previous {
                None => Some((0, 0, self.width, self.height)),
                Some(prev) => changed(prev, &indexed, self.width, self.height, same),
            };
            let Some((x, y, w, h)) = rect else {
                // Same pixels after quantizing: lengthen the held frame.
                if let Some((frame, _)) = &mut pending {
                    frame.delay += delay as u16;
                }
                continue;
            };
            // Inside the changed rectangle, a pixel that did not change can
            // be either transparent, letting the frame under it show, or its
            // own color again. Either looks the same, so each takes whichever
            // continues the run before it: long runs compress to little.
            let mut buffer = Vec::with_capacity(w * h);
            let mut last = TRANSPARENT;
            for row in y..y + h {
                let span = row * self.width + x..row * self.width + x + w;
                match &previous {
                    Some(prev) => {
                        for (&now, &before) in indexed[span.clone()].iter().zip(&prev[span]) {
                            last = if !same(before, now) || now == last {
                                now
                            } else {
                                TRANSPARENT
                            };
                            buffer.push(last);
                        }
                    }
                    None => buffer.extend_from_slice(&indexed[span]),
                }
            }
            let (encoded, seen) = lzw::encode(&buffer, Some(&lossy));
            // Keep what the viewer now sees: transparent pixels leave what
            // was there.
            let mut now_shown = previous.clone().unwrap_or_else(|| indexed.clone());
            for (row, line) in seen.chunks(w).enumerate() {
                let start = (y + row) * self.width + x;
                for (at, &index) in now_shown[start..start + w].iter_mut().zip(line) {
                    if index != TRANSPARENT || previous.is_none() {
                        *at = index;
                    }
                }
            }
            let frame = gif::Frame {
                left: x as u16,
                top: y as u16,
                width: w as u16,
                height: h as u16,
                delay: delay as u16,
                buffer: Cow::Owned(encoded),
                transparent: previous.is_some().then_some(TRANSPARENT),
                ..gif::Frame::default()
            };
            if let Some((held, about)) = pending.replace((frame, this)) {
                encoder
                    .write_lzw_pre_encoded_frame(&held)
                    .map_err(io::Error::other)?;
                charge(&mut sizes, about);
                frames += 1;
            }
            previous = Some(now_shown);
        }
        if let Some((held, about)) = pending {
            encoder
                .write_lzw_pre_encoded_frame(&held)
                .map_err(io::Error::other)?;
            charge(&mut sizes, about);
            frames += 1;
        }
        drop(encoder);
        // The header and palette count toward the start; parts with no time
        // in the clip are left out.
        let total = std::fs::metadata(path)?.len();
        let header = total.saturating_sub(sizes.iter().map(|s| s.bytes).sum());
        sizes[0].bytes += header;
        sizes.retain(|s| s.seconds > 0.0);
        Ok(Written {
            bytes: total,
            frames,
            seconds: written_cs as f64 / 100.0,
            camera_moved: self.moving.iter().any(|&m| m),
            segments: sizes,
        })
    }

    /// Every color the clip shows, weighted by how long it is on screen:
    /// pixels times ticks. Counts change only where an image differs from
    /// the one before, so this stays quick for long clips.
    fn histogram(&self, runs: &[((Tick, Shown), usize)]) -> Histogram {
        let mut shown_for = vec![0u64; self.images.len()];
        for &(image, _) in &self.ticks {
            shown_for[image] += 1;
        }
        let mut weights = Histogram::new();
        // For each color on screen: how many pixels, and the tick its
        // weight was last brought up to date.
        let mut on_screen: HashMap<u32, (u64, u64)> = HashMap::new();
        let mut now = 0u64;
        let mut change = |weights: &mut Histogram, color: u32, add: bool, now: u64| {
            let (count, since) = on_screen.entry(color).or_insert((0, now));
            *weights.entry(color).or_insert(0) += *count * (now - *since);
            *since = now;
            if add {
                *count += 1;
            } else {
                *count -= 1;
            }
        };
        for (i, image) in self.images.iter().enumerate() {
            match i.checked_sub(1).map(|p| &self.images[p]) {
                None => {
                    for &c in image {
                        change(&mut weights, c, true, now);
                    }
                }
                Some(before) => {
                    for (&c, &was) in image.iter().zip(before) {
                        if c != was {
                            change(&mut weights, was, false, now);
                            change(&mut weights, c, true, now);
                        }
                    }
                }
            }
            now += shown_for[i];
        }
        for (&color, &(count, since)) in &on_screen {
            *weights.entry(color).or_insert(0) += count * (now - since);
        }

        // Overlays blend into what is under them, so count each different
        // look as drawn over the image it first shows on, for as long as it
        // shows.
        let mut looks: HashMap<&Shown, (usize, u64)> = HashMap::new();
        for (((image, _), shown), n) in runs {
            if !shown.is_empty() {
                looks.entry(shown).or_insert((*image, 0)).1 += *n as u64;
            }
        }
        for (shown, (image, ticks)) in looks {
            let mut pixels = self.images[image].clone();
            if let Some((x, y, w, h)) = self.painter.draw(&mut pixels, shown, &self.events) {
                for row in y..y + h {
                    for &c in &pixels[row * self.width + x..row * self.width + x + w] {
                        *weights.entry(c).or_insert(0) += ticks;
                    }
                }
            }
        }
        // The pointer's black and white, for as long as it shows.
        let pointer = self.ticks.iter().filter(|t| t.1.is_some()).count() as u64;
        let arrow = ARROW.iter().map(|r| r.len() as u64).sum::<u64>();
        for c in [0x000000, 0xffffff] {
            *weights.entry(c).or_insert(0) += pointer * arrow / 2;
        }
        weights
    }

    /// An app image with the overlays and the pointer drawn on it.
    fn compose(&self, image: usize, pointer: Option<(i32, i32)>, shown: &Shown) -> Vec<u32> {
        let mut pixels = self.images[image].clone();
        if !shown.is_empty() {
            self.painter.draw(&mut pixels, shown, &self.events);
        }
        if let Some(at) = pointer {
            draw_pointer(&mut pixels, (self.width, self.height), at, 0);
        }
        pixels
    }
}

/// Draw the pointer with its tip at `at` onto `pixels`, `size` wide and
/// high, with `alpha` in each pixel's top byte.
pub fn draw_pointer(pixels: &mut [u32], size: (usize, usize), at: (i32, i32), alpha: u32) {
    let (width, height) = size;
    for (dy, row) in ARROW.iter().enumerate() {
        for (dx, c) in row.chars().enumerate() {
            let color = match c {
                'X' => 0x000000,
                '.' => 0xffffff,
                _ => continue,
            };
            let (x, y) = (at.0 + dx as i32, at.1 + dy as i32);
            if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                pixels[y as usize * width + x as usize] = alpha | color;
            }
        }
    }
}

/// Average each `scale` by `scale` block of `frame` into one pixel.
fn downscale(frame: &Frame, scale: usize, width: usize, height: usize) -> Vec<u32> {
    if scale == 1 {
        return frame.pixels.iter().map(|p| p & 0xffffff).collect();
    }
    let mut out = Vec::with_capacity(width * height);
    let n = (scale * scale) as u32;
    for y in 0..height {
        for x in 0..width {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for sy in 0..scale {
                let row = (y * scale + sy) * frame.width;
                for sx in 0..scale {
                    let p = frame.pixels[row + x * scale + sx];
                    r += (p >> 16) & 0xff;
                    g += (p >> 8) & 0xff;
                    b += p & 0xff;
                }
            }
            let avg = |sum: u32| (sum + n / 2) / n;
            out.push((avg(r) << 16) | (avg(g) << 8) | avg(b));
        }
    }
    out
}

/// The part of `frame` under `view` (logical points, `scale` device pixels
/// each), averaged down to `width` by `height` pixels. Each output pixel is
/// the coverage-weighted average of the device pixels under it.
fn resample(frame: &Frame, view: Rect, scale: usize, width: usize, height: usize) -> Vec<u32> {
    let s = scale as f64;
    let cols = spans(view.x * s, view.w * s / width as f64, width, frame.width);
    let rows = spans(view.y * s, view.h * s / height as f64, height, frame.height);
    let mut out = Vec::with_capacity(width * height);
    for row in &rows {
        for col in &cols {
            let (mut r, mut g, mut b, mut total) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for &(sy, wy) in row {
                let line = sy * frame.width;
                for &(sx, wx) in col {
                    let p = frame.pixels[line + sx];
                    let w = wx * wy;
                    r += ((p >> 16) & 0xff) as f64 * w;
                    g += ((p >> 8) & 0xff) as f64 * w;
                    b += (p & 0xff) as f64 * w;
                    total += w;
                }
            }
            let c = |v: f64| (v / total.max(f64::EPSILON)).round().clamp(0.0, 255.0) as u32;
            out.push((c(r) << 16) | (c(g) << 8) | c(b));
        }
    }
    out
}

/// For each of `count` output pixels starting at device coordinate `start`
/// and `step` device pixels wide, the device pixels it covers and how much
/// of each.
fn spans(start: f64, step: f64, count: usize, limit: usize) -> Vec<Vec<(usize, f64)>> {
    (0..count)
        .map(|i| {
            let (a, b) = (start + i as f64 * step, start + (i + 1) as f64 * step);
            let first = (a.floor().max(0.0) as usize).min(limit - 1);
            let last = (b.ceil() as usize).clamp(first + 1, limit);
            (first..last)
                .map(|p| {
                    let cover = (b.min(p as f64 + 1.0) - a.max(p as f64)).max(0.0);
                    (p, cover)
                })
                .filter(|&(_, w)| w > 0.0)
                .collect()
        })
        .collect()
}

/// The smallest rectangle holding every pixel of `b` that `a` cannot stand
/// for, if any.
fn changed(
    a: &[u8],
    b: &[u8],
    width: usize,
    height: usize,
    same: impl Fn(u8, u8) -> bool,
) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            if !same(a[row + x], b[row + x]) {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    (x0 <= x1).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole of the 20-point test screen.
    fn all() -> Rect {
        Rect::new(0.0, 0.0, 20.0, 20.0)
    }

    fn frame(width: usize, height: usize, color: u32) -> Frame {
        Frame {
            width,
            height,
            pixels: vec![color; width * height],
        }
    }

    #[test]
    fn resampling_the_whole_screen_matches_the_block_average() {
        let mut f = frame(8, 4, 0xff000000);
        for (i, p) in f.pixels.iter_mut().enumerate() {
            *p = 0xff000000 | ((i as u32 * 37) % 256) << 8;
        }
        let whole = Rect::new(0.0, 0.0, 4.0, 2.0);
        assert_eq!(resample(&f, whole, 2, 4, 2), downscale(&f, 2, 4, 2));
    }

    #[test]
    fn resampling_a_zoomed_view_shows_only_that_part() {
        // Left half white, right half black, at scale 2 on a 4x2 screen.
        let mut f = frame(8, 4, 0xff000000);
        for y in 0..4 {
            for x in 0..4 {
                f.pixels[y * 8 + x] = 0xffffffff;
            }
        }
        let left = resample(&f, Rect::new(0.0, 0.0, 2.0, 1.0), 2, 4, 2);
        assert!(left.iter().all(|&p| p == 0xffffff), "{left:x?}");
        let middle = resample(&f, Rect::new(1.0, 0.0, 2.0, 1.0), 2, 4, 2);
        assert_eq!(middle[0], 0xffffff);
        assert_eq!(middle[3], 0x000000);
    }

    #[test]
    fn downscale_averages_blocks() {
        let mut f = frame(4, 2, 0xff000000);
        f.pixels[0] = 0xffffffff;
        f.pixels[1] = 0xffffffff;
        let small = downscale(&f, 2, 2, 1);
        assert_eq!(small, vec![0x808080, 0x000000]);
    }

    #[test]
    fn changed_rectangle() {
        let a = vec![0u8; 16];
        let mut b = a.clone();
        assert_eq!(changed(&a, &b, 4, 4, |x, y| x == y), None);
        b[5] = 1;
        b[10] = 1;
        assert_eq!(changed(&a, &b, 4, 4, |x, y| x == y), Some((1, 1, 2, 2)));
    }

    #[test]
    fn repeated_frames_merge_and_the_length_is_kept() {
        let dir = std::env::temp_dir().join(format!("matinee-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.gif");
        let mut rec = Recording::new(20, 20, 1);
        // 15 ticks of one image, then 15 of another: one second at 15 fps
        // is 100 centiseconds each.
        for _ in 0..15 {
            rec.add(&frame(20, 20, 0xff102030), all(), None);
        }
        for _ in 0..15 {
            rec.add(&frame(20, 20, 0xffa0b0c0), all(), None);
        }
        let written = rec.write_gif(&path, 15).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written.frames, 2);
        assert_eq!(written.seconds, 2.0);
    }

    /// The total of the delays actually stored in a GIF, in seconds.
    fn stored_seconds(path: &Path) -> f64 {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options
            .read_info(std::fs::File::open(path).unwrap())
            .unwrap();
        let mut cs = 0u32;
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            cs += frame.delay as u32;
        }
        cs as f64 / 100.0
    }

    #[test]
    fn frames_that_encode_the_same_keep_their_time() {
        let dir = std::env::temp_dir().join(format!("matinee-gifq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.gif");
        let mut rec = Recording::new(20, 20, 1);
        // A second each: the pointer moves but stays off screen, so the
        // first two seconds look the same; then the image changes.
        for (color, pointer) in [
            (0xff102030, Some((-50.0, -50.0))),
            (0xff102030, Some((-60.0, -60.0))),
            (0xffa0b0c0, Some((-60.0, -60.0))),
        ] {
            for _ in 0..15 {
                rec.add(&frame(20, 20, color), all(), pointer);
            }
        }
        let written = rec.write_gif(&path, 15).unwrap();
        let stored = stored_seconds(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written.frames, 2);
        assert_eq!(written.seconds, 3.0);
        assert_eq!(stored, 3.0);
    }

    #[test]
    fn unchanged_pixels_in_a_changed_rectangle_are_transparent() {
        let dir = std::env::temp_dir().join(format!("matinee-gift-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.gif");
        let (dark, light) = (0xff203040u32, 0xffffffffu32);
        let mut rec = Recording::new(20, 20, 1);
        let mut images = vec![frame(20, 20, dark)];
        // Two opposite corners change, so the rectangle covers everything
        // between them.
        let mut next = frame(20, 20, dark);
        next.pixels[0] = light;
        next.pixels[20 * 20 - 1] = light;
        images.push(next);
        for image in &images {
            for _ in 0..15 {
                rec.add(image, all(), None);
            }
        }
        rec.write_gif(&path, 15).unwrap();

        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options
            .read_info(std::fs::File::open(&path).unwrap())
            .unwrap();
        let palette = decoder.global_palette().unwrap().to_vec();
        let mut canvas = vec![0u8; 20 * 20];
        let mut shown = vec![];
        while let Some(f) = decoder.read_next_frame().unwrap() {
            for row in 0..f.height as usize {
                for col in 0..f.width as usize {
                    let index = f.buffer[row * f.width as usize + col];
                    if Some(index) != f.transparent {
                        canvas[(f.top as usize + row) * 20 + f.left as usize + col] = index;
                    }
                }
            }
            shown.push((canvas.clone(), f.transparent));
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[1].1, Some(TRANSPARENT));
        let rgb = |i: u8| {
            let p = &palette[i as usize * 3..i as usize * 3 + 3];
            (p[0] as i32, p[1] as i32, p[2] as i32)
        };
        let near = |i: u8, color: u32| {
            let (r, g, b) = rgb(i);
            let [_, er, eg, eb] = color.to_be_bytes();
            (r - er as i32).abs() < 12 && (g - eg as i32).abs() < 12 && (b - eb as i32).abs() < 12
        };
        for (at, &expected) in images[1].pixels.iter().enumerate() {
            assert!(near(shown[1].0[at], expected), "pixel {at}");
        }
    }

    #[test]
    fn sizes_are_reported_per_part_and_add_up() {
        let dir = std::env::temp_dir().join(format!("matinee-gifs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.gif");
        let mut rec = Recording::new(20, 20, 1);
        rec.add(&frame(20, 20, 0xff102030), all(), None);
        rec.segment("scene 1");
        for _ in 0..15 {
            rec.add(&frame(20, 20, 0xffa0b0c0), all(), None);
        }
        // A scene with no frames of its own is left out.
        rec.segment("scene 2");
        rec.segment("end");
        for _ in 0..15 {
            rec.add(
                &frame(20, 20, 0xffffffff),
                Rect::new(0.0, 0.0, 10.0, 10.0),
                None,
            );
        }
        let written = rec.write_gif(&path, 15).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let labels: Vec<&str> = written.segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["start", "scene 1", "end"]);
        assert_eq!(
            written.segments.iter().map(|s| s.bytes).sum::<u64>(),
            written.bytes
        );
        let seconds: f64 = written.segments.iter().map(|s| s.seconds).sum();
        assert!((seconds - written.seconds).abs() < 1e-9);
        assert_eq!(written.segments[1].seconds, 1.0);
        assert_eq!(written.segments[1].camera_bytes, 0);
        assert!(written.segments[2].camera_bytes > 0, "the view changed");
    }
}
