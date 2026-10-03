//! The settings part of the desktop portal, served by matinee on the
//! session bus.
//!
//! GTK 4, libadwaita, and Qt 6 read the light or dark preference and their
//! interface settings through `org.freedesktop.portal.Settings`. Answering
//! it here makes those settings the same on every host, and is how a story
//! is put into its light or dark theme. Only the settings that affect a
//! recording are provided; anything else is reported as not found, which
//! apps treat as a default.
//!
//! Outside a sandbox, GTK reads its own settings from GSettings instead, so
//! the same `org.gnome.*` values are also written as a GSettings keyfile
//! (see [`Settings::keyfile`]).

use crate::story::Theme;
use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Value};

/// The bus name and path apps look for.
pub const NAME: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Clone, PartialEq)]
enum Setting {
    U32(u32),
    Str(&'static str),
    Bool(bool),
}

impl Setting {
    fn value(&self) -> OwnedValue {
        match self {
            Setting::U32(v) => OwnedValue::from(*v),
            Setting::Str(v) => Value::from(*v)
                .try_into()
                .expect("a string is a plain value"),
            Setting::Bool(v) => OwnedValue::from(*v),
        }
    }
}

pub struct Settings {
    values: Vec<(&'static str, &'static str, Setting)>,
}

impl Settings {
    pub fn new(theme: Theme) -> Settings {
        let dark = theme == Theme::Dark;
        Settings {
            values: vec![
                // 1 prefers dark, 2 prefers light.
                (
                    "org.freedesktop.appearance",
                    "color-scheme",
                    Setting::U32(if dark { 1 } else { 2 }),
                ),
                ("org.freedesktop.appearance", "contrast", Setting::U32(0)),
                (
                    "org.gnome.desktop.interface",
                    "color-scheme",
                    Setting::Str(if dark { "prefer-dark" } else { "prefer-light" }),
                ),
                // A blinking caret keeps changing the screen.
                (
                    "org.gnome.desktop.interface",
                    "cursor-blink",
                    Setting::Bool(false),
                ),
                (
                    "org.gnome.desktop.interface",
                    "gtk-theme",
                    Setting::Str("Adwaita"),
                ),
                (
                    "org.gnome.desktop.interface",
                    "icon-theme",
                    Setting::Str("Adwaita"),
                ),
                (
                    "org.gnome.desktop.interface",
                    "font-antialiasing",
                    Setting::Str("grayscale"),
                ),
                (
                    "org.gnome.desktop.interface",
                    "font-hinting",
                    Setting::Str("slight"),
                ),
            ],
        }
    }

    /// The `org.gnome.*` settings in the format of GSettings' keyfile
    /// backend, for `~/.config/glib-2.0/settings/keyfile`.
    pub fn keyfile(&self) -> String {
        let mut out = String::new();
        let mut section = "";
        for (ns, key, value) in &self.values {
            if !ns.starts_with("org.gnome.") {
                continue;
            }
            if *ns != section {
                section = ns;
                out.push_str(&format!("[{}]\n", ns.replace('.', "/")));
            }
            let text = match value {
                Setting::U32(v) => format!("uint32 {v}"),
                Setting::Str(v) => format!("'{v}'"),
                Setting::Bool(v) => v.to_string(),
            };
            out.push_str(&format!("{key}={text}\n"));
        }
        out
    }

    fn find(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.values
            .iter()
            .find(|(ns, k, _)| *ns == namespace && *k == key)
            .map(|(_, _, v)| v.value())
            .ok_or_else(|| PortalError::NotFound(format!("{namespace} {key}")))
    }
}

/// Whether `namespace` matches one of the requested patterns. A pattern
/// ending in `*` matches by prefix; an empty list or empty pattern
/// matches everything.
fn requested(namespace: &str, patterns: &[String]) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => namespace.starts_with(prefix),
            None => p.is_empty() || p == namespace,
        })
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
        for (ns, key, value) in &self.values {
            if requested(ns, &namespaces) {
                out.entry((*ns).to_owned())
                    .or_default()
                    .insert((*key).to_owned(), value.value());
            }
        }
        out
    }

    fn read_one(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.find(namespace, key)
    }

    /// The first version of this method wraps the value in a second variant.
    fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        let value = self.find(namespace, key)?;
        Ok(Value::Value(Box::new(value.into()))
            .try_into()
            .expect("a nested plain value is a plain value"))
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_patterns() {
        assert!(requested("org.gnome.desktop.interface", &[]));
        assert!(requested(
            "org.gnome.desktop.interface",
            &["org.gnome.*".into()]
        ));
        assert!(requested(
            "org.freedesktop.appearance",
            &["org.freedesktop.appearance".into()]
        ));
        assert!(!requested(
            "org.freedesktop.appearance",
            &["org.gnome.*".into()]
        ));
    }

    #[test]
    fn theme_sets_color_scheme() {
        let dark = Settings::new(Theme::Dark);
        let light = Settings::new(Theme::Light);
        let scheme = |s: &Settings| {
            u32::try_from(
                s.find("org.freedesktop.appearance", "color-scheme")
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(scheme(&dark), 1);
        assert_eq!(scheme(&light), 2);
        assert!(matches!(
            dark.find("org.example", "nope"),
            Err(PortalError::NotFound(_))
        ));
    }

    #[test]
    fn keyfile_holds_the_gnome_settings() {
        let keyfile = Settings::new(Theme::Dark).keyfile();
        assert!(
            keyfile.starts_with("[org/gnome/desktop/interface]\n"),
            "{keyfile}"
        );
        assert!(keyfile.contains("cursor-blink=false\n"), "{keyfile}");
        assert!(
            keyfile.contains("color-scheme='prefer-dark'\n"),
            "{keyfile}"
        );
        assert!(!keyfile.contains("appearance"), "{keyfile}");
    }
}
