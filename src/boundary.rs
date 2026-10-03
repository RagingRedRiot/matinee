//! The isolation boundary every story runs in.
//!
//! matinee builds it directly from Linux namespaces: a user namespace that
//! maps only the caller's own user and group, plus private mount, PID,
//! network, IPC, and hostname namespaces. Inside, the filesystem is a fresh
//! root holding read-only `/usr`, a short list of `/etc` entries, synthetic
//! identity files, the project at `/project` (read-only), any extra paths
//! the storyboard declares (read-only, at their host paths), and a writable
//! `/work` for results. `/home`, `/tmp`, `/run`, and `/var` are empty and
//! private.
//!
//! The project is the nearest folder above the storyboard that holds a
//! `.git`. Without one, or when that folder is the home folder or `/`, only
//! the storyboard's own folder is shown, so the boundary never quietly
//! widens to the whole home. The only network is a loopback interface shared by
//! everything inside, so a terminal command can reach a GUI app.
//!
//! The work runs as PID 1 of the new PID namespace. When it returns, the
//! kernel stops every process left inside, including daemons that detached.
//!
//! No step needs privileges, but the kernel must allow unprivileged user
//! namespaces.
//!
//! Where it does not, as on some CI runners, a boundary can be made
//! without isolation. Stories then run on the host as themselves: commands
//! and apps still get a fresh home, runtime folder, and environment, and
//! the session's own display and buses, so every step and recording works
//! the same, but they can read the host's files and use its network. The
//! work runs in a child that adopts every orphaned process, and whatever is
//! left when it returns is stopped, as PID 1 would.

use crate::story::Story;
use std::{
    ffi::CString,
    fs, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

/// The home directory inside the boundary.
pub const HOME: &str = "/home/matinee";
/// Where the project appears inside.
pub const PROJECT: &str = "/project";
/// The writable directory whose contents come back to the caller.
pub const WORK: &str = "/work";
const HOSTNAME: &str = "matinee";
const MACHINE_ID: &str = "6d6174696e656500000000000000000a";

/// System configuration that apps need to start, shown read-only. Entries
/// the host lacks are skipped.
const ETC: &[&str] = &[
    "alternatives",
    "bash.bashrc",
    "dconf",
    "fonts",
    "gtk-3.0",
    "gtk-4.0",
    "inputrc",
    "ld.so.cache",
    "ld.so.conf",
    "ld.so.conf.d",
    "mime.types",
    "nsswitch.conf",
    "os-release",
    "profile",
    "profile.d",
    "terminfo",
    "X11",
    "xdg",
];

/// Top-level directories that merged-`/usr` systems make symlinks.
const USR_LINKS: &[&str] = &["bin", "sbin", "lib", "lib32", "lib64", "libx32"];

/// Device nodes shared from the host.
const DEVICES: &[&str] = &["null", "zero", "full", "random", "urandom", "tty"];

pub struct Boundary {
    /// Host folder shown read-only at [`PROJECT`].
    project_dir: PathBuf,
    /// The storyboard's folder as seen inside, where commands start.
    story_dir: PathBuf,
    /// Extra host paths shown read-only at the same path.
    mounts: Vec<PathBuf>,
    /// Folders put at the front of `PATH`, also among `mounts`.
    path: Vec<PathBuf>,
    /// Host folder shown writable at [`WORK`].
    work_dir: PathBuf,
    /// Whether the work runs inside namespaces.
    isolated: bool,
    /// The home, runtime, and work folders, as the session sees them.
    home: PathBuf,
    runtime: PathBuf,
    work: PathBuf,
}

impl Boundary {
    /// The boundary for `story`, with `work_dir` as its writable folder.
    pub fn for_story(story: &Story, work_dir: &Path) -> io::Result<Boundary> {
        let home = std::env::var_os("HOME").map(fs::canonicalize).transpose()?;
        let dir = if story.dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &story.dir
        };
        let story_dir = fs::canonicalize(dir)?;
        let too_wide = |p: &Path| p == Path::new("/") || Some(p) == home.as_deref();
        if too_wide(&story_dir) {
            return Err(io::Error::other(format!(
                "the storyboard is directly in {}, which would show all of it to the \
                 session; move it into a project folder",
                story_dir.display()
            )));
        }
        let project_dir = story_dir
            .ancestors()
            .find(|d| d.join(".git").exists())
            .filter(|d| !too_wide(d))
            .unwrap_or(&story_dir)
            .to_owned();
        let inside = Path::new(PROJECT).join(story_dir.strip_prefix(&project_dir).unwrap());

        let mut mounts = Vec::new();
        let mut path_dirs = Vec::new();
        let declared = story
            .session
            .mounts
            .iter()
            .map(|m| ("session.mounts", m))
            .chain(story.session.path.iter().map(|m| ("session.path", m)));
        for (field, written) in declared {
            let path = match (written.strip_prefix('~'), &home) {
                (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
                    home.join(rest.trim_start_matches('/'))
                }
                _ => story_dir.join(written),
            };
            let path = fs::canonicalize(&path)
                .map_err(|e| io::Error::new(e.kind(), format!("{field}: {written}: {e}")))?;
            if too_wide(&path) {
                return Err(io::Error::other(format!(
                    "{field}: {written} would show all of {} to the session",
                    path.display()
                )));
            }
            if field == "session.path" {
                path_dirs.push(path.clone());
            }
            mounts.push(path);
        }

        Ok(Boundary {
            project_dir,
            story_dir: inside,
            mounts,
            path: path_dirs,
            work_dir: fs::canonicalize(work_dir)?,
            isolated: true,
            home: HOME.into(),
            runtime: format!("/run/user/{}", unsafe { libc::getuid() }).into(),
            work: WORK.into(),
        })
    }

    /// The same boundary without isolation: the work runs on the host, with
    /// its home and runtime folders inside the work folder.
    pub fn without_isolation(mut self) -> io::Result<Boundary> {
        let session = self.work_dir.join("session");
        let runtime = session.join("run");
        // Sockets live in the runtime folder, and a socket's path must fit
        // in 108 bytes; the longest matinee makes adds about 30.
        if runtime.as_os_str().len() > 70 {
            return Err(io::Error::other(format!(
                "{} is too long a path for the session's sockets; set TMPDIR to a \
                 shorter folder",
                runtime.display()
            )));
        }
        self.story_dir = self.project_dir.join(
            self.story_dir
                .strip_prefix(PROJECT)
                .unwrap_or(&self.story_dir),
        );
        self.home = session.join("home");
        self.runtime = runtime;
        self.work = self.work_dir.clone();
        self.isolated = false;
        Ok(self)
    }

    /// Whether the work runs inside namespaces.
    pub fn is_isolated(&self) -> bool {
        self.isolated
    }

    /// The session's home folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The session's runtime folder, for its display and bus sockets.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime
    }

    /// Where results go, to come back to the caller.
    pub fn work(&self) -> &Path {
        &self.work
    }

    /// `PATH` inside: the storyboard's `session.path`, then the system's.
    fn search_path(&self) -> String {
        self.path
            .iter()
            .map(|p| p.display().to_string())
            .chain(["/usr/local/bin", "/usr/bin", "/bin"].map(String::from))
            .collect::<Vec<_>>()
            .join(":")
    }

    /// The storyboard's folder as seen inside.
    pub fn story_dir(&self) -> &Path {
        &self.story_dir
    }

    /// A command to run inside, with a clean environment, starting in the
    /// storyboard's folder.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        let home = self.home.display();
        cmd.env_clear()
            .current_dir(&self.story_dir)
            .env("HOME", &self.home)
            .env("USER", HOSTNAME)
            .env("LOGNAME", HOSTNAME)
            .env("PATH", self.search_path())
            .env("LANG", "C.UTF-8")
            .env("TZ", "UTC")
            .env("TERM", "xterm-256color")
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CONFIG_HOME", format!("{home}/.config"))
            .env("XDG_DATA_HOME", format!("{home}/.local/share"))
            .env("XDG_CACHE_HOME", format!("{home}/.cache"))
            .env("XDG_STATE_HOME", format!("{home}/.local/state"));
        cmd
    }

    /// Run `inside` within a new boundary and return its exit code.
    ///
    /// This forks, so the calling process must have only one thread.
    pub fn run(&self, inside: impl FnOnce() -> i32) -> io::Result<i32> {
        if thread_count()? != 1 {
            return Err(io::Error::other(
                "the isolation boundary must be created before any threads start",
            ));
        }
        if !self.isolated {
            return self.run_on_host(inside);
        }
        let exe = std::env::current_exe()?;
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };

        // Setup errors travel back through this pipe; a clean setup closes
        // it without writing.
        let mut fds = [0; 2];
        check(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
        let [read_end, write_end] = fds;

        match check(unsafe { libc::fork() })? {
            0 => {
                unsafe { libc::close(read_end) };
                let code = match enter_namespaces(uid, gid) {
                    Ok(()) => run_init(
                        write_end,
                        || {
                            build_root(self, &exe, uid, gid)?;
                            bring_up_loopback()?;
                            set_hostname()
                        },
                        inside,
                    ),
                    Err(e) => report(
                        write_end,
                        &format!("creating namespaces: {e}{}", userns_hint(&e)),
                    ),
                };
                unsafe { libc::_exit(code) }
            }
            child => {
                unsafe { libc::close(write_end) };
                let message = read_all(read_end);
                let status = wait(child)?;
                if !message.is_empty() {
                    return Err(io::Error::other(message));
                }
                Ok(status)
            }
        }
    }
}

impl Boundary {
    /// Run `inside` on the host, in a child that adopts orphaned processes,
    /// with a fresh home and runtime folder; stop whatever is left after.
    fn run_on_host(&self, inside: impl FnOnce() -> i32) -> io::Result<i32> {
        for dir in [&self.home, &self.runtime] {
            match fs::remove_dir_all(dir) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        match check(unsafe { libc::fork() })? {
            0 => {
                let code = match set_parent_death_signal()
                    .and_then(|()| check(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }))
                {
                    Ok(_) => {
                        unsafe { libc::umask(0o022) };
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(inside))
                            .unwrap_or(101)
                    }
                    Err(e) => {
                        eprintln!("matinee: starting the session: {e}");
                        125
                    }
                };
                stop_descendants();
                unsafe { libc::_exit(code) }
            }
            child => wait(child),
        }
    }
}

/// Stop every process this one has adopted or started, and their children,
/// which come to this process as their parents stop.
fn stop_descendants() {
    let me = std::process::id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let children = children_of(me);
        if children.is_empty() || std::time::Instant::now() > deadline {
            break;
        }
        for pid in children {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        // Collect the stopped ones, so they are not found again.
        let mut status = 0;
        while unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) } > 0 {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The processes whose parent is `parent`.
fn children_of(parent: u32) -> Vec<libc::pid_t> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    entries
        .filter_map(|e| {
            let pid: libc::pid_t = e.ok()?.file_name().to_str()?.parse().ok()?;
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // The command name is in parentheses and may hold spaces; the
            // parent's PID is the second field after it.
            let rest = &stat[stat.rfind(')')? + 1..];
            let ppid: u32 = rest.split_whitespace().nth(1)?.parse().ok()?;
            (ppid == parent).then_some(pid)
        })
        .collect()
}

fn userns_hint(e: &io::Error) -> &'static str {
    if e.raw_os_error() == Some(libc::EPERM) || e.raw_os_error() == Some(libc::EACCES) {
        "; this system does not allow unprivileged user namespaces \
         (on Ubuntu, see kernel.apparmor_restrict_unprivileged_userns), \
         or pass --no-isolation to run without them"
    } else {
        ""
    }
}

/// In the first child: create the namespaces and map our own IDs. The next
/// process forked becomes PID 1 inside.
fn enter_namespaces(uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    set_parent_death_signal()?;
    check(unsafe {
        libc::unshare(
            libc::CLONE_NEWUSER
                | libc::CLONE_NEWNS
                | libc::CLONE_NEWPID
                | libc::CLONE_NEWNET
                | libc::CLONE_NEWIPC
                | libc::CLONE_NEWUTS,
        )
    })?;
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    Ok(())
}

/// Fork PID 1, set it up, run `inside` in it, and return its exit code.
fn run_init(
    errors: libc::c_int,
    setup: impl FnOnce() -> io::Result<()>,
    inside: impl FnOnce() -> i32,
) -> i32 {
    let pid = match check(unsafe { libc::fork() }) {
        Ok(pid) => pid,
        Err(e) => return report(errors, &format!("starting the session: {e}")),
    };
    if pid == 0 {
        let code = match set_parent_death_signal().and_then(|()| setup()) {
            Ok(()) => {
                unsafe { libc::close(errors) };
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(inside)).unwrap_or(101)
            }
            Err(e) => report(errors, &format!("building the isolated session: {e}")),
        };
        unsafe { libc::_exit(code) }
    }
    unsafe { libc::close(errors) };
    wait(pid).unwrap_or(125)
}

fn report(fd: libc::c_int, message: &str) -> i32 {
    unsafe {
        libc::write(fd, message.as_ptr().cast(), message.len());
        libc::close(fd);
    }
    125
}

/// Replace the root with one holding only what a story may see.
fn build_root(b: &Boundary, exe: &Path, uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    // Files and folders the session creates are not writable by others,
    // whatever the caller's umask; apps that check the folders above their
    // sockets refuse group-writable ones.
    unsafe { libc::umask(0o022) };
    // Keep every mount change inside this namespace.
    mount(None, "/", None, libc::MS_REC | libc::MS_PRIVATE, None)?;

    // Pivot onto a scratch tmpfs first, so the host root stays reachable at
    // /oldroot while the new root is filled in. The host's /tmp is still
    // visible there, under the scratch mount.
    tmpfs("/tmp", libc::MS_NOSUID | libc::MS_NODEV, Some("mode=0755"))?;
    fs::create_dir("/tmp/newroot")?;
    fs::create_dir("/tmp/oldroot")?;
    pivot_root("/tmp", "/tmp/oldroot")?;
    std::env::set_current_dir("/")?;
    tmpfs(
        "/newroot",
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=0755"),
    )?;

    let old = |p: &Path| Path::new("/oldroot").join(p.strip_prefix("/").unwrap_or(p));
    let new = |p: &str| Path::new("/newroot").join(p.trim_start_matches('/'));

    bind(&old(Path::new("/usr")), &new("usr"), true)?;
    for name in USR_LINKS {
        let host = old(Path::new(name));
        if let Ok(target) = fs::read_link(&host) {
            std::os::unix::fs::symlink(target, new(name))?;
        } else if host.is_dir() {
            bind(&host, &new(name), true)?;
        }
    }

    fs::create_dir(new("etc"))?;
    for name in ETC {
        let host = old(&Path::new("/etc").join(name));
        if host.exists() {
            bind(&host, &new(&format!("etc/{name}")), true)?;
        }
    }
    fs::write(new("etc/hostname"), format!("{HOSTNAME}\n"))?;
    fs::write(
        new("etc/hosts"),
        format!("127.0.0.1 localhost {HOSTNAME}\n::1 localhost\n"),
    )?;
    fs::write(new("etc/machine-id"), format!("{MACHINE_ID}\n"))?;
    fs::write(
        new("etc/passwd"),
        format!(
            "{HOSTNAME}:x:{uid}:{gid}:matinee:{HOME}:/bin/bash\nnobody:x:65534:65534::/nonexistent:/usr/sbin/nologin\n"
        ),
    )?;
    fs::write(
        new("etc/group"),
        format!("{HOSTNAME}:x:{gid}:\nnogroup:x:65534:\n"),
    )?;
    std::os::unix::fs::symlink("/usr/share/zoneinfo/UTC", new("etc/localtime"))?;

    for dir in ["tmp", "var", "run", "home"] {
        let flags = libc::MS_NOSUID | libc::MS_NODEV;
        let mode = if dir == "tmp" {
            "mode=1777"
        } else {
            "mode=0755"
        };
        fs::create_dir(new(dir))?;
        tmpfs(new(dir), flags, Some(mode))?;
    }
    fs::create_dir(new("var/tmp"))?;
    fs::set_permissions(new("var/tmp"), fs::Permissions::from_mode(0o1777))?;
    for dir in [HOME.to_owned(), format!("/run/user/{uid}")] {
        fs::create_dir_all(new(&dir))?;
        fs::set_permissions(new(&dir), fs::Permissions::from_mode(0o700))?;
    }

    build_dev(&new("dev"), &old(Path::new("/dev")))?;
    fs::create_dir(new("proc"))?;
    mount(
        Some(Path::new("proc")),
        new("proc"),
        Some("proc"),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )?;

    bind(&old(&b.project_dir), &new(PROJECT), true)?;
    bind(&old(&b.work_dir), &new(WORK), false)?;
    for path in &b.mounts {
        bind(&old(path), &new(&path.to_string_lossy()), true)?;
    }
    // matinee itself, for helpers that re-run it inside.
    bind(&old(exe), &new("/.matinee/matinee"), true)?;

    std::env::set_current_dir("/newroot")?;
    pivot_root(".", ".")?;
    check(unsafe { libc::umount2(c".".as_ptr(), libc::MNT_DETACH) })?;
    std::env::set_current_dir("/")?;
    // The root itself holds only mount points and identity files.
    mount(
        None,
        "/",
        None,
        libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
        None,
    )
}

fn build_dev(dev: &Path, host_dev: &Path) -> io::Result<()> {
    fs::create_dir(dev)?;
    tmpfs(dev, libc::MS_NOSUID, Some("mode=0755"))?;
    for name in DEVICES {
        bind(&host_dev.join(name), &dev.join(name), false)?;
    }
    fs::create_dir(dev.join("pts"))?;
    mount(
        Some(Path::new("devpts")),
        dev.join("pts"),
        Some("devpts"),
        libc::MS_NOSUID | libc::MS_NOEXEC,
        Some("newinstance,ptmxmode=0666,mode=620"),
    )?;
    std::os::unix::fs::symlink("pts/ptmx", dev.join("ptmx"))?;
    fs::create_dir(dev.join("shm"))?;
    tmpfs(
        dev.join("shm"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("mode=1777"),
    )?;
    for (link, target) in [
        ("fd", "/proc/self/fd"),
        ("stdin", "/proc/self/fd/0"),
        ("stdout", "/proc/self/fd/1"),
        ("stderr", "/proc/self/fd/2"),
    ] {
        std::os::unix::fs::symlink(target, dev.join(link))?;
    }
    Ok(())
}

/// Bind `src` at `dst`, creating `dst` to match. A read-only bind keeps the
/// flags the kernel locks on mounts from outside the namespace.
fn bind(src: &Path, dst: &Path, read_only: bool) -> io::Result<()> {
    let meta = fs::metadata(src)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", src.display())))?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
    } else {
        fs::write(dst, "")?;
    }
    mount(Some(src), dst, None, libc::MS_BIND | libc::MS_REC, None)?;
    if read_only {
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        check(unsafe { libc::statvfs(cpath(dst)?.as_ptr(), &mut stat) })?;
        let mut flags = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY;
        for (st, ms) in [
            (libc::ST_NOSUID, libc::MS_NOSUID),
            (libc::ST_NODEV, libc::MS_NODEV),
            (libc::ST_NOEXEC, libc::MS_NOEXEC),
            (libc::ST_NOATIME, libc::MS_NOATIME),
            (libc::ST_NODIRATIME, libc::MS_NODIRATIME),
            (libc::ST_RELATIME, libc::MS_RELATIME),
        ] {
            if stat.f_flag & st != 0 {
                flags |= ms;
            }
        }
        mount(None, dst, None, flags, None)?;
    }
    Ok(())
}

fn tmpfs(dst: impl AsRef<Path>, flags: libc::c_ulong, mode: Option<&str>) -> io::Result<()> {
    mount(Some(Path::new("tmpfs")), dst, Some("tmpfs"), flags, mode)
}

fn mount(
    src: Option<&Path>,
    dst: impl AsRef<Path>,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let dst = dst.as_ref();
    let src = src.map(cpath).transpose()?;
    let fstype = fstype.map(|s| CString::new(s).unwrap());
    let data = data.map(|s| CString::new(s).unwrap());
    let ptr = |s: &Option<CString>| s.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    check(unsafe {
        libc::mount(
            ptr(&src),
            cpath(dst)?.as_ptr(),
            ptr(&fstype),
            flags,
            ptr(&data).cast(),
        )
    })
    .map(drop)
    .map_err(|e| io::Error::new(e.kind(), format!("mount {}: {e}", dst.display())))
}

fn pivot_root(new_root: &str, put_old: &str) -> io::Result<()> {
    let (a, b) = (
        CString::new(new_root).unwrap(),
        CString::new(put_old).unwrap(),
    );
    check(unsafe { libc::syscall(libc::SYS_pivot_root, a.as_ptr(), b.as_ptr()) } as libc::c_int)
        .map(drop)
        .map_err(|e| io::Error::new(e.kind(), format!("pivot_root: {e}")))
}

fn bring_up_loopback() -> io::Result<()> {
    let sock =
        check(unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) })?;
    let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
    for (dst, src) in req.ifr_name.iter_mut().zip(b"lo\0") {
        *dst = *src as libc::c_char;
    }
    let result = check(unsafe { libc::ioctl(sock, libc::SIOCGIFFLAGS, &mut req) }).and_then(|_| {
        unsafe { req.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short };
        check(unsafe { libc::ioctl(sock, libc::SIOCSIFFLAGS, &req) })
    });
    unsafe { libc::close(sock) };
    result
        .map(drop)
        .map_err(|e| io::Error::new(e.kind(), format!("loopback: {e}")))
}

fn set_hostname() -> io::Result<()> {
    check(unsafe { libc::sethostname(HOSTNAME.as_ptr().cast(), HOSTNAME.len()) }).map(drop)
}

fn set_parent_death_signal() -> io::Result<()> {
    check(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) }).map(drop)
}

fn thread_count() -> io::Result<usize> {
    let status = fs::read_to_string("/proc/self/status")?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|n| n.trim().parse().ok())
        .ok_or_else(|| io::Error::other("no thread count in /proc/self/status"))
}

/// Wait for `pid` and return its exit code, or 128 plus the signal.
fn wait(pid: libc::pid_t) -> io::Result<i32> {
    let mut status = 0;
    loop {
        match check(unsafe { libc::waitpid(pid, &mut status, 0) }) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    })
}

fn read_all(fd: libc::c_int) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        } else if n == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break;
        }
    }
    unsafe { libc::close(fd) };
    String::from_utf8_lossy(&out).into_owned()
}

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::other("path contains a NUL byte"))
}

fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}
