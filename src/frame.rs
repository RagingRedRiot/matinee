//! One rendered frame of the whole output.

use std::io;

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
