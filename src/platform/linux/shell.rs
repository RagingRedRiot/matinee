//! The shell behind the terminal pane: one bash on a pseudo-terminal for
//! the whole story, so `cd` and exported variables carry from one step to
//! the next.
//!
//! It does not read the terminal: matinee sends it each command over a
//! private pipe, and bash reports each exit status over another. The
//! command itself runs with the terminal as its input and output, so
//! programs behave as they would for a person, and its output is exactly
//! what appears.

use super::boundary::Boundary;
use std::{
    io::{self, BufRead, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
};

/// The loop bash runs: read a NUL-terminated command from fd 3, run it on
/// the terminal, and write its status to fd 4. Ctrl+C stops the command,
/// not the loop.
const LOOP: &str = r#"trap : INT
while IFS= read -r -d '' __matinee_cmd <&3; do
  eval "$__matinee_cmd" </dev/tty 3<&- 4>&-
  printf '%d\n' "$?" >&4
done"#;

pub struct Shell {
    master: std::fs::File,
    commands: std::fs::File,
    statuses: Receiver<i32>,
    child: Child,
}

impl Shell {
    /// Start a shell on a terminal of `rows` by `cols`, with `env` added
    /// to the boundary's environment. Everything it prints goes to
    /// `screen`.
    pub fn start(
        boundary: &Boundary,
        rows: u16,
        cols: u16,
        env: &[(String, String)],
        screen: Arc<Mutex<vt100::Parser>>,
    ) -> io::Result<Shell> {
        let (master, slave) = open_pty(rows, cols)?;
        let (command_read, command_write) = pipe()?;
        let (status_read, status_write) = pipe()?;
        let (command_fd, status_fd) = (command_read.as_raw_fd(), status_write.as_raw_fd());

        let mut cmd = boundary.command("bash");
        cmd.args(["--noprofile", "--norc", "-c", LOOP])
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        unsafe {
            cmd.pre_exec(move || {
                // Its own session, with the terminal as the controlling
                // terminal, so /dev/tty and Ctrl+C work.
                if libc::setsid() < 0
                    || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0
                    || libc::dup2(command_fd, 3) < 0
                    || libc::dup2(status_fd, 4) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("bash: {e}")))?;
        drop((command_read, status_write));

        let mut reader = std::fs::File::from(master.try_clone()?);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            // Ends with an error once every writer has gone.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                screen.lock().unwrap().process(&buf[..n]);
            }
        });
        let (status_tx, statuses) = mpsc::channel();
        let status_reader = io::BufReader::new(std::fs::File::from(status_read));
        std::thread::spawn(move || {
            for line in status_reader.lines() {
                let Ok(line) = line else { break };
                if let Ok(status) = line.trim().parse() {
                    if status_tx.send(status).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Shell {
            master: std::fs::File::from(master),
            commands: std::fs::File::from(command_write),
            statuses,
            child,
        })
    }

    /// Start a command. Its status arrives through [`Shell::finished`].
    pub fn run(&mut self, command: &str) -> io::Result<()> {
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(0);
        self.commands.write_all(&bytes)
    }

    /// The exit status of the last command, once it has finished.
    pub fn finished(&mut self) -> io::Result<Option<i32>> {
        match self.statuses.try_recv() {
            Ok(status) => Ok(Some(status)),
            Err(mpsc::TryRecvError::Empty) => match self.child.try_wait()? {
                Some(status) => Err(io::Error::other(format!(
                    "the terminal's shell exited ({status})"
                ))),
                None => Ok(None),
            },
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(io::Error::other("the terminal's shell stopped"))
            }
        }
    }

    /// Send keys to whatever is running in the terminal.
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn open_pty(rows: u16, cols: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (-1, -1);
    let size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok((master, slave))
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
