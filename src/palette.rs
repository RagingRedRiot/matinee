//! Choosing a clip's palette: the colors it shows, each weighted by how
//! long it is on screen, reduced to at most 255 by median cut and then
//! refined with weighted k-means. The result depends only on the colors and
//! their weights, so the same clip always gets the same palette.

use std::collections::HashMap;

/// How many rounds of k-means refine the median cut.
const ROUNDS: usize = 8;

/// Colors as 0xRRGGBB, with how much each is seen.
pub type Histogram = HashMap<u32, u64>;

/// At most `max` colors that stand in for `histogram` with little error.
pub fn quantize(histogram: &Histogram, max: usize) -> Vec<u32> {
    let mut colors: Vec<(u32, u64)> = histogram
        .iter()
        .filter(|&(_, &w)| w > 0)
        .map(|(&c, &w)| (c, w))
        .collect();
    // A fixed order, so the result does not depend on the map's.
    colors.sort_unstable();
    if colors.len() <= max {
        return colors.into_iter().map(|(c, _)| c).collect();
    }
    let mut centers = median_cut(&mut colors, max);
    let mut nearest = vec![0usize; colors.len()];
    for _ in 0..ROUNDS {
        let mut sums = vec![[0f64; 4]; centers.len()];
        let mut moved = false;
        for (i, &(c, w)) in colors.iter().enumerate() {
            let k = closest(&centers, rgb(c));
            moved |= nearest[i] != k;
            nearest[i] = k;
            let [r, g, b] = rgb(c);
            let w = w as f64;
            let sum = &mut sums[k];
            sum[0] += r * w;
            sum[1] += g * w;
            sum[2] += b * w;
            sum[3] += w;
        }
        for (center, sum) in centers.iter_mut().zip(&sums) {
            if sum[3] > 0.0 {
                *center = [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]];
            }
        }
        if !moved {
            break;
        }
    }
    centers
        .into_iter()
        .map(|[r, g, b]| {
            let c = |v: f64| v.round().clamp(0.0, 255.0) as u32;
            (c(r) << 16) | (c(g) << 8) | c(b)
        })
        .collect()
}

/// Finds the closest color in a palette.
pub struct Nearest {
    centers: Vec<[f64; 3]>,
}

impl Nearest {
    pub fn new(palette: &[u32]) -> Nearest {
        Nearest {
            centers: palette.iter().map(|&c| rgb(c)).collect(),
        }
    }

    /// The index of the palette color closest to `color`.
    pub fn index(&self, color: u32) -> usize {
        closest(&self.centers, rgb(color))
    }
}

fn rgb(c: u32) -> [f64; 3] {
    [
        ((c >> 16) & 0xff) as f64,
        ((c >> 8) & 0xff) as f64,
        (c & 0xff) as f64,
    ]
}

/// Squared distance, weighted toward green, to which the eye is most
/// sensitive.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    2.0 * dr * dr + 4.0 * dg * dg + 3.0 * db * db
}

fn closest(centers: &[[f64; 3]], color: [f64; 3]) -> usize {
    let mut best = (f64::MAX, 0);
    for (k, &center) in centers.iter().enumerate() {
        let d = distance(center, color);
        if d < best.0 {
            best = (d, k);
            if d == 0.0 {
                break;
            }
        }
    }
    best.1
}

/// Split the colors into `max` boxes, each time splitting the box with the
/// most weighted error at the weighted median of its widest channel, and
/// return each box's weighted mean.
fn median_cut(colors: &mut [(u32, u64)], max: usize) -> Vec<[f64; 3]> {
    // Boxes are ranges of `colors`, which is reordered as they split.
    let mut boxes: Vec<(usize, usize)> = vec![(0, colors.len())];
    while boxes.len() < max {
        let Some((at, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| b - a > 1)
            .map(|(i, &(a, b))| {
                let (channel, spread) = widest(&colors[a..b]);
                (i, channel, spread)
            })
            .filter(|&(_, _, spread)| spread > 0.0)
            .max_by(|x, y| x.2.total_cmp(&y.2))
            .map(|(i, channel, _)| (i, channel))
        else {
            break;
        };
        let (a, b) = boxes[at];
        let part = &mut colors[a..b];
        let shift = 16 - 8 * channel as u32;
        part.sort_unstable_by_key(|&(c, _)| ((c >> shift) & 0xff, c));
        let half = part.iter().map(|&(_, w)| w).sum::<u64>() / 2;
        let mut seen = 0;
        let mut split = 1;
        for (i, &(_, w)) in part.iter().enumerate() {
            seen += w;
            if seen >= half {
                split = (i + 1).clamp(1, part.len() - 1);
                break;
            }
        }
        boxes[at] = (a, a + split);
        boxes.push((a + split, b));
    }
    boxes
        .iter()
        .map(|&(a, b)| {
            let mut sum = [0f64; 4];
            for &(c, w) in &colors[a..b] {
                let [r, g, bl] = rgb(c);
                let w = w as f64;
                sum[0] += r * w;
                sum[1] += g * w;
                sum[2] += bl * w;
                sum[3] += w;
            }
            [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]]
        })
        .collect()
}

/// The channel (0 red, 1 green, 2 blue) with the most weighted variance in
/// `colors`, and that variance times the colors' total weight, so heavily
/// used, spread-out boxes split first.
fn widest(colors: &[(u32, u64)]) -> (usize, f64) {
    let mut sum = [0f64; 3];
    let mut squares = [0f64; 3];
    let mut total = 0f64;
    for &(c, w) in colors {
        let w = w as f64;
        for (i, v) in rgb(c).into_iter().enumerate() {
            sum[i] += v * w;
            squares[i] += v * v * w;
        }
        total += w;
    }
    let scale = [2.0, 4.0, 3.0];
    (0..3)
        .map(|i| {
            let mean = sum[i] / total;
            let variance = (squares[i] / total - mean * mean).max(0.0);
            (i, variance * scale[i] * total)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_colors_are_kept_exactly() {
        let histogram: Histogram = [(0x102030, 5), (0xffffff, 1)].into();
        let mut palette = quantize(&histogram, 255);
        palette.sort();
        assert_eq!(palette, [0x102030, 0xffffff]);
    }

    #[test]
    fn heavy_colors_survive_and_light_ones_merge() {
        // A background shown everywhere, a rare accent, and a gradient of
        // many near-identical greys.
        let mut histogram: Histogram = [(0x0f172a, 1_000_000), (0xff8800, 50)].into();
        for v in 0..200u32 {
            histogram.insert(0x404040 + v * 0x000101, 10);
        }
        let palette = quantize(&histogram, 16);
        assert!(palette.len() <= 16);
        assert!(palette.contains(&0x0f172a), "{palette:x?}");
        let accent = palette[Nearest::new(&palette).index(0xff8800)];
        let [_, r, g, b] = accent.to_be_bytes();
        assert!(
            r > 0xe0 && (0x70..0xa0).contains(&g) && b < 0x20,
            "{accent:06x}"
        );
    }

    #[test]
    fn the_same_colors_give_the_same_palette() {
        let mut histogram = Histogram::new();
        for i in 0..5000u32 {
            histogram.insert(i.wrapping_mul(2_654_435_761) & 0xffffff, (i % 7 + 1) as u64);
        }
        assert_eq!(quantize(&histogram, 255), quantize(&histogram.clone(), 255));
    }
}
