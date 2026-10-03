//! Capturing a session's frames and writing them as a GIF.
//!
//! Frames are rendered at the story's scale and stored at output size, one
//! per tick of the timeline. An app image identical to the previous one is
//! not stored again. The pointer is drawn afterwards, at output size, so it
//! stays sharp. When encoding, frames that look the same merge into one
//! longer frame, each later frame stores only the rectangle that changed,
//! and the whole clip shares one 256-color palette.

use crate::{camera::Rect, compositor::Frame};
use std::{borrow::Cow, collections::HashMap, io, path::Path};

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
    last_view: Option<Rect>,
}

/// The size a GIF should stay under, per theme: a moving camera changes
/// nearly every pixel while it moves, so it gets more room.
pub const BUDGET: u64 = 2 * 1024 * 1024;
pub const CAMERA_BUDGET: u64 = 5 * 1024 * 1024;

/// What was written.
pub struct Written {
    pub bytes: u64,
    /// Frames in the GIF after merging repeats.
    pub frames: usize,
    pub seconds: f64,
    /// Whether the camera moved at any point.
    pub camera_moved: bool,
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
            last_view: None,
        }
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
        self.moving.push(self.last_view.is_some_and(|v| v != view));
        self.last_view = Some(view);
    }

    /// Write the recording as a looping GIF, at `fps` ticks per second.
    pub fn write_gif(&self, path: &Path, fps: u32) -> io::Result<Written> {
        // Runs of ticks that look the same become one frame.
        let mut runs: Vec<(Tick, usize)> = vec![];
        let mut kept_moving = false;
        for (i, &tick) in self.ticks.iter().enumerate() {
            let moving = self.moving[i];
            match runs.last_mut() {
                Some((last, n)) if *last == tick => *n += 1,
                // While the camera moves, every other frame is kept: a move
                // changes nearly every pixel, and half its frames still read
                // as smooth motion.
                Some((_, n)) if moving && kept_moving => {
                    *n += 1;
                    kept_moving = false;
                    continue;
                }
                _ => runs.push((tick, 1)),
            }
            kept_moving = moving;
        }
        if runs.is_empty() {
            return Err(io::Error::other("nothing was recorded"));
        }

        let quant = self.palette();
        let palette = quant.color_map_rgb();
        let mut lookup: HashMap<u32, u8> = HashMap::new();
        let mut index = |rgb: u32| -> u8 {
            *lookup.entry(rgb).or_insert_with(|| {
                let [_, r, g, b] = rgb.to_be_bytes();
                quant.index_of(&[r, g, b, 255]) as u8
            })
        };

        let file = io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = gif::Encoder::new(file, self.width as u16, self.height as u16, &palette)
            .map_err(io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(io::Error::other)?;

        let mut previous: Option<Vec<u8>> = None;
        // Each frame is held until the next one differs, so time from
        // frames that turn out the same can still be added to it.
        let mut pending: Option<gif::Frame<'static>> = None;
        let mut ticks_done = 0usize;
        let mut written_cs = 0u32;
        let mut frames = 0;
        for ((image, pointer), n) in runs {
            let pixels = self.compose(image, pointer);
            let indexed: Vec<u8> = pixels.iter().map(|&p| index(p)).collect();
            ticks_done += n;
            // Delays are whole centiseconds; rounding the running total
            // keeps the clip's length right.
            let end_cs = (ticks_done as f64 * 100.0 / fps as f64).round() as u32;
            let delay = (end_cs - written_cs).max(1);
            written_cs += delay;

            let rect = match &previous {
                None => Some((0, 0, self.width, self.height)),
                Some(prev) => changed(prev, &indexed, self.width, self.height),
            };
            let Some((x, y, w, h)) = rect else {
                // Same pixels after quantizing: lengthen the held frame.
                if let Some(frame) = &mut pending {
                    frame.delay += delay as u16;
                }
                continue;
            };
            let mut buffer = Vec::with_capacity(w * h);
            for row in y..y + h {
                buffer.extend_from_slice(&indexed[row * self.width + x..row * self.width + x + w]);
            }
            let frame = gif::Frame {
                left: x as u16,
                top: y as u16,
                width: w as u16,
                height: h as u16,
                delay: delay as u16,
                buffer: Cow::Owned(buffer),
                ..gif::Frame::default()
            };
            if let Some(held) = pending.replace(frame) {
                encoder.write_frame(&held).map_err(io::Error::other)?;
                frames += 1;
            }
            previous = Some(indexed);
        }
        if let Some(held) = pending {
            encoder.write_frame(&held).map_err(io::Error::other)?;
            frames += 1;
        }
        drop(encoder);
        Ok(Written {
            bytes: std::fs::metadata(path)?.len(),
            frames,
            seconds: written_cs as f64 / 100.0,
            camera_moved: self.moving.iter().any(|&m| m),
        })
    }

    /// One palette for the whole clip, learned from every app image and
    /// the pointer's colors.
    fn palette(&self) -> color_quant::NeuQuant {
        let mut sample: Vec<u8> = Vec::new();
        for image in &self.images {
            for &p in image {
                let [_, r, g, b] = p.to_be_bytes();
                sample.extend_from_slice(&[r, g, b, 255]);
            }
        }
        // Make sure pure black and white survive for the pointer.
        for _ in 0..64 {
            sample.extend_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]);
        }
        color_quant::NeuQuant::new(10, 256, &sample)
    }

    /// An app image with the pointer drawn on it.
    fn compose(&self, image: usize, pointer: Option<(i32, i32)>) -> Vec<u32> {
        let mut pixels = self.images[image].clone();
        let Some((px, py)) = pointer else {
            return pixels;
        };
        for (dy, row) in ARROW.iter().enumerate() {
            for (dx, c) in row.chars().enumerate() {
                let color = match c {
                    'X' => 0x000000,
                    '.' => 0xffffff,
                    _ => continue,
                };
                let (x, y) = (px + dx as i32, py + dy as i32);
                if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
                    pixels[y as usize * self.width + x as usize] = color;
                }
            }
        }
        pixels
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

/// The smallest rectangle holding every pixel that differs, if any.
fn changed(
    a: &[u8],
    b: &[u8],
    width: usize,
    height: usize,
) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            if a[row + x] != b[row + x] {
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
        assert_eq!(changed(&a, &b, 4, 4), None);
        b[5] = 1;
        b[10] = 1;
        assert_eq!(changed(&a, &b, 4, 4), Some((1, 1, 2, 2)));
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
}
