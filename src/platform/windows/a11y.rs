//! Accessibility on Windows is not read yet: no app publishes a tree, so
//! every semantic target fails with the usual diagnostic.

use crate::platform::Node;

pub struct A11y;

/// An app with an accessibility tree.
#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub pid: u32,
}

impl A11y {
    /// Apps that have published a tree: none yet.
    pub fn apps(&self) -> Vec<App> {
        vec![]
    }

    pub fn nodes(&self, _app: &App, _all: bool) -> Vec<Node> {
        vec![]
    }
}
