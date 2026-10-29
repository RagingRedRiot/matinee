//! The session's D-Bus: a session bus and an accessibility bus, each a
//! private `dbus-daemon`, plus the system's AT-SPI registry.
//!
//! Apps find the accessibility bus through `AT_SPI_BUS_ADDRESS`, or by
//! asking the session bus, where matinee answers `org.a11y.Bus` and reports
//! accessibility as enabled through `org.a11y.Status`. Matinee also serves
//! the desktop portal's settings (see [`super::portal`]).
//!
//! Neither bus starts services on demand, so what runs in a session does
//! not depend on what the host has installed. `dbus-daemon` and
//! `at-spi2-registryd` come with any desktop that runs AT-SPI: at-spi2-core
//! starts its own accessibility bus with a D-Bus daemon.

use super::{boundary::Boundary, portal};
use crate::story::Theme;
use std::{
    io,
    path::Path,
    process::Child,
    time::{Duration, Instant},
};

/// Where distributions install the AT-SPI registry.
const REGISTRYD: &[&str] = &[
    "/usr/libexec/at-spi2-registryd",
    "/usr/lib/at-spi2-core/at-spi2-registryd",
    "/usr/lib/at-spi2/at-spi2-registryd",
    "/usr/lib/x86_64-linux-gnu/at-spi2-core/at-spi2-registryd",
    "/usr/lib/aarch64-linux-gnu/at-spi2-core/at-spi2-registryd",
    "/usr/lib64/at-spi2-core/at-spi2-registryd",
];

/// Configuration for both buses, with `{listen}` replaced by the address:
/// anyone in the session may own any name and send anything, and there are
/// no service directories, so nothing starts on demand.
const CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>{listen}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
  <limit name="max_incoming_bytes">1000000000</limit>
  <limit name="max_outgoing_bytes">1000000000</limit>
  <limit name="max_message_size">1000000000</limit>
</busconfig>
"#;

pub struct Buses {
    /// The session bus address, for `DBUS_SESSION_BUS_ADDRESS`.
    pub session: String,
    /// The accessibility bus address, for `AT_SPI_BUS_ADDRESS`.
    pub a11y: String,
    children: Vec<(&'static str, Child)>,
    /// Matinee's own connection on the session bus, serving `org.a11y.Bus`
    /// and the portal settings.
    _service: zbus::blocking::Connection,
}

impl Buses {
    /// Start both buses with sockets in `dir`, then the registry, with the
    /// portal reporting `theme`. Call this inside the boundary.
    pub fn start(boundary: &Boundary, dir: &Path, theme: Theme) -> io::Result<Buses> {
        let session_socket = dir.join("bus");
        let a11y_socket = dir.join("at-spi-bus");
        let session = format!("unix:path={}", session_socket.display());
        let a11y = format!("unix:path={}", a11y_socket.display());
        let mut children = vec![];

        for (name, file, address, socket) in [
            ("session bus", "bus.conf", &session, &session_socket),
            ("accessibility bus", "at-spi-bus.conf", &a11y, &a11y_socket),
        ] {
            let config = dir.join(file);
            std::fs::write(&config, CONFIG.replace("{listen}", address))?;
            let child = boundary
                .command("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--nopidfile", "--nosyslog"])
                .spawn()
                .map_err(|e| {
                    io::Error::new(e.kind(), format!("dbus-daemon: {e}; install dbus-daemon"))
                })?;
            children.push((name, child));
            wait(name, &mut children, || socket.exists())?;
        }

        let service = zbus::blocking::connection::Builder::address(session.as_str())
            .and_then(|b| b.name("org.a11y.Bus"))
            .and_then(|b| {
                b.serve_at(
                    "/org/a11y/bus",
                    A11yBus {
                        address: a11y.clone(),
                    },
                )
            })
            .and_then(|b| b.serve_at("/org/a11y/bus", A11yStatus))
            .and_then(|b| b.name(portal::NAME))
            .and_then(|b| b.serve_at(portal::PATH, portal::Settings::new(theme)))
            .and_then(|b| b.build())
            .map_err(|e| io::Error::other(format!("session services: {e}")))?;

        let program = REGISTRYD
            .iter()
            .find(|p| Path::new(p).exists())
            .ok_or_else(|| {
                io::Error::other("at-spi2-registryd was not found; install at-spi2-core")
            })?;
        let registry = boundary
            .command(program)
            .env("DBUS_SESSION_BUS_ADDRESS", &session)
            .env("AT_SPI_BUS_ADDRESS", &a11y)
            // It announces itself on every start; errors still show.
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{program}: {e}")))?;
        children.push(("at-spi2-registryd", registry));

        let probe = zbus::blocking::connection::Builder::address(a11y.as_str())
            .and_then(|b| b.build())
            .map_err(|e| io::Error::other(format!("accessibility bus: {e}")))?;
        wait("at-spi2-registryd", &mut children, || {
            probe
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "NameHasOwner",
                    &("org.a11y.atspi.Registry",),
                )
                .and_then(|reply| reply.body().deserialize::<bool>())
                .unwrap_or(false)
        })?;

        Ok(Buses {
            session,
            a11y,
            children,
            _service: service,
        })
    }

    /// Environment that points an app at these buses and turns on its
    /// accessibility support.
    pub fn env(&self) -> [(&str, &str); 6] {
        [
            ("DBUS_SESSION_BUS_ADDRESS", &self.session),
            ("AT_SPI_BUS_ADDRESS", &self.a11y),
            // Qt and Chromium publish their trees only when asked.
            ("QT_LINUX_ACCESSIBILITY_ALWAYS_ON", "1"),
            ("QT_ACCESSIBILITY", "1"),
            ("ACCESSIBILITY_ENABLED", "1"),
            ("GTK_A11Y", "atspi"),
        ]
    }

    /// Fail if a bus or the registry has stopped.
    pub fn check(&mut self) -> io::Result<()> {
        for (name, child) in &mut self.children {
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!("{name} exited ({status})")));
            }
        }
        Ok(())
    }
}

/// Wait until `ready`, failing early if one of `children` exits.
fn wait(
    what: &str,
    children: &mut [(&'static str, Child)],
    mut ready: impl FnMut() -> bool,
) -> io::Result<()> {
    let start = Instant::now();
    while !ready() {
        for (name, child) in children.iter_mut() {
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!("{name} exited ({status})")));
            }
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err(io::Error::other(format!("{what} did not start")));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

struct A11yBus {
    address: String,
}

#[zbus::interface(name = "org.a11y.Bus")]
impl A11yBus {
    fn get_address(&self) -> String {
        self.address.clone()
    }
}

struct A11yStatus;

#[zbus::interface(name = "org.a11y.Status")]
impl A11yStatus {
    #[zbus(property)]
    fn is_enabled(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn set_is_enabled(&mut self, _enabled: bool) {}

    /// Some apps, such as Firefox and Chromium, publish a full tree only
    /// when they believe a screen reader is running.
    #[zbus(property)]
    fn screen_reader_enabled(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn set_screen_reader_enabled(&mut self, _enabled: bool) {}
}
