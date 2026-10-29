//! What an accessibility tree looks like to the runner, whichever platform
//! read it.

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub role: String,
    /// The node's depth below its window; windows are 0.
    pub depth: usize,
    /// The index of its parent in the same list; `None` for a window.
    pub parent: Option<usize>,
    /// The title of the window it is in.
    pub window: String,
    /// Position and size relative to its window, in logical points.
    pub extents: Option<(i32, i32, i32, i32)>,
    pub showing: bool,
    pub focused: bool,
    /// The contents of a text field, for the roles in `TEXT_ROLES`.
    pub text: Option<String>,
    /// A slider's current, minimum, and maximum value.
    pub value: Option<(f64, f64, f64)>,
}

/// Node `i`'s position and size relative to its window, in logical points.
///
/// Toolkits that report device pixels (AccessKit) are recognized by their
/// window's root element measuring the window's size times the output
/// `scale`, in both directions, and scaled back down. Anything else, such
/// as a root still at an old size while the window is resized, is taken
/// as points. A platform whose trees always report device pixels says so,
/// and its extents are always scaled down.
pub fn in_points(
    nodes: &[Node],
    i: usize,
    window: (i32, i32),
    scale: i32,
) -> Option<(i32, i32, i32, i32)> {
    scaled(nodes, i, window, scale, super::ACCESSIBILITY_IN_PIXELS)
}

/// [`in_points`], for a platform whose trees report device pixels always
/// (`pixels`) or only sometimes.
fn scaled(
    nodes: &[Node],
    i: usize,
    window: (i32, i32),
    scale: i32,
    pixels: bool,
) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = nodes[i].extents?;
    let near = |a: i32, b: i32| (a - b).abs() <= scale;
    // Some Chromium browsers report the top-level frame in points but the
    // document beneath it in device pixels. Use the nearest ancestor whose
    // size identifies its coordinate space, rather than only the root.
    let device = pixels
        || scale > 1
            && std::iter::successors(Some(i), |&j| nodes[j].parent)
                .filter_map(|j| nodes[j].extents)
                .find_map(|(_, _, rw, rh)| {
                    if near(rw, window.0 * scale) && near(rh, window.1 * scale) {
                        Some(true)
                    } else if near(rw, window.0) && near(rh, window.1) {
                        Some(false)
                    } else {
                        None
                    }
                })
                .unwrap_or(false);
    let f = if device { scale } else { 1 };
    Some((x / f, y / f, w / f, h / f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_that_reports_pixels_is_always_scaled() {
        // A window's root with its invisible borders, as UI Automation
        // reports it, is not exactly the window's size times the scale.
        let nodes = vec![
            node((-18, 0, 2436, 2018), None),
            node((40, 20, 200, 60), Some(0)),
        ];
        assert_eq!(
            scaled(&nodes, 1, (1200, 1000), 2, true),
            Some((20, 10, 100, 30))
        );
    }

    fn node(extents: (i32, i32, i32, i32), parent: Option<usize>) -> Node {
        Node {
            name: String::new(),
            role: String::new(),
            depth: parent.map_or(0, |_| 1),
            parent,
            window: String::new(),
            extents: Some(extents),
            showing: true,
            focused: false,
            text: None,
            value: None,
        }
    }

    #[test]
    fn device_pixels_are_scaled_to_points() {
        // A 1200-point window whose root reports 2400 device pixels.
        let nodes = [
            node((0, 0, 2400, 1440), None),
            node((20, 12, 210, 46), Some(0)),
        ];
        assert_eq!(
            scaled(&nodes, 1, (1200, 720), 2, false),
            Some((10, 6, 105, 23))
        );
    }

    #[test]
    fn device_pixel_document_under_point_sized_frame() {
        let nodes = [
            node((0, 0, 1200, 800), None),
            node((0, 0, 2400, 1600), Some(0)),
            node((610, 654, 892, 100), Some(1)),
        ];
        assert_eq!(
            scaled(&nodes, 2, (1200, 800), 2, false),
            Some((305, 327, 446, 50))
        );
        assert_eq!(
            scaled(&nodes, 0, (1200, 800), 2, false),
            Some((0, 0, 1200, 800))
        );
    }

    #[test]
    fn a_root_at_an_old_size_is_not_taken_for_device_pixels() {
        // Mid-resize: the root still reports 1300x900 in an 800x600 window.
        let nodes = [
            node((0, 0, 1300, 900), None),
            node((20, 12, 210, 46), Some(0)),
        ];
        assert_eq!(
            scaled(&nodes, 1, (800, 600), 2, false),
            Some((20, 12, 210, 46))
        );
    }

    #[test]
    fn points_are_left_alone() {
        let nodes = [
            node((0, 0, 1200, 720), None),
            node((20, 12, 210, 46), Some(0)),
        ];
        assert_eq!(
            scaled(&nodes, 1, (1200, 720), 2, false),
            Some((20, 12, 210, 46))
        );
    }
}
