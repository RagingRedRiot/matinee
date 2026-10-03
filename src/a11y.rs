//! Reading apps' accessibility trees over AT-SPI.
//!
//! On Wayland an app does not know where its windows are, so positions are
//! asked for relative to the window; the compositor knows where each window
//! is. A node's window is the top-level frame it sits under, matched to a
//! compositor window by title.

use crate::bus::Buses;
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
            let role: String = self
                .call(bus, p, ACCESSIBLE, "GetRoleName", &())
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
