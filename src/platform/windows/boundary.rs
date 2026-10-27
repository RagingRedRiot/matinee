//! The session boundary on Windows: a fresh profile, not isolation.
//!
//! Commands and apps get a throwaway home with its own `AppData`, a fresh
//! temp folder, a neutral user and computer name, and a clean environment,
//! so the obvious personal details stay out of recordings. They can still
//! read the host's files and use its network, as `--no-isolation` allows on
//! Linux, and matinee says so on every run. A private desktop and a demo
//! account are the planned next steps.

use crate::story::Story;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// Host variables passed through unchanged: where Windows and its programs
/// are, and what the machine is, none of which identify the user.
const PASSTHROUGH: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "ProgramData",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// The user and computer name the session reports.
const HOSTNAME: &str = "matinee";

pub struct Boundary {
    /// The storyboard's folder, where commands start.
    story_dir: PathBuf,
    /// Folders put at the front of `PATH`.
    path: Vec<PathBuf>,
    /// The caller's folder for results.
    work_dir: PathBuf,
    /// The throwaway home and temp folders.
    home: PathBuf,
    runtime: PathBuf,
}

impl Boundary {
    /// The boundary for `story`, with `work_dir` as its writable folder.
    pub fn for_story(story: &Story, work_dir: &Path) -> io::Result<Boundary> {
        let host_home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let dir = if story.dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &story.dir
        };
        let story_dir = plain(&fs::canonicalize(dir)?);
        let mut path = Vec::new();
        let declared = story
            .session
            .mounts
            .iter()
            .map(|m| ("session.mounts", m))
            .chain(story.session.path.iter().map(|m| ("session.path", m)));
        for (field, written) in declared {
            let resolved = match (written.strip_prefix('~'), &host_home) {
                (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                    home.join(rest.trim_start_matches(['/', '\\']))
                }
                _ => story_dir.join(written),
            };
            let resolved = fs::canonicalize(&resolved)
                .map_err(|e| io::Error::new(e.kind(), format!("{field}: {written}: {e}")))?;
            if field == "session.path" {
                path.push(plain(&resolved));
            }
        }
        let work_dir = plain(&fs::canonicalize(work_dir)?);
        let session = work_dir.join("session");
        Ok(Boundary {
            story_dir,
            path,
            home: session.join("home"),
            runtime: session.join("run"),
            work_dir,
        })
    }

    /// There is no isolation to leave out on Windows.
    pub fn without_isolation(self) -> io::Result<Boundary> {
        Ok(self)
    }

    /// Whether the work runs isolated from the host: never, yet.
    pub fn is_isolated(&self) -> bool {
        false
    }

    /// The session's home folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The session's temp folder.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime
    }

    /// Where results go, to come back to the caller.
    pub fn work(&self) -> &Path {
        &self.work_dir
    }

    /// The storyboard's folder as seen inside.
    pub fn story_dir(&self) -> &Path {
        &self.story_dir
    }

    /// The whole environment of a command inside: the throwaway profile,
    /// a neutral identity, `PATH` with the storyboard's `session.path`
    /// first, and the host's system locations.
    pub fn env(&self) -> Vec<(String, String)> {
        let home = self.home.display().to_string();
        let runtime = self.runtime.display().to_string();
        let path = self
            .path
            .iter()
            .map(|p| p.display().to_string())
            .chain(std::env::var("PATH").ok())
            .collect::<Vec<_>>()
            .join(";");
        let mut env: Vec<(String, String)> = [
            ("USERPROFILE", home.clone()),
            ("HOME", home.clone()),
            ("APPDATA", format!("{home}\\AppData\\Roaming")),
            ("LOCALAPPDATA", format!("{home}\\AppData\\Local")),
            ("TEMP", runtime.clone()),
            ("TMP", runtime),
            ("USERNAME", HOSTNAME.into()),
            ("USERDOMAIN", HOSTNAME.into()),
            ("COMPUTERNAME", HOSTNAME.into()),
            ("PATH", path),
            ("TERM", "xterm-256color".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        for name in PASSTHROUGH {
            if let Ok(value) = std::env::var(name) {
                env.push(((*name).to_owned(), value));
            }
        }
        env
    }

    /// A command to run inside, with a clean environment, starting in the
    /// storyboard's folder.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .envs(self.env())
            .current_dir(&self.story_dir);
        cmd
    }

    /// Start `program` with `args` on the session's private desktop, with
    /// `env` added to the boundary's environment, as a launched app.
    pub fn spawn_app(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> io::Result<super::Child> {
        let mut environment = self.env();
        environment.extend(env.iter().cloned());
        super::proc::spawn_on_desktop(super::DESKTOP, program, args, &environment, &self.story_dir)
    }

    /// A command that runs `script` in the session's shell, as
    /// [`Boundary::command`] would run a program.
    pub fn shell(&self, script: &str) -> Command {
        let mut cmd = self.command(super::SHELL);
        cmd.args(["-NoLogo", "-NoProfile", "-Command", script]);
        cmd
    }

    /// Run `inside` with a fresh profile and return its exit code.
    pub fn run(&self, inside: impl FnOnce() -> i32) -> io::Result<i32> {
        for dir in [
            self.home.join("AppData\\Roaming"),
            self.home.join("AppData\\Local"),
            self.runtime.clone(),
        ] {
            fs::create_dir_all(&dir)?;
        }
        eprintln!(
            "matinee: warning: sessions on Windows are not isolated: commands get a fresh \
             profile, but can read your files and use the network"
        );
        Ok(inside())
    }
}

/// `path` without the `\\?\` prefix that `canonicalize` adds, which shells
/// and programs do not all accept.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => PathBuf::from(format!(r"\\{rest}")),
        None => PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)),
    }
}
