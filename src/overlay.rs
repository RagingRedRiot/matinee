//! What recordings draw over the app after the camera's crop, at output
//! size, so it stays crisp at any zoom: the scene's caption, a ripple where
//! a click lands, and a badge naming each key chord pressed.
//!
//! The recording keeps when each of these happened, by tick, and works out
//! what shows on each tick only when it is written, since a caption fades
//! out before its scene ends. What shows is a plain value, so ticks that
//! look the same still merge into one GIF frame.

use crate::camera::Rect;
use std::{cell::RefCell, collections::HashMap};

/// How long a caption takes to fade in, and out at its scene's end.
const CAPTION_FADE: f64 = 0.3;
/// How long a click's ripple lasts.
const RIPPLE: f64 = 0.45;
/// How long a key badge shows, and the fade at the end of that time.
const BADGE: f64 = 1.2;
const BADGE_FADE: f64 = 0.25;
/// Opacity steps in a fade. Fewer steps keep more ticks the same.
const LEVELS: u8 = 8;

const RIPPLE_COLOR: u32 = 0x3b82f6;
/// The dark backing behind caption and badge text, in either theme.
const BACKING: u32 = 0x111827;
const BACKING_ALPHA: f64 = 0.82;
const TEXT: u32 = 0xffffff;

/// What happened during a recording, by tick.
#[derive(Debug, Default)]
pub struct Events {
    /// Each caption's text, first tick, and the tick after its last; the
    /// latest has no end until the next one starts.
    captions: Vec<(String, usize, Option<usize>)>,
    clicks: Vec<usize>,
    keys: Vec<(String, usize)>,
}

/// What the overlays show on one tick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Shown {
    /// A caption by index, and its opacity out of `LEVELS`.
    caption: Option<(usize, u8)>,
    /// A key badge by index, and its opacity out of `LEVELS`.
    badge: Option<(usize, u8)>,
    /// A ripple's center in the output and how far along it is, out of
    /// `LEVELS`.
    ripple: Option<(i32, i32, u8)>,
    /// The part of the output not covered by the terminal pane: x, y,
    /// width, height.
    area: (i32, i32, i32, i32),
}

impl Shown {
    pub fn is_empty(&self) -> bool {
        self.caption.is_none() && self.badge.is_none() && self.ripple.is_none()
    }
}

impl Events {
    /// Show `text` from `tick` on, or no caption with `None`.
    pub fn caption(&mut self, tick: usize, text: Option<&str>) {
        if let Some((_, _, end @ None)) = self.captions.last_mut() {
            *end = Some(tick);
        }
        if let Some(text) = text {
            self.captions.push((text.to_string(), tick, None));
        }
    }

    pub fn click(&mut self, tick: usize) {
        self.clicks.push(tick);
    }

    pub fn key(&mut self, tick: usize, chord: &str) {
        self.keys.push((badge_label(chord), tick));
    }

    /// What shows on `tick` of `ticks` at `fps`, with the pointer where
    /// each tick put it, and the pane's area in the output for this tick.
    pub fn shown(
        &self,
        tick: usize,
        ticks: usize,
        fps: u32,
        pointer: impl Fn(usize) -> Option<(i32, i32)>,
        area: (i32, i32, i32, i32),
    ) -> Shown {
        let frames = |seconds: f64| ((seconds * fps as f64).round() as usize).max(1);
        let level = |opacity: f64| (opacity.clamp(0.0, 1.0) * LEVELS as f64).round() as u8;

        let caption = self
            .captions
            .iter()
            .enumerate()
            .find(|(_, (_, start, end))| tick >= *start && tick < end.unwrap_or(ticks))
            .and_then(|(i, (_, start, end))| {
                let fade = frames(CAPTION_FADE) as f64;
                let since = (tick - start + 1) as f64;
                let left = (end.unwrap_or(ticks) - tick) as f64;
                let opacity = level((since / fade).min(left / fade).min(1.0));
                (opacity > 0).then_some((i, opacity))
            });

        let badge = self
            .keys
            .iter()
            .enumerate()
            .rfind(|(_, (_, at))| *at <= tick)
            .and_then(|(i, (_, at))| {
                let since = tick - at;
                let shown = frames(BADGE);
                if since >= shown {
                    return None;
                }
                let fade = frames(BADGE_FADE) as f64;
                let opacity = level(((shown - since) as f64 / fade).min(1.0));
                (opacity > 0).then_some((i, opacity))
            });

        let ripple = self.clicks.iter().rfind(|&&at| at <= tick).and_then(|&at| {
            let length = frames(RIPPLE);
            let since = tick - at;
            if since >= length {
                return None;
            }
            let (x, y) = pointer(at)?;
            Some((x, y, level(since as f64 / length as f64)))
        });

        Shown {
            caption,
            badge,
            ripple,
            area,
        }
    }
}

/// The part of a `width` by `height` output that the terminal pane, at
/// `pane` on the screen, leaves for the app while the camera shows `view`.
/// A pane that fills the screen leaves the whole output, so overlays go on
/// top of the terminal.
pub fn free_area(
    view: Rect,
    pane: Option<Rect>,
    width: usize,
    height: usize,
) -> (i32, i32, i32, i32) {
    let (w, h) = (width as i32, height as i32);
    let Some(p) = pane else {
        return (0, 0, w, h);
    };
    if p.x <= 0.0 && p.y <= 0.0 {
        return (0, 0, w, h);
    }
    if p.x <= 0.0 {
        // Below the app: keep everything above the pane's top edge, and at
        // least a third of the output.
        let top = ((p.y - view.y) * height as f64 / view.h).round() as i32;
        (0, 0, w, top.clamp(h / 3, h))
    } else {
        let left = ((p.x - view.x) * width as f64 / view.w).round() as i32;
        (0, 0, left.clamp(w / 3, w), h)
    }
}

/// How a chord reads on a badge: `ctrl+k` as `Ctrl+K`, `up` as an arrow.
pub fn badge_label(chord: &str) -> String {
    chord
        .split('+')
        .map(str::trim)
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => "Ctrl".into(),
                "shift" => "Shift".into(),
                "alt" => "Alt".into(),
                "super" | "meta" => "Super".into(),
                "enter" | "return" => "Enter".into(),
                "escape" | "esc" => "Esc".into(),
                "delete" | "del" => "Delete".into(),
                "pageup" => "PgUp".into(),
                "pagedown" => "PgDn".into(),
                "up" => "↑".into(),
                "down" => "↓".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "plus" => "+".into(),
                "minus" => "-".into(),
                _ if part.chars().count() == 1 => part.to_uppercase(),
                _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => {
                    part.to_uppercase()
                }
                _ => {
                    let mut chars = lower.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
            }
        })
        .collect::<Vec<String>>()
        .join("+")
}

/// A rasterized glyph: its metrics and coverage bitmap.
type Glyph = (fontdue::Metrics, Vec<u8>);

/// Draws overlays onto output-size images.
pub struct Painter {
    font: fontdue::Font,
    width: usize,
    height: usize,
    /// Text size in output pixels.
    size: f32,
    glyphs: RefCell<HashMap<(char, u32), Glyph>>,
}

impl Painter {
    pub fn new(width: usize, height: usize) -> Painter {
        Painter {
            font: fontdue::Font::from_bytes(
                crate::terminal::BOLD,
                fontdue::FontSettings::default(),
            )
            .expect("the embedded font is valid"),
            width,
            height,
            size: (width as f32 * 0.022).clamp(14.0, 26.0),
            glyphs: RefCell::new(HashMap::new()),
        }
    }

    /// Draw what `shown` shows onto `pixels`, with the texts from `events`,
    /// and return the rectangle drawn on, if any: x, y,
    /// width, height.
    pub fn draw(
        &self,
        pixels: &mut [u32],
        shown: &Shown,
        events: &Events,
    ) -> Option<(usize, usize, usize, usize)> {
        let mut drawn = Bounds::default();
        let (ax, ay, aw, ah) = shown.area;
        let margin = (self.size * 0.6).round() as i32;

        let mut caption_box = None;
        if let Some((i, level)) = shown.caption {
            let text = &events.captions[i].0;
            let size = self.size;
            let (w, h) = self.pill_size(text, size);
            let x = ax + (aw - w) / 2;
            let y = ay + ah - margin - h;
            let opacity = level as f64 / LEVELS as f64;
            self.pill(pixels, (x, y, w, h), h as f64 / 2.0, text, size, opacity);
            drawn.add(x, y, w, h);
            caption_box = Some((x, y, w, h));
        }
        if let Some((i, level)) = shown.badge {
            let text = &events.keys[i].0;
            let size = (self.size * 0.85).round();
            let (w, h) = self.pill_size(text, size);
            let x = ax + aw - margin - w;
            let mut y = ay + ah - margin - h;
            // Above the caption rather than over it.
            if let Some((cx, cy, cw, _)) = caption_box {
                if x < cx + cw && cx < x + w {
                    y = cy - margin / 2 - h;
                }
            }
            let opacity = level as f64 / LEVELS as f64;
            self.pill(
                pixels,
                (x, y, w, h),
                size as f64 * 0.35,
                text,
                size,
                opacity,
            );
            drawn.add(x, y, w, h);
        }
        if let Some((cx, cy, step)) = shown.ripple {
            let t = step as f64 / LEVELS as f64;
            // Eases out: fast at first, slowing as it fades.
            let grown = 1.0 - (1.0 - t) * (1.0 - t);
            let radius = 4.0 + 20.0 * grown;
            let opacity = 0.9 * (1.0 - t);
            let thickness = 2.5;
            let reach = (radius + thickness).ceil() as i32 + 1;
            for y in cy - reach..=cy + reach {
                for x in cx - reach..=cx + reach {
                    let d =
                        (((x - cx) as f64 + 0.5).powi(2) + ((y - cy) as f64 + 0.5).powi(2)).sqrt();
                    let cover = (thickness / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
                    self.blend(pixels, x, y, RIPPLE_COLOR, cover * opacity);
                }
            }
            drawn.add(cx - reach, cy - reach, 2 * reach + 1, 2 * reach + 1);
        }
        drawn.clip(self.width, self.height)
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }

    fn pill_size(&self, text: &str, size: f32) -> (i32, i32) {
        let h = (size * 1.7).round() as i32;
        let w = (self.text_width(text, size) + size * 1.6).round() as i32;
        (w, h)
    }

    /// A rounded box of the backing color with `text` centered in it.
    fn pill(
        &self,
        pixels: &mut [u32],
        (x, y, w, h): (i32, i32, i32, i32),
        radius: f64,
        text: &str,
        size: f32,
        opacity: f64,
    ) {
        let (cx, cy) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        let (hw, hh) = (w as f64 / 2.0 - radius, h as f64 / 2.0 - radius);
        for py in y..y + h {
            for px in x..x + w {
                // Distance outside the rounded rectangle, negative inside.
                let dx = ((px as f64 + 0.5 - cx).abs() - hw).max(0.0);
                let dy = ((py as f64 + 0.5 - cy).abs() - hh).max(0.0);
                let d = (dx * dx + dy * dy).sqrt() - radius;
                let cover = (0.5 - d).clamp(0.0, 1.0);
                self.blend(pixels, px, py, BACKING, cover * BACKING_ALPHA * opacity);
            }
        }
        let line = self
            .font
            .horizontal_line_metrics(size)
            .expect("the font has horizontal metrics");
        let baseline =
            (y as f32 + (h as f32 - (line.ascent - line.descent)) / 2.0 + line.ascent).round();
        let mut pen = cx as f32 - self.text_width(text, size) / 2.0;
        let mut glyphs = self.glyphs.borrow_mut();
        for c in text.chars() {
            let (m, bitmap) = glyphs
                .entry((c, size.to_bits()))
                .or_insert_with(|| self.font.rasterize(c, size));
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = baseline as i32 - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let cover = bitmap[row * m.width + col] as f64 / 255.0;
                    self.blend(
                        pixels,
                        gx + col as i32,
                        gy + row as i32,
                        TEXT,
                        cover * opacity,
                    );
                }
            }
            pen += m.advance_width;
        }
    }

    fn blend(&self, pixels: &mut [u32], x: i32, y: i32, rgb: u32, alpha: f64) {
        if alpha <= 0.0 || x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let at = y as usize * self.width + x as usize;
        let under = pixels[at];
        let mix = |shift: u32| {
            let a = ((under >> shift) & 0xff) as f64;
            let b = ((rgb >> shift) & 0xff) as f64;
            ((a + (b - a) * alpha.min(1.0)).round() as u32) << shift
        };
        pixels[at] = mix(16) | mix(8) | mix(0);
    }
}

/// A growing bounding rectangle.
#[derive(Default)]
struct Bounds(Option<(i32, i32, i32, i32)>);

impl Bounds {
    fn add(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let (x1, y1) = (x + w, y + h);
        self.0 = Some(match self.0 {
            None => (x, y, x1, y1),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x1), d.max(y1)),
        });
    }

    fn clip(&self, width: usize, height: usize) -> Option<(usize, usize, usize, usize)> {
        let (x0, y0, x1, y1) = self.0?;
        let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
        let (x1, y1) = (
            (x1.max(0) as usize).min(width),
            (y1.max(0) as usize).min(height),
        );
        (x0 < x1 && y0 < y1).then(|| (x0, y0, x1 - x0, y1 - y0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_name_keys_the_way_keyboards_do() {
        assert_eq!(badge_label("ctrl+k"), "Ctrl+K");
        assert_eq!(badge_label("enter"), "Enter");
        assert_eq!(badge_label("ctrl+shift+t"), "Ctrl+Shift+T");
        assert_eq!(badge_label("alt+left"), "Alt+←");
        assert_eq!(badge_label("f5"), "F5");
        assert_eq!(badge_label("ctrl+plus"), "Ctrl++");
        assert_eq!(badge_label("backspace"), "Backspace");
    }

    #[test]
    fn a_caption_fades_in_and_out_within_its_scene() {
        let mut events = Events::default();
        events.caption(10, Some("Hello"));
        events.caption(40, None);
        let at = |tick| {
            events
                .shown(tick, 100, 10, |_| None, (0, 0, 100, 100))
                .caption
        };
        assert_eq!(at(9), None);
        // Three ticks of fade at 10 fps.
        assert!(matches!(at(10), Some((0, l)) if l > 0 && l < LEVELS));
        assert_eq!(at(20), Some((0, LEVELS)));
        assert!(matches!(at(39), Some((0, l)) if l < LEVELS));
        assert_eq!(at(40), None);
    }

    #[test]
    fn the_last_caption_lasts_until_the_next_or_the_end() {
        let mut events = Events::default();
        events.caption(0, Some("One"));
        events.caption(20, Some("Two"));
        let shown = |tick| {
            events
                .shown(tick, 50, 10, |_| None, (0, 0, 100, 100))
                .caption
                .map(|(i, _)| i)
        };
        assert_eq!(shown(19), Some(0));
        assert_eq!(shown(20), Some(1));
        assert_eq!(shown(48), Some(1));
    }

    #[test]
    fn a_ripple_starts_at_the_click_and_ends() {
        let mut events = Events::default();
        events.click(5);
        let at = |tick| {
            events
                .shown(
                    tick,
                    100,
                    10,
                    |t| (t == 5).then_some((30, 40)),
                    (0, 0, 100, 100),
                )
                .ripple
        };
        assert_eq!(at(4), None);
        assert_eq!(at(5), Some((30, 40, 0)));
        assert!(at(9).is_some());
        assert_eq!(at(10), None);
    }

    #[test]
    fn a_badge_shows_the_latest_key_then_goes() {
        let mut events = Events::default();
        events.key(0, "ctrl+k");
        events.key(3, "enter");
        let at = |tick| {
            events
                .shown(tick, 100, 10, |_| None, (0, 0, 100, 100))
                .badge
        };
        assert_eq!(at(0), Some((0, LEVELS)));
        assert_eq!(at(3), Some((1, LEVELS)));
        assert_eq!(at(15), None);
    }

    #[test]
    fn overlays_keep_clear_of_a_terminal_pane() {
        let whole = Rect::new(0.0, 0.0, 800.0, 600.0);
        // Below the app.
        let below = Some(Rect::new(0.0, 400.0, 800.0, 200.0));
        assert_eq!(free_area(whole, below, 800, 600), (0, 0, 800, 400));
        // Zoomed in on the top-left quarter, the pane is out of view.
        let zoomed = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(free_area(zoomed, below, 800, 600), (0, 0, 800, 600));
        // Beside the app.
        let right = Some(Rect::new(500.0, 0.0, 300.0, 600.0));
        assert_eq!(free_area(whole, right, 800, 600), (0, 0, 500, 600));
        // A terminal-only story.
        assert_eq!(free_area(whole, Some(whole), 800, 600), (0, 0, 800, 600));
    }

    #[test]
    fn drawing_marks_only_what_it_covers() {
        let mut events = Events::default();
        events.caption(0, Some("Hi"));
        let painter = Painter::new(400, 300);
        let shown = events.shown(10, 100, 10, |_| None, (0, 0, 400, 300));
        let mut pixels = vec![0xffffff; 400 * 300];
        let (x, y, w, h) = painter.draw(&mut pixels, &shown, &events).unwrap();
        assert!(y + h <= 300 && y > 200, "near the bottom: {y}+{h}");
        assert!(x > 100 && x + w < 300, "centered: {x}+{w}");
        let changed: Vec<usize> = (0..pixels.len())
            .filter(|&i| pixels[i] != 0xffffff)
            .collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|&i| {
            let (px, py) = (i % 400, i / 400);
            px >= x && px < x + w && py >= y && py < y + h
        }));
        // The backing is dark under the middle of the text.
        let middle = pixels[(y + h / 2) * 400 + x + 4];
        assert!(middle & 0xff < 0x80, "{middle:x}");
    }
}
