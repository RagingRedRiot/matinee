//! matinee's terminal pane: a real pseudo-terminal, read by a terminal
//! emulator and drawn into each frame with an embedded font, so it looks
//! the same on every machine.
//!
//! One shell runs for the whole story (see [`Shell`]), so `cd` and exported
//! variables carry from one step to the next. matinee draws the prompt and
//! types the command line, which is how a staged step can show one command
//! while running another.

use crate::{
    frame::Frame,
    platform::{Boundary, Shell},
    story::Theme,
};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

const REGULAR: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");
pub(crate) const BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf");

/// The prompt matinee draws before each command: a bold cyan `$`.
const PROMPT: &str = "\x1b[1;36m$\x1b[0m ";

pub struct Terminal {
    screen: Arc<Mutex<vt100::Parser>>,
    shell: Shell,
}

impl Terminal {
    /// Start a shell on a terminal of `rows` by `cols`, with `env` added
    /// to the boundary's environment.
    pub fn start(
        boundary: &Boundary,
        rows: u16,
        cols: u16,
        env: &[(String, String)],
    ) -> io::Result<Terminal> {
        let screen = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let shell = Shell::start(boundary, rows, cols, env, screen.clone())?;
        let terminal = Terminal { screen, shell };
        terminal.show(PROMPT);
        Ok(terminal)
    }

    /// Draw text into the pane as if the terminal had printed it.
    pub fn show(&self, text: &str) {
        self.screen.lock().unwrap().process(text.as_bytes());
    }

    /// Start a command. Its status arrives through [`Terminal::finished`].
    pub fn run(&mut self, command: &str) -> io::Result<()> {
        self.shell.run(command)
    }

    /// The exit status of the last command, once it has finished.
    pub fn finished(&mut self) -> io::Result<Option<i32>> {
        self.shell.finished()
    }

    /// After a command: start a fresh line if needed, then show the prompt.
    pub fn prompt(&self) {
        let column = self.screen.lock().unwrap().screen().cursor_position().1;
        if column != 0 {
            self.show("\r\n");
        }
        self.show(PROMPT);
    }

    /// Send keys to whatever is running in the terminal.
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.shell.send(bytes)
    }

    /// Whether the running program has asked for application cursor keys.
    pub fn application_cursor(&self) -> bool {
        self.screen.lock().unwrap().screen().application_cursor()
    }

    /// Whether the terminal shows `text` anywhere on screen.
    pub fn contains(&self, text: &str) -> bool {
        self.screen
            .lock()
            .unwrap()
            .screen()
            .contents()
            .contains(text)
    }

    /// Everything on screen, for diagnostics.
    pub fn contents(&self) -> String {
        self.screen.lock().unwrap().screen().contents()
    }
}

/// A rasterized glyph's character, weight (bold or not), and scale.
type GlyphKey = (char, bool, u32);

/// How the pane looks, and where it sits on the output.
pub struct Pane {
    regular: fontdue::Font,
    bold: fontdue::Font,
    /// Text size in points.
    font_size: f32,
    background: u32,
    foreground: u32,
    palette: [u32; 16],
    /// x, y, width, height in points.
    pub rect: (f32, f32, f32, f32),
    padding: f32,
    glyphs: Mutex<HashMap<GlyphKey, (fontdue::Metrics, Vec<u8>)>>,
}

impl Pane {
    pub fn new(rect: (f32, f32, f32, f32), font_size: f32, theme: Theme) -> Pane {
        let load = |data| {
            fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
                .expect("the embedded font is valid")
        };
        let (background, foreground, palette) = match theme {
            Theme::Dark => (
                0x0F172A,
                0xE2E8F0,
                [
                    0x1E293B, 0xF87171, 0x4ADE80, 0xFACC15, 0x60A5FA, 0xC084FC, 0x22D3EE, 0xCBD5E1,
                    0x475569, 0xFCA5A5, 0x86EFAC, 0xFDE047, 0x93C5FD, 0xD8B4FE, 0x67E8F9, 0xF8FAFC,
                ],
            ),
            Theme::Light => (
                0xF8FAFC,
                0x1E293B,
                [
                    0x334155, 0xDC2626, 0x16A34A, 0xA16207, 0x2563EB, 0x9333EA, 0x0891B2, 0x64748B,
                    0x475569, 0xEF4444, 0x22C55E, 0xCA8A04, 0x3B82F6, 0xA855F7, 0x06B6D4, 0x0F172A,
                ],
            ),
        };
        Pane {
            regular: load(REGULAR),
            bold: load(BOLD),
            font_size,
            background,
            foreground,
            palette,
            rect,
            padding: (font_size * 0.8).round(),
            glyphs: Mutex::new(HashMap::new()),
        }
    }

    /// Cell width and line height in points.
    fn cell(&self) -> (f32, f32) {
        let width = self.regular.metrics('M', self.font_size).advance_width;
        let line = self
            .regular
            .horizontal_line_metrics(self.font_size)
            .expect("the embedded font has line metrics");
        (width, (line.new_line_size * 1.15).ceil())
    }

    /// Rows and columns that fit in the pane.
    pub fn grid(&self) -> (u16, u16) {
        let (cw, lh) = self.cell();
        let (_, _, w, h) = self.rect;
        (
            (((h - 2.0 * self.padding) / lh).floor() as u16).max(1),
            (((w - 2.0 * self.padding) / cw).floor() as u16).max(1),
        )
    }

    fn color(&self, c: vt100::Color, default: u32) -> u32 {
        match c {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) if i < 16 => self.palette[i as usize],
            vt100::Color::Idx(i) if i < 232 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
                level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
            }
            vt100::Color::Idx(i) => {
                let g = 8 + (i as u32 - 232) * 10;
                g << 16 | g << 8 | g
            }
            vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        }
    }

    /// Draw the terminal's screen into `frame`, which is `scale` device
    /// pixels per point.
    pub fn draw(&self, terminal: &Terminal, frame: &mut Frame, scale: u32) {
        let s = scale as f32;
        let px = |v: f32| (v * s).round() as i64;
        let (x0, y0, w, h) = self.rect;
        fill(
            frame,
            px(x0),
            px(y0),
            px(x0 + w),
            px(y0 + h),
            self.background,
        );

        let (cw, lh) = self.cell();
        let size = self.font_size * s;
        let ascent = self
            .regular
            .horizontal_line_metrics(size)
            .expect("the embedded font has line metrics")
            .ascent;
        // The glyph's own line height, centered in the taller row.
        let lead = (lh - lh / 1.15) / 2.0;
        let parser = terminal.screen.lock().unwrap();
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let mut glyphs = self.glyphs.lock().unwrap();
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                let cx = x0 + self.padding + col as f32 * cw;
                let cy = y0 + self.padding + row as f32 * lh;
                let mut fg = self.color(cell.fgcolor(), self.foreground);
                let mut bg = self.color(cell.bgcolor(), self.background);
                if cell.bold() {
                    if let vt100::Color::Idx(i @ 0..8) = cell.fgcolor() {
                        fg = self.palette[i as usize + 8];
                    }
                }
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if !screen.hide_cursor() && screen.cursor_position() == (row, col) {
                    bg = self.foreground;
                    fg = self.background;
                }
                if bg != self.background {
                    fill(frame, px(cx), px(cy), px(cx + cw), px(cy + lh), bg);
                }
                let Some(c) = cell.contents().chars().next() else {
                    continue;
                };
                if c == ' ' {
                    continue;
                }
                let bold = cell.bold();
                let (m, bitmap) = glyphs.entry((c, bold, scale)).or_insert_with(|| {
                    let font = if bold { &self.bold } else { &self.regular };
                    font.rasterize(c, size)
                });
                let gx = px(cx) + m.xmin as i64;
                let gy =
                    ((cy + lead) * s + ascent).round() as i64 - m.height as i64 - m.ymin as i64;
                blend(frame, gx, gy, m.width, m.height, bitmap, fg);
            }
        }
    }
}

fn fill(frame: &mut Frame, x0: i64, y0: i64, x1: i64, y1: i64, rgb: u32) {
    let (w, h) = (frame.width as i64, frame.height as i64);
    for y in y0.max(0)..y1.min(h) {
        let row = y as usize * frame.width;
        frame.pixels[row + x0.max(0) as usize..row + x1.min(w).max(0) as usize]
            .fill(0xff000000 | rgb);
    }
}

fn blend(frame: &mut Frame, x0: i64, y0: i64, w: usize, h: usize, alpha: &[u8], rgb: u32) {
    for gy in 0..h {
        let y = y0 + gy as i64;
        if y < 0 || y >= frame.height as i64 {
            continue;
        }
        for gx in 0..w {
            let x = x0 + gx as i64;
            let a = alpha[gy * w + gx] as u32;
            if a == 0 || x < 0 || x >= frame.width as i64 {
                continue;
            }
            let at = y as usize * frame.width + x as usize;
            let d = frame.pixels[at];
            let mix = |shift: u32| {
                let c = (rgb >> shift) & 0xff;
                let b = (d >> shift) & 0xff;
                ((c * a + b * (255 - a)) / 255) << shift
            };
            frame.pixels[at] = 0xff000000 | mix(16) | mix(8) | mix(0);
        }
    }
}
