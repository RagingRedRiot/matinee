//! What recordings draw over the app after the camera's crop, at output
//! size, so it stays crisp at any zoom: the scene's caption, highlights
//! that mark part of the screen, a ripple where a click lands, and a badge
//! naming each key chord pressed.
//!
//! The recording keeps when each of these happened, by tick, and works out
//! what shows on each tick only when it is written, since a caption fades
//! out before its scene ends. What shows is a plain value, so ticks that
//! look the same still merge into one GIF frame.

use crate::{
    camera::Rect,
    story::{CaptionPosition, HighlightStyle, Theme},
};
use std::{cell::RefCell, collections::HashMap};

/// How long a caption takes to fade in, and out at its scene's end.
const CAPTION_FADE: f64 = 0.3;
/// How long a highlight takes to fade in.
const HIGHLIGHT_FADE: f64 = 0.25;
/// How long a click's ripple lasts.
const RIPPLE: f64 = 0.45;
/// How long a key badge shows, and the fade at the end of that time.
const BADGE: f64 = 1.2;
const BADGE_FADE: f64 = 0.25;
/// Opacity steps in a fade. Fewer steps keep more ticks the same.
const LEVELS: u8 = 8;

const RIPPLE_COLOR: u32 = 0x3b82f6;
/// Highlights, their numbers, and their labels.
const ACCENT: u32 = 0xf59e0b;
/// Text on the accent color.
const ON_ACCENT: u32 = 0x111827;
/// How much a spotlight darkens the rest of the frame.
const DIM: f64 = 0.55;

/// The colors of captions and badges in one theme.
#[derive(Debug, Clone, Copy)]
struct Palette {
    /// A caption band's background, and the line between it and the frame.
    band: u32,
    band_rule: u32,
    /// Text on the band, or on a caption over the frame.
    text: u32,
    /// The backing of a caption or badge over the frame, its opacity, and
    /// its border.
    backing: u32,
    backing_alpha: f64,
    border: u32,
}

impl Palette {
    fn of(theme: Theme) -> Palette {
        match theme {
            Theme::Dark => Palette {
                band: 0x0b1120,
                band_rule: 0x1e293b,
                text: 0xf8fafc,
                backing: 0x1e293b,
                backing_alpha: 0.95,
                border: 0x475569,
            },
            Theme::Light => Palette {
                band: 0xf8fafc,
                band_rule: 0xe2e8f0,
                text: 0x0f172a,
                backing: 0xffffff,
                backing_alpha: 0.96,
                border: 0xcbd5e1,
            },
        }
    }
}

/// Where a caption is drawn on a tick: over the frame, or in a band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Bottom,
    Top,
    Below,
    Above,
}

/// A highlight as recorded: what it marks on the screen, in points, how
/// it looks, and its first tick and the tick after its last.
#[derive(Debug)]
struct Mark {
    rect: Rect,
    style: HighlightStyle,
    label: Option<String>,
    number: Option<u32>,
    start: usize,
    end: Option<usize>,
}

/// What happened during a recording, by tick.
#[derive(Debug, Default)]
pub struct Events {
    /// Each caption's text and place, first tick, and the tick after its
    /// last; the latest has no end until the next one starts.
    captions: Vec<(String, CaptionPosition, usize, Option<usize>)>,
    marks: Vec<Mark>,
    clicks: Vec<usize>,
    keys: Vec<(String, usize)>,
}

/// A highlight showing on a tick: its index, its rectangle in the output,
/// and its opacity out of `LEVELS`.
type ShownMark = (usize, (i32, i32, i32, i32), u8);

/// What the overlays show on one tick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Shown {
    /// A caption by index, its opacity out of `LEVELS`, and where it goes.
    caption: Option<(usize, u8, Place)>,
    /// A key badge by index, and its opacity out of `LEVELS`.
    badge: Option<(usize, u8)>,
    /// A ripple's center in the output and how far along it is, out of
    /// `LEVELS`.
    ripple: Option<(i32, i32, u8)>,
    /// Each highlight showing.
    marks: Vec<ShownMark>,
    /// The part of the output not covered by the terminal pane: x, y,
    /// width, height.
    area: (i32, i32, i32, i32),
    /// The frame in the output, without caption bands.
    frame: (i32, i32, i32, i32),
}

impl Shown {
    /// The same, with the caption and highlights fully in, as a still
    /// picture shows them even at a moment they were fading.
    pub fn settled(&self) -> Shown {
        let mut shown = self.clone();
        if let Some((_, level, _)) = &mut shown.caption {
            *level = LEVELS;
        }
        for (_, _, level) in &mut shown.marks {
            *level = LEVELS;
        }
        shown
    }

    pub fn is_empty(&self) -> bool {
        self.caption.is_none()
            && self.badge.is_none()
            && self.ripple.is_none()
            && self.marks.is_empty()
    }
}

/// Where a tick's overlays go in the output: the frame without bands, the
/// part of it the terminal pane leaves, and how a rectangle on the screen,
/// in points, lands in the output on this tick.
pub struct Layout<'a> {
    pub frame: (i32, i32, i32, i32),
    pub area: (i32, i32, i32, i32),
    pub map: &'a dyn Fn(Rect) -> (i32, i32, i32, i32),
}

impl Events {
    /// Show `text` at `position` from `tick` on, or no caption with
    /// `None`.
    pub fn caption(&mut self, tick: usize, text: Option<&str>, position: CaptionPosition) {
        if let Some((_, _, _, end @ None)) = self.captions.last_mut() {
            *end = Some(tick);
        }
        if let Some(text) = text {
            self.captions.push((text.to_string(), position, tick, None));
        }
    }

    /// Mark `rect` on the screen, in points, from `tick` until the
    /// highlights are cleared.
    pub fn highlight(
        &mut self,
        tick: usize,
        rect: Rect,
        style: HighlightStyle,
        label: Option<&str>,
        number: Option<u32>,
    ) {
        self.marks.push(Mark {
            rect,
            style,
            label: label.map(str::to_string),
            number,
            start: tick,
            end: None,
        });
    }

    /// Take down every highlight showing, from `tick`.
    pub fn clear_highlights(&mut self, tick: usize) {
        for mark in self.marks.iter_mut().filter(|m| m.end.is_none()) {
            mark.end = Some(tick);
        }
    }

    pub fn click(&mut self, tick: usize) {
        self.clicks.push(tick);
    }

    pub fn key(&mut self, tick: usize, chord: &str) {
        self.keys.push((badge_label(chord), tick));
    }

    /// What shows on `tick` of `ticks` at `fps`, with the pointer where
    /// each tick put it, laid out by `layout`.
    pub fn shown(
        &self,
        tick: usize,
        ticks: usize,
        fps: u32,
        pointer: impl Fn(usize) -> Option<(i32, i32)>,
        layout: &Layout,
    ) -> Shown {
        let frames = |seconds: f64| ((seconds * fps as f64).round() as usize).max(1);
        let level = |opacity: f64| (opacity.clamp(0.0, 1.0) * LEVELS as f64).round() as u8;

        let marks: Vec<ShownMark> = self
            .marks
            .iter()
            .enumerate()
            .filter(|(_, m)| tick >= m.start && tick < m.end.unwrap_or(ticks))
            .map(|(i, m)| {
                let since = (tick - m.start + 1) as f64;
                let opacity = level(since / frames(HIGHLIGHT_FADE) as f64);
                (i, (layout.map)(m.rect), opacity)
            })
            .collect();

        let caption = self
            .captions
            .iter()
            .enumerate()
            .find(|(_, (_, _, start, end))| tick >= *start && tick < end.unwrap_or(ticks))
            .and_then(|(i, (_, position, start, end))| {
                let fade = frames(CAPTION_FADE) as f64;
                let since = (tick - start + 1) as f64;
                let left = (end.unwrap_or(ticks) - tick) as f64;
                let opacity = level((since / fade).min(left / fade).min(1.0));
                let place = match position {
                    CaptionPosition::Bottom => Place::Bottom,
                    CaptionPosition::Top => Place::Top,
                    CaptionPosition::Below => Place::Below,
                    CaptionPosition::Above => Place::Above,
                    // Away from what the viewer is looking at: the newest
                    // highlight, or else the pointer.
                    CaptionPosition::Auto => {
                        let subject = marks
                            .last()
                            .map(|(_, (_, y, _, h), _)| y + h / 2)
                            .or_else(|| pointer(tick).map(|(_, y)| y));
                        let (_, ay, _, ah) = layout.area;
                        match subject {
                            Some(y) if y > ay + ah / 2 => Place::Top,
                            _ => Place::Bottom,
                        }
                    }
                };
                (opacity > 0).then_some((i, opacity, place))
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
            marks,
            area: layout.area,
            frame: layout.frame,
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

/// The font overlays are set in.
const FONT: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");

/// A rasterized glyph: its metrics and coverage bitmap.
type Glyph = (fontdue::Metrics, Vec<u8>);

/// A box to draw: where, its corner radius, its fill and that fill's
/// opacity, and an optional border.
struct Plate {
    rect: (i32, i32, i32, i32),
    radius: f64,
    fill: u32,
    fill_alpha: f64,
    border: Option<u32>,
}

/// Draws overlays onto output-size images.
pub struct Painter {
    font: fontdue::Font,
    width: usize,
    height: usize,
    /// Text size in output pixels.
    size: f32,
    palette: Palette,
    glyphs: RefCell<HashMap<(char, u32), Glyph>>,
}

impl Painter {
    /// A painter for a `width` by `height` output in `theme`, with text
    /// sized for a frame `width` wide.
    pub fn new(width: usize, height: usize, theme: Theme) -> Painter {
        Painter {
            font: fontdue::Font::from_bytes(FONT, fontdue::FontSettings::default())
                .expect("the embedded font is valid"),
            width,
            height,
            size: Painter::text_size(width),
            palette: Palette::of(theme),
            glyphs: RefCell::new(HashMap::new()),
        }
    }

    /// The text size for an output `width` wide.
    fn text_size(width: usize) -> f32 {
        (width as f32 * 0.021).clamp(14.0, 24.0)
    }

    /// How tall a caption band is for an output `width` wide.
    pub fn band_height(width: usize) -> usize {
        (Painter::text_size(width) * 2.6).round() as usize
    }

    /// Fill a caption band's background, `top` rows down for `height` rows,
    /// with the line between it and the frame on the side given by
    /// `rule_at_top`.
    pub fn fill_band(&self, pixels: &mut [u32], top: usize, height: usize, rule_at_top: bool) {
        for y in top..(top + height).min(self.height) {
            let rule = if rule_at_top {
                y == top
            } else {
                y + 1 == top + height
            };
            let color = if rule {
                self.palette.band_rule
            } else {
                self.palette.band
            };
            pixels[y * self.width..(y + 1) * self.width].fill(color);
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
        let (fx, fy, fw, fh) = shown.frame;
        let margin = (self.size * 0.6).round() as i32;

        self.draw_marks(pixels, shown, events, &mut drawn);

        let mut caption_box = None;
        if let Some((i, level, place)) = shown.caption {
            let text = &events.captions[i].0;
            let size = self.size;
            let opacity = level as f64 / LEVELS as f64;
            let (w, h) = self.plate_size(text, size);
            match place {
                Place::Below | Place::Above => {
                    // A subtitle in its band: plain text, centered.
                    let band = Painter::band_height(self.width) as i32;
                    let top = if place == Place::Below {
                        fy + fh
                    } else {
                        fy - band
                    };
                    let x = fx + (fw - w) / 2;
                    let y = top + (band - h) / 2;
                    self.text(pixels, (x, y, w, h), text, size, self.palette.text, opacity);
                    drawn.add(x, y, w, h);
                }
                Place::Bottom | Place::Top => {
                    let x = ax + (aw - w) / 2;
                    let y = if place == Place::Bottom {
                        ay + ah - margin - h
                    } else {
                        ay + margin
                    };
                    let plate = Plate {
                        rect: (x, y, w, h),
                        radius: h as f64 * 0.32,
                        fill: self.palette.backing,
                        fill_alpha: self.palette.backing_alpha,
                        border: Some(self.palette.border),
                    };
                    self.plate(pixels, &plate, opacity);
                    self.text(pixels, (x, y, w, h), text, size, self.palette.text, opacity);
                    drawn.add(x, y, w, h);
                    caption_box = Some((x, y, w, h));
                }
            }
        }
        if let Some((i, level)) = shown.badge {
            let text = &events.keys[i].0;
            let size = (self.size * 0.85).round();
            let (w, h) = self.plate_size(text, size);
            let x = ax + aw - margin - w;
            let mut y = ay + ah - margin - h;
            // Above the caption rather than over it.
            if let Some((cx, cy, cw, _)) = caption_box {
                if x < cx + cw && cx < x + w && cy > ay + ah / 2 {
                    y = cy - margin / 2 - h;
                }
            }
            let opacity = level as f64 / LEVELS as f64;
            let plate = Plate {
                rect: (x, y, w, h),
                radius: size as f64 * 0.35,
                fill: self.palette.backing,
                fill_alpha: self.palette.backing_alpha,
                border: Some(self.palette.border),
            };
            self.plate(pixels, &plate, opacity);
            self.text(pixels, (x, y, w, h), text, size, self.palette.text, opacity);
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

    /// Draw the highlights showing: a spotlight's dimming first, then each
    /// mark's outline, number, and label.
    fn draw_marks(&self, pixels: &mut [u32], shown: &Shown, events: &Events, drawn: &mut Bounds) {
        let pad = (self.size * 0.3).round() as i32;
        let padded =
            |(x, y, w, h): (i32, i32, i32, i32)| (x - pad, y - pad, w + 2 * pad, h + 2 * pad);
        let radius = (self.size * 0.35) as f64;
        let (fx, fy, fw, fh) = shown.frame;

        let spots: Vec<((i32, i32, i32, i32), f64)> = shown
            .marks
            .iter()
            .filter(|(i, _, _)| events.marks[*i].style == HighlightStyle::Spotlight)
            .map(|&(_, rect, level)| (padded(rect), level as f64 / LEVELS as f64))
            .collect();
        if let Some(opacity) = spots.iter().map(|(_, o)| *o).reduce(f64::max) {
            for y in fy..fy + fh {
                for x in fx..fx + fw {
                    // How far inside the nearest spotlight this pixel is.
                    let inside = spots
                        .iter()
                        .map(|(r, _)| rounded_cover(*r, radius, x, y))
                        .fold(0.0, f64::max);
                    self.blend(pixels, x, y, 0x000000, DIM * opacity * (1.0 - inside));
                }
            }
            drawn.add(fx, fy, fw, fh);
        }

        let stroke = (self.size * 0.14).max(2.0) as f64;
        for &(i, rect, level) in &shown.marks {
            let mark = &events.marks[i];
            let opacity = level as f64 / LEVELS as f64;
            let (x, y, w, h) = padded(rect);
            let outline = Plate {
                rect: (x, y, w, h),
                radius,
                fill: ACCENT,
                fill_alpha: if mark.style == HighlightStyle::Box {
                    0.10
                } else {
                    0.0
                },
                border: None,
            };
            self.plate(pixels, &outline, opacity);
            self.ring(pixels, (x, y, w, h), radius, stroke, ACCENT, opacity);
            drawn.add(x - 2, y - 2, w + 4, h + 4);

            // A number goes left of the mark, at its middle, or right of it
            // when the frame leaves no room on the left; it is small enough
            // that marks on neighboring lines keep their numbers apart.
            let mut beside = x + w;
            if let Some(n) = mark.number {
                let r = (self.size * 0.6).round() as i32;
                let left = x - r - 3;
                let cx = if left - r >= fx {
                    left
                } else {
                    beside = x + w + 2 * r + 3;
                    x + w + r + 3
                };
                let cy = y + h / 2;
                let disc = Plate {
                    rect: (cx - r, cy - r, 2 * r, 2 * r),
                    radius: r as f64,
                    fill: ACCENT,
                    fill_alpha: 1.0,
                    border: None,
                };
                self.plate(pixels, &disc, opacity);
                let size = (self.size * 0.75).round();
                self.text(
                    pixels,
                    (cx - r, cy - r, 2 * r, 2 * r),
                    &n.to_string(),
                    size,
                    ON_ACCENT,
                    opacity,
                );
                drawn.add(cx - r, cy - r, 2 * r, 2 * r);
            }

            if let Some(label) = &mark.label {
                let size = (self.size * 0.85).round();
                let (lw, lh) = self.plate_size(label, size);
                let gap = (self.size * 0.4).round() as i32;
                // Beside the mark when it fits in the frame, where it
                // covers least; else below it, else above.
                let (lx, ly) = if beside + gap + lw <= fx + fw {
                    (beside + gap, y + (h - lh) / 2)
                } else if y + h + gap + lh <= fy + fh {
                    (x.clamp(fx, (fx + fw - lw).max(fx)), y + h + gap)
                } else {
                    (x.clamp(fx, (fx + fw - lw).max(fx)), (y - gap - lh).max(fy))
                };
                let plate = Plate {
                    rect: (lx, ly, lw, lh),
                    radius: lh as f64 * 0.3,
                    fill: ACCENT,
                    fill_alpha: 1.0,
                    border: None,
                };
                self.plate(pixels, &plate, opacity);
                self.text(pixels, (lx, ly, lw, lh), label, size, ON_ACCENT, opacity);
                drawn.add(lx, ly, lw, lh);
            }
        }
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }

    /// The size of a plate holding `text` at `size`, with room around it.
    fn plate_size(&self, text: &str, size: f32) -> (i32, i32) {
        let h = (size * 1.8).round() as i32;
        let w = (self.text_width(text, size) + size * 1.4).round() as i32;
        (w, h)
    }

    /// A rounded box, filled and optionally bordered, at `opacity`.
    fn plate(&self, pixels: &mut [u32], plate: &Plate, opacity: f64) {
        let (x, y, w, h) = plate.rect;
        for py in y..y + h {
            for px in x..x + w {
                let cover = rounded_cover(plate.rect, plate.radius, px, py);
                self.blend(
                    pixels,
                    px,
                    py,
                    plate.fill,
                    cover * plate.fill_alpha * opacity,
                );
            }
        }
        if let Some(border) = plate.border {
            self.ring(pixels, plate.rect, plate.radius, 1.0, border, opacity);
        }
    }

    /// The edge of a rounded box, `thickness` wide on the inside.
    fn ring(
        &self,
        pixels: &mut [u32],
        rect: (i32, i32, i32, i32),
        radius: f64,
        thickness: f64,
        color: u32,
        opacity: f64,
    ) {
        let (x, y, w, h) = rect;
        let t = thickness.ceil() as i32;
        let inner = (x + t, y + t, w - 2 * t, h - 2 * t);
        for py in y..y + h {
            for px in x..x + w {
                if px >= x + t && px < x + w - t && py >= y + t && py < y + h - t {
                    // Away from the corners, the inside is not edge.
                    let near_corner = (px < x + radius as i32 + t
                        || px >= x + w - radius as i32 - t)
                        && (py < y + radius as i32 + t || py >= y + h - radius as i32 - t);
                    if !near_corner {
                        continue;
                    }
                }
                let outer = rounded_cover(rect, radius, px, py);
                let hole = rounded_cover(inner, (radius - thickness).max(0.0), px, py);
                self.blend(pixels, px, py, color, (outer - hole).max(0.0) * opacity);
            }
        }
    }

    /// `text` centered in `rect`, in `color`.
    fn text(
        &self,
        pixels: &mut [u32],
        (x, y, w, h): (i32, i32, i32, i32),
        text: &str,
        size: f32,
        color: u32,
        opacity: f64,
    ) {
        let line = self
            .font
            .horizontal_line_metrics(size)
            .expect("the font has horizontal metrics");
        let baseline =
            (y as f32 + (h as f32 - (line.ascent - line.descent)) / 2.0 + line.ascent).round();
        let mut pen = x as f32 + w as f32 / 2.0 - self.text_width(text, size) / 2.0;
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
                        color,
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

/// How much of pixel `(px, py)` a rounded box at `rect` covers, from 0 to 1.
fn rounded_cover((x, y, w, h): (i32, i32, i32, i32), radius: f64, px: i32, py: i32) -> f64 {
    let (cx, cy) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
    let radius = radius.min(w as f64 / 2.0).min(h as f64 / 2.0).max(0.0);
    let (hw, hh) = (w as f64 / 2.0 - radius, h as f64 / 2.0 - radius);
    // Distance outside the rounded rectangle, negative inside.
    let dx = ((px as f64 + 0.5 - cx).abs() - hw).max(0.0);
    let dy = ((py as f64 + 0.5 - cy).abs() - hh).max(0.0);
    let d = (dx * dx + dy * dy).sqrt() - radius;
    (0.5 - d).clamp(0.0, 1.0)
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

    /// A layout with no bands and no pane, where the screen maps to the
    /// output one to one.
    fn whole(w: i32, h: i32) -> impl Fn(&dyn Fn(Layout) -> Shown) -> Shown {
        move |f: &dyn Fn(Layout) -> Shown| {
            let map = |r: Rect| (r.x as i32, r.y as i32, r.w as i32, r.h as i32);
            f(Layout {
                frame: (0, 0, w, h),
                area: (0, 0, w, h),
                map: &map,
            })
        }
    }

    fn shown_at(events: &Events, tick: usize, ticks: usize, w: i32, h: i32) -> Shown {
        whole(w, h)(&|layout| events.shown(tick, ticks, 10, |_| None, &layout))
    }

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
        events.caption(10, Some("Hello"), CaptionPosition::Bottom);
        events.caption(40, None, CaptionPosition::Bottom);
        let at = |tick| shown_at(&events, tick, 100, 100, 100).caption;
        assert_eq!(at(9), None);
        // Three ticks of fade at 10 fps.
        assert!(matches!(at(10), Some((0, l, Place::Bottom)) if l > 0 && l < LEVELS));
        assert_eq!(at(20), Some((0, LEVELS, Place::Bottom)));
        assert!(matches!(at(39), Some((0, l, _)) if l < LEVELS));
        assert_eq!(at(40), None);
    }

    #[test]
    fn the_last_caption_lasts_until_the_next_or_the_end() {
        let mut events = Events::default();
        events.caption(0, Some("One"), CaptionPosition::Top);
        events.caption(20, Some("Two"), CaptionPosition::Below);
        let shown = |tick| {
            shown_at(&events, tick, 50, 100, 100)
                .caption
                .map(|(i, _, place)| (i, place))
        };
        assert_eq!(shown(19), Some((0, Place::Top)));
        assert_eq!(shown(20), Some((1, Place::Below)));
        assert_eq!(shown(48), Some((1, Place::Below)));
    }

    #[test]
    fn an_auto_caption_keeps_away_from_the_highlight() {
        let mut events = Events::default();
        events.caption(0, Some("Look"), CaptionPosition::Auto);
        let place = |events: &Events| shown_at(events, 5, 50, 100, 100).caption.map(|c| c.2);
        // Nothing to look at: the bottom.
        assert_eq!(place(&events), Some(Place::Bottom));
        events.highlight(
            0,
            Rect::new(10.0, 70.0, 20.0, 10.0),
            HighlightStyle::Box,
            None,
            None,
        );
        assert_eq!(place(&events), Some(Place::Top));
        events.clear_highlights(1);
        events.highlight(
            2,
            Rect::new(10.0, 10.0, 20.0, 10.0),
            HighlightStyle::Box,
            None,
            None,
        );
        assert_eq!(place(&events), Some(Place::Bottom));
    }

    #[test]
    fn a_highlight_lands_where_the_camera_puts_it_and_fades_in() {
        let mut events = Events::default();
        events.highlight(
            10,
            Rect::new(100.0, 50.0, 40.0, 20.0),
            HighlightStyle::Box,
            None,
            None,
        );
        // Zoomed in twice on the screen's top-left quarter.
        let map = |r: Rect| {
            (
                (r.x * 2.0) as i32,
                (r.y * 2.0) as i32,
                (r.w * 2.0) as i32,
                (r.h * 2.0) as i32,
            )
        };
        let layout = Layout {
            frame: (0, 0, 400, 300),
            area: (0, 0, 400, 300),
            map: &map,
        };
        let marks = |tick| events.shown(tick, 100, 10, |_| None, &layout).marks;
        assert!(marks(9).is_empty());
        assert!(matches!(marks(10)[..], [(0, (200, 100, 80, 40), l)] if l < LEVELS));
        assert_eq!(marks(20), vec![(0, (200, 100, 80, 40), LEVELS)]);
        events.clear_highlights(30);
        let marks = |tick| events.shown(tick, 100, 10, |_| None, &layout).marks;
        assert!(marks(30).is_empty());
    }

    #[test]
    fn a_ripple_starts_at_the_click_and_ends() {
        let mut events = Events::default();
        events.click(5);
        let at = |tick| {
            whole(100, 100)(&|layout| {
                events.shown(tick, 100, 10, |t| (t == 5).then_some((30, 40)), &layout)
            })
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
        let at = |tick| shown_at(&events, tick, 100, 100, 100).badge;
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

    /// The pixels `draw` changed from a plain white image, and the
    /// rectangle it reported.
    fn drawn(
        painter: &Painter,
        events: &Events,
        shown: &Shown,
        w: usize,
        h: usize,
    ) -> (Vec<usize>, (usize, usize, usize, usize), Vec<u32>) {
        let mut pixels = vec![0xffffff; w * h];
        let bounds = painter.draw(&mut pixels, shown, events).unwrap();
        let changed = (0..pixels.len())
            .filter(|&i| pixels[i] != 0xffffff)
            .collect();
        (changed, bounds, pixels)
    }

    #[test]
    fn drawing_marks_only_what_it_covers() {
        let mut events = Events::default();
        events.caption(0, Some("Hi"), CaptionPosition::Bottom);
        let painter = Painter::new(400, 300, Theme::Dark);
        let shown = shown_at(&events, 10, 100, 400, 300);
        let (changed, (x, y, w, h), pixels) = drawn(&painter, &events, &shown, 400, 300);
        assert!(y + h <= 300 && y > 200, "near the bottom: {y}+{h}");
        assert!(x > 100 && x + w < 300, "centered: {x}+{w}");
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|&i| {
            let (px, py) = (i % 400, i / 400);
            px >= x && px < x + w && py >= y && py < y + h
        }));
        // The backing is dark under the middle of the text.
        let middle = pixels[(y + h / 2) * 400 + x + 4];
        assert!(middle & 0xff < 0x80, "{middle:x}");
    }

    #[test]
    fn a_band_caption_stays_in_its_band() {
        let band = Painter::band_height(400) as i32;
        let mut events = Events::default();
        events.caption(0, Some("Under the frame"), CaptionPosition::Below);
        let painter = Painter::new(400, 300 + band as usize, Theme::Light);
        let map = |r: Rect| (r.x as i32, r.y as i32, r.w as i32, r.h as i32);
        let layout = Layout {
            frame: (0, 0, 400, 300),
            area: (0, 0, 400, 300),
            map: &map,
        };
        let shown = events.shown(10, 100, 10, |_| None, &layout);
        let (changed, _, _) = drawn(&painter, &events, &shown, 400, 300 + band as usize);
        assert!(!changed.is_empty());
        assert!(
            changed.iter().all(|&i| i / 400 >= 300),
            "drawn over the frame"
        );
    }

    #[test]
    fn a_spotlight_dims_around_the_mark_and_not_inside() {
        let mut events = Events::default();
        events.highlight(
            0,
            Rect::new(150.0, 100.0, 100.0, 60.0),
            HighlightStyle::Spotlight,
            None,
            None,
        );
        let painter = Painter::new(400, 300, Theme::Dark);
        let shown = shown_at(&events, 10, 100, 400, 300);
        let (_, _, pixels) = drawn(&painter, &events, &shown, 400, 300);
        assert!(pixels[10 * 400 + 10] & 0xff < 0xa0, "the corner is dimmed");
        assert_eq!(
            pixels[130 * 400 + 200],
            0xffffff,
            "the middle of the mark is untouched"
        );
    }

    #[test]
    fn a_label_goes_beside_its_mark() {
        let painter = Painter::new(400, 300, Theme::Dark);
        let mut events = Events::default();
        events.highlight(
            0,
            Rect::new(20.0, 100.0, 100.0, 20.0),
            HighlightStyle::Box,
            Some("Here"),
            None,
        );
        let shown = shown_at(&events, 10, 100, 400, 300);
        let (_, _, pixels) = drawn(&painter, &events, &shown, 400, 300);
        // Accent to the right of the mark, on its row, and none under it.
        assert!((130..400).any(|col| pixels[110 * 400 + col] == ACCENT));
        assert!((140..300).all(|row| pixels[row * 400 + 60] != ACCENT));
    }

    #[test]
    fn a_number_goes_right_of_a_mark_at_the_frame_s_left_edge() {
        let painter = Painter::new(400, 300, Theme::Dark);
        let mut events = Events::default();
        events.highlight(
            0,
            Rect::new(4.0, 100.0, 100.0, 20.0),
            HighlightStyle::Box,
            None,
            Some(1),
        );
        let shown = shown_at(&events, 10, 100, 400, 300);
        let (_, _, pixels) = drawn(&painter, &events, &shown, 400, 300);
        // The disc is solid accent just right of the mark, and nothing
        // covers the start of the marked text.
        assert!((112..140).any(|col| pixels[110 * 400 + col] == ACCENT));
        assert!((6..14).all(|col| pixels[110 * 400 + col] != ACCENT));
    }

    #[test]
    fn a_wide_mark_s_label_goes_below_or_above_when_there_is_no_room() {
        let painter = Painter::new(400, 300, Theme::Dark);
        let label_rows = |y: f64| {
            let mut events = Events::default();
            events.highlight(
                0,
                Rect::new(150.0, y, 240.0, 20.0),
                HighlightStyle::Box,
                Some("Here"),
                None,
            );
            let shown = shown_at(&events, 10, 100, 400, 300);
            let (_, _, pixels) = drawn(&painter, &events, &shown, 400, 300);
            // Rows where the label's accent fill shows, under the mark's
            // left edge.
            (0..300)
                .filter(|&row| pixels[row * 400 + 160] == ACCENT)
                .collect::<Vec<usize>>()
        };
        // The mark's own outline is the same color, so look only outside
        // it: from 40 to 60, padded by a few rows.
        let low = label_rows(40.0);
        assert!(low.iter().any(|&r| r > 66), "a label below: {low:?}");
        assert!(low.iter().all(|&r| r > 30), "nothing above: {low:?}");
        // From 270 to 290 there is no room below in a 300-row frame.
        let high = label_rows(270.0);
        assert!(high.iter().any(|&r| r < 262), "a label above: {high:?}");
        assert!(high.iter().all(|&r| r < 297), "nothing below: {high:?}");
    }
}
