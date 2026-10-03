//! GIF's LZW compression, with an optional lossy mode.
//!
//! Exact mode compresses as any GIF encoder does. Lossy mode, the idea
//! behind gifsicle's `--lossy`, lets a string in the dictionary stand in for
//! the next pixels when each of its colors is within a small distance of
//! the color it replaces, so matches run longer and the output shrinks.
//! The colors that change are close by design; a transparent pixel is never
//! replaced, and never replaces one, since what shows through it can be any
//! color. The caller gets back what a viewer will see, so the next frame is
//! compared against that rather than against what was asked for.

/// Codes for 8-bit pixels: 256 colors, then these two.
const CLEAR: u16 = 256;
const END: u16 = 257;
const FIRST: u16 = 258;
const MAX_CODES: usize = 4096;
const MIN_CODE_SIZE: u8 = 8;

/// How far a color may move in lossy mode, and the palette to measure by.
pub struct Lossy<'a> {
    /// The palette, as 0xRRGGBB.
    pub palette: &'a [u32],
    /// The largest change allowed in any one channel.
    pub tolerance: u8,
    /// The palette index that means transparent.
    pub transparent: Option<u8>,
}

impl Lossy<'_> {
    /// Whether `a` may stand in for `b`.
    pub fn close(&self, a: u8, b: u8) -> bool {
        if a == b {
            return true;
        }
        if Some(a) == self.transparent || Some(b) == self.transparent {
            return false;
        }
        let (Some(&x), Some(&y)) = (self.palette.get(a as usize), self.palette.get(b as usize))
        else {
            return false;
        };
        [16, 8, 0]
            .iter()
            .all(|s| ((x >> s) & 0xff).abs_diff((y >> s) & 0xff) <= self.tolerance as u32)
    }
}

/// The dictionary: for each code, the pixel it ends with and its children.
struct Dictionary {
    /// Each code's children, as (pixel, child code).
    children: Vec<Vec<(u8, u16)>>,
    next: u16,
}

impl Dictionary {
    fn new() -> Dictionary {
        Dictionary {
            children: vec![Vec::new(); MAX_CODES],
            next: FIRST,
        }
    }

    fn reset(&mut self) {
        for c in &mut self.children {
            c.clear();
        }
        self.next = FIRST;
    }

    fn child(&self, code: u16, pixel: u8) -> Option<u16> {
        self.children[code as usize]
            .iter()
            .find(|&&(p, _)| p == pixel)
            .map(|&(_, c)| c)
    }
}

/// Packs codes of varying width, least significant bit first.
struct Bits {
    out: Vec<u8>,
    buffer: u32,
    count: u32,
}

impl Bits {
    fn push(&mut self, code: u16, width: u32) {
        self.buffer |= (code as u32) << self.count;
        self.count += width;
        while self.count >= 8 {
            self.out.push(self.buffer as u8);
            self.buffer >>= 8;
            self.count -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.buffer as u8);
        }
        self.out
    }
}

/// Compress `pixels`, returning the minimum code size followed by the LZW
/// stream (as `gif`'s pre-encoded frames expect), and the pixels a viewer
/// will see, which differ from `pixels` only in lossy mode.
pub fn encode(pixels: &[u8], lossy: Option<&Lossy>) -> (Vec<u8>, Vec<u8>) {
    let mut bits = Bits {
        out: vec![MIN_CODE_SIZE],
        buffer: 0,
        count: 0,
    };
    let mut shown = Vec::with_capacity(pixels.len());
    let mut dict = Dictionary::new();
    // The decoder's table size, which sets the width of the next code: it
    // adds an entry for every code after the first since a clear.
    let mut decoder_size = FIRST as usize;
    let mut since_clear = 0usize;
    let width = |size: usize| (usize::BITS - size.leading_zeros()).clamp(9, 12);
    let emit = |bits: &mut Bits, code: u16, size: &mut usize, since: &mut usize| {
        bits.push(code, width(*size));
        if *since > 0 && *size < MAX_CODES {
            *size += 1;
        }
        *since += 1;
    };

    bits.push(CLEAR, width(decoder_size));
    let mut i = 0;
    while i < pixels.len() {
        let mut code = pixels[i] as u16;
        // The colors the current code stands for, from the trie walk.
        shown.push(pixels[i]);
        let mut j = i + 1;
        while j < pixels.len() {
            let want = pixels[j];
            if let Some(child) = dict.child(code, want) {
                code = child;
                shown.push(want);
                j += 1;
                continue;
            }
            let substitute = lossy.and_then(|l| {
                dict.children[code as usize]
                    .iter()
                    .filter(|&&(p, _)| l.close(p, want))
                    .min_by_key(|&&(p, _)| {
                        let (x, y) = (l.palette[p as usize], l.palette[want as usize]);
                        [16, 8, 0]
                            .iter()
                            .map(|s| ((x >> s) & 0xff).abs_diff((y >> s) & 0xff))
                            .sum::<u32>()
                    })
                    .copied()
            });
            match substitute {
                Some((p, child)) => {
                    code = child;
                    shown.push(p);
                    j += 1;
                }
                None => break,
            }
        }
        emit(&mut bits, code, &mut decoder_size, &mut since_clear);
        if j < pixels.len() {
            if (dict.next as usize) < MAX_CODES {
                let next = dict.next;
                dict.children[code as usize].push((pixels[j], next));
                dict.next += 1;
            } else {
                bits.push(CLEAR, width(decoder_size));
                dict.reset();
                decoder_size = FIRST as usize;
                since_clear = 0;
            }
        }
        i = j;
    }
    bits.push(END, width(decoder_size));
    (bits.finish(), shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode with the `gif` crate, through a one-frame GIF.
    fn decode(encoded: &[u8], width: u16, height: u16) -> Vec<u8> {
        let palette: Vec<u8> = (0..=255u8).flat_map(|i| [i, i, i]).collect();
        let mut gif = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut gif, width, height, &palette).unwrap();
            let frame = gif::Frame {
                width,
                height,
                buffer: std::borrow::Cow::Borrowed(encoded),
                ..gif::Frame::default()
            };
            encoder.write_lzw_pre_encoded_frame(&frame).unwrap();
        }
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(&gif[..]).unwrap();
        decoder.read_next_frame().unwrap().unwrap().buffer.to_vec()
    }

    fn noise(n: usize, seed: u32, colors: u32) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x % colors) as u8
            })
            .collect()
    }

    #[test]
    fn exact_mode_round_trips() {
        let cases: Vec<(Vec<u8>, u16, u16)> = vec![
            (vec![7], 1, 1),
            (vec![3; 640 * 480], 640, 480),
            // Enough distinct strings to fill the dictionary many times.
            (noise(400 * 300, 1, 256), 400, 300),
            (noise(300 * 200, 2, 4), 300, 200),
            ((0..=255u8).cycle().take(512 * 64).collect(), 512, 64),
        ];
        for (pixels, w, h) in cases {
            let (encoded, shown) = encode(&pixels, None);
            assert_eq!(shown, pixels);
            assert_eq!(decode(&encoded, w, h), pixels, "{w}x{h}");
        }
    }

    #[test]
    fn exact_mode_is_about_as_small_as_the_gif_crate() {
        // Mostly a few colors, as frames are, but using the whole palette,
        // as frames do, so both start from the same code size.
        let mut pixels = noise(400 * 300, 3, 16);
        pixels[0] = 255;
        let (ours, _) = encode(&pixels, None);
        let mut theirs = gif::Frame {
            width: 400,
            height: 300,
            buffer: std::borrow::Cow::Owned(pixels),
            ..gif::Frame::default()
        };
        theirs.make_lzw_pre_encoded();
        let (a, b) = (ours.len() as f64, theirs.buffer.len() as f64);
        assert!(a <= b * 1.02, "{a} against {b}");
    }

    #[test]
    fn lossy_mode_shrinks_and_shows_what_it_says() {
        // A grey ramp palette and a noisy image near mid-grey.
        let palette: Vec<u32> = (0..=255u32).map(|i| i * 0x010101).collect();
        let pixels: Vec<u8> = noise(320 * 240, 4, 12).iter().map(|v| 120 + v).collect();
        let lossy = Lossy {
            palette: &palette,
            tolerance: 8,
            transparent: Some(255),
        };
        let (exact, _) = encode(&pixels, None);
        let (small, shown) = encode(&pixels, Some(&lossy));
        assert!(
            small.len() * 3 < exact.len() * 2,
            "{} vs {}",
            small.len(),
            exact.len()
        );
        assert_eq!(decode(&small, 320, 240), shown);
        for (&want, &got) in pixels.iter().zip(&shown) {
            assert!(want.abs_diff(got) <= 8);
        }
    }

    #[test]
    fn lossy_mode_keeps_transparency_exact() {
        let palette: Vec<u32> = (0..=255u32).map(|i| i * 0x010101).collect();
        let pixels: Vec<u8> = noise(200 * 100, 5, 4)
            .iter()
            .map(|&v| if v == 0 { 255 } else { 250 + v })
            .collect();
        let lossy = Lossy {
            palette: &palette,
            tolerance: 40,
            transparent: Some(255),
        };
        let (encoded, shown) = encode(&pixels, Some(&lossy));
        assert_eq!(decode(&encoded, 200, 100), shown);
        for (&want, &got) in pixels.iter().zip(&shown) {
            assert_eq!(want == 255, got == 255);
        }
    }
}
