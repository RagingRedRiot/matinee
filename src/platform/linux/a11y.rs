//! Reading apps' accessibility trees over AT-SPI.
//!
//! On Wayland an app does not know where its windows are, so positions are
//! asked for relative to the window; the compositor knows where each window
//! is. A node's window is the top-level frame it sits under, matched to a
//! compositor window by title.

use super::bus::Buses;
use std::{collections::HashSet, io, time::Duration};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const TEXT: &str = "org.a11y.atspi.Text";
const VALUE: &str = "org.a11y.atspi.Value";
/// Roles whose contents are read: fields a person types into or reads
/// from. Password fields are never read.
const TEXT_ROLES: &[&str] = &[
    "text box",
    "text",
    "entry",
    "editbar",
    "terminal",
    "paragraph",
    "document text",
];
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const COORD_WINDOW: u32 = 1;
/// AT-SPI's role names, by role number, for apps that report a role but
/// not its name (AccessKit, used by egui and others, reports only the
/// number).
const ROLE_NAMES: [&str; 130] = [
    "invalid",
    "accelerator label",
    "alert",
    "animation",
    "arrow",
    "calendar",
    "canvas",
    "check box",
    "check menu item",
    "color chooser",
    "column header",
    "combo box",
    "date editor",
    "desktop icon",
    "desktop frame",
    "dial",
    "dialog",
    "directory pane",
    "drawing area",
    "file chooser",
    "filler",
    "focus traversable",
    "font chooser",
    "frame",
    "glass pane",
    "html container",
    "icon",
    "image",
    "internal frame",
    "label",
    "layered pane",
    "list",
    "list item",
    "menu",
    "menu bar",
    "menu item",
    "option pane",
    "page tab",
    "page tab list",
    "panel",
    "password text",
    "popup menu",
    "progress bar",
    "push button",
    "radio button",
    "radio menu item",
    "root pane",
    "row header",
    "scroll bar",
    "scroll pane",
    "separator",
    "slider",
    "spin button",
    "split pane",
    "status bar",
    "table",
    "table cell",
    "table column header",
    "table row header",
    "tearoff menu item",
    "terminal",
    "text",
    "toggle button",
    "tool bar",
    "tool tip",
    "tree",
    "tree table",
    "unknown",
    "viewport",
    "window",
    "extended",
    "header",
    "footer",
    "paragraph",
    "ruler",
    "application",
    "autocomplete",
    "editbar",
    "embedded",
    "entry",
    "chart",
    "caption",
    "document frame",
    "heading",
    "page",
    "section",
    "redundant object",
    "form",
    "link",
    "input method window",
    "table row",
    "tree item",
    "document spreadsheet",
    "document presentation",
    "document text",
    "document web",
    "document email",
    "comment",
    "list box",
    "grouping",
    "image map",
    "notification",
    "info bar",
    "level bar",
    "title bar",
    "block quote",
    "audio",
    "video",
    "definition",
    "article",
    "landmark",
    "log",
    "marquee",
    "math",
    "rating",
    "timer",
    "static",
    "math fraction",
    "math root",
    "subscript",
    "superscript",
    "description list",
    "description term",
    "description value",
    "footnote",
    "content deletion",
    "content insertion",
    "mark",
    "suggestion",
    "push button menu",
];

/// Bit positions in the AT-SPI state set.
const STATE_FOCUSED: u32 = 12;
const STATE_SHOWING: u32 = 25;
const STATE_VISIBLE: u32 = 30;
/// Stop walking a tree after this many nodes.
const MAX_NODES: usize = 5000;

pub struct A11y {
    conn: zbus::blocking::Connection,
}

/// An app with an accessibility tree.
#[derive(Debug, Clone)]
pub struct App {
    pub bus: String,
    pub path: OwnedObjectPath,
    pub name: String,
    pub pid: u32,
}

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

impl A11y {
    pub fn connect(buses: &Buses) -> io::Result<A11y> {
        let conn = zbus::blocking::connection::Builder::address(buses.a11y.as_str())
            .map(|b| b.method_timeout(Duration::from_secs(2)))
            .and_then(|b| b.build())
            .map_err(|e| io::Error::other(format!("accessibility bus: {e}")))?;
        Ok(A11y { conn })
    }

    fn call<B, R>(
        &self,
        bus: &str,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let reply = self
            .conn
            .call_method(Some(bus), path, Some(iface), method, body)?;
        reply.body().deserialize()
    }

    fn property(&self, bus: &str, path: &str, name: &str) -> String {
        self.call::<_, OwnedValue>(
            bus,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(ACCESSIBLE, name),
        )
        .ok()
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
    }

    fn children(&self, bus: &str, path: &str) -> Vec<(String, OwnedObjectPath)> {
        self.call(bus, path, ACCESSIBLE, "GetChildren", &())
            .unwrap_or_default()
    }

    /// Apps that have published a tree.
    pub fn apps(&self) -> Vec<App> {
        self.children(REGISTRY, ROOT)
            .into_iter()
            .map(|(bus, path)| App {
                name: self.property(&bus, path.as_str(), "Name"),
                pid: self
                    .call(
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "GetConnectionUnixProcessID",
                        &(bus.as_str(),),
                    )
                    .unwrap_or(0),
                bus,
                path,
            })
            .collect()
    }

    /// The app's windows and everything under them, in tree order. Nodes
    /// that are not showing, and their subtrees, are left out unless `all`.
    pub fn nodes(&self, app: &App, all: bool) -> Vec<Node> {
        let bus = app.bus.as_str();
        let mut out = vec![];
        let mut seen = HashSet::new();
        // Depth-first, in order: push children reversed.
        let mut stack: Vec<(OwnedObjectPath, usize, String, Option<usize>)> = self
            .children(bus, app.path.as_str())
            .into_iter()
            .rev()
            .filter(|(b, _)| b == bus)
            .map(|(_, p)| (p, 0, String::new(), None))
            .collect();
        while let Some((path, depth, window, parent)) = stack.pop() {
            if !seen.insert(path.clone()) || seen.len() > MAX_NODES {
                continue;
            }
            let p = path.as_str();
            let states: Vec<u32> = self
                .call(bus, p, ACCESSIBLE, "GetState", &())
                .unwrap_or_default();
            let state = |bit: u32| {
                states
                    .get((bit / 32) as usize)
                    .is_some_and(|w| w & (1 << (bit % 32)) != 0)
            };
            let showing = state(STATE_SHOWING) && state(STATE_VISIBLE);
            if !showing && !all {
                continue;
            }
            let name = self.property(bus, p, "Name");
            let window = if depth == 0 { name.clone() } else { window };
            let index = out.len();
            for (child_bus, child) in self.children(bus, p).into_iter().rev() {
                if child_bus == bus {
                    stack.push((child, depth + 1, window.clone(), Some(index)));
                }
            }
            let role = self
                .call::<_, String>(bus, p, ACCESSIBLE, "GetRoleName", &())
                .ok()
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    let number: u32 = self.call(bus, p, ACCESSIBLE, "GetRole", &()).ok()?;
                    ROLE_NAMES.get(number as usize).map(|name| name.to_string())
                })
                .unwrap_or_default();
            let text = TEXT_ROLES
                .contains(&role.as_str())
                .then(|| {
                    self.call::<_, String>(bus, p, TEXT, "GetText", &(0i32, -1i32))
                        .ok()
                })
                .flatten();
            let value = (role == "slider")
                .then(|| {
                    let get = |name: &str| {
                        self.call::<_, OwnedValue>(
                            bus,
                            p,
                            "org.freedesktop.DBus.Properties",
                            "Get",
                            &(VALUE, name),
                        )
                        .ok()
                        .and_then(|v| f64::try_from(v).ok())
                    };
                    Some((
                        get("CurrentValue")?,
                        get("MinimumValue")?,
                        get("MaximumValue")?,
                    ))
                })
                .flatten();
            out.push(Node {
                role,
                text,
                value,
                extents: self
                    .call::<_, (i32, i32, i32, i32)>(
                        bus,
                        p,
                        COMPONENT,
                        "GetExtents",
                        &(COORD_WINDOW,),
                    )
                    .ok(),
                focused: state(STATE_FOCUSED),
                name,
                depth,
                parent,
                window,
                showing,
            });
        }
        out
    }
}

/// Node `i`'s position and size relative to its window, in logical points.
///
/// Toolkits that report device pixels (AccessKit) are recognized by their
/// window's root element measuring the window's size times the output
/// `scale`, in both directions, and scaled back down. Anything else, such
/// as a root still at an old size while the window is resized, is taken
/// as points.
pub fn in_points(
    nodes: &[Node],
    i: usize,
    window: (i32, i32),
    scale: i32,
) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = nodes[i].extents?;
    let near = |a: i32, b: i32| (a - b).abs() <= scale;
    // Some Chromium browsers report the top-level frame in points but the
    // document beneath it in device pixels. Use the nearest ancestor whose
    // size identifies its coordinate space, rather than only the root.
    let device = scale > 1
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
        assert_eq!(in_points(&nodes, 1, (1200, 720), 2), Some((10, 6, 105, 23)));
    }

    #[test]
    fn device_pixel_document_under_point_sized_frame() {
        let nodes = [
            node((0, 0, 1200, 800), None),
            node((0, 0, 2400, 1600), Some(0)),
            node((610, 654, 892, 100), Some(1)),
        ];
        assert_eq!(
            in_points(&nodes, 2, (1200, 800), 2),
            Some((305, 327, 446, 50))
        );
        assert_eq!(
            in_points(&nodes, 0, (1200, 800), 2),
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
        assert_eq!(in_points(&nodes, 1, (800, 600), 2), Some((20, 12, 210, 46)));
    }

    #[test]
    fn points_are_left_alone() {
        let nodes = [
            node((0, 0, 1200, 720), None),
            node((20, 12, 210, 46), Some(0)),
        ];
        assert_eq!(
            in_points(&nodes, 1, (1200, 720), 2),
            Some((20, 12, 210, 46))
        );
    }
}
