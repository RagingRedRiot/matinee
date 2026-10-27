//! A launched program on the session's private desktop.
//!
//! The standard library's process spawn gives no way to say which desktop a
//! new process should use, and a new process otherwise opens its windows on
//! whichever desktop the launcher's process started on, not the private one
//! this thread moved to. So an app launch builds the process directly, with
//! the desktop named in its startup info, the way any launcher that targets
//! a specific desktop does. Everything else, the environment and working
//! directory, matches what the standard spawn would have used.
//!
//! Each launch also goes into the session's [`Job`], before it can run, so
//! whatever it starts in turn ends with the session too.
//!
//! A packaged (Store) app is not a program to start: Windows starts it,
//! given its app ID, on the user's own desktop and outside any job. So
//! [`activate`] asks the shell to, and keeps the process it started.

use std::{
    io, mem,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE},
    Storage::Packaging::Appx::GetPackageFamilyName,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject,
        },
        SystemInformation::GetSystemTimeAsFileTime,
        Threading::{
            CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
            ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
        },
    },
};

/// `GetExitCodeProcess` reports this code while the process is still running.
const STILL_ACTIVE: u32 = 259;

/// A launched program: its process handle and id, and the session's job
/// it went into (null for a packaged app, which Windows started outside).
pub struct Child {
    handle: HANDLE,
    pid: u32,
    job: HANDLE,
}

impl Child {
    /// The process id.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// `Some(description)` once the program has exited, for a diagnostic.
    /// A launcher that started the real app and left, as Edge does on the
    /// screen, has not: the program runs on while anything it started does.
    pub fn exited(&mut self) -> io::Result<Option<String>> {
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if code == STILL_ACTIVE || (!self.job.is_null() && job_holds_descendant(self.job, self.pid))
        {
            return Ok(None);
        }
        Ok(Some(format!("exit code {code}")))
    }

    /// Stop the process and reap it.
    pub fn kill(&mut self) {
        unsafe {
            TerminateProcess(self.handle, 1);
            WaitForSingleObject(self.handle, 2000);
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// A job that stops every process in it when its last handle closes: when
/// the session ends, or when matinee itself exits or is stopped.
pub struct Job(HANDLE);

impl Job {
    pub fn new() -> io::Result<Job> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }

    /// Stop every process in the job now.
    pub fn kill(&self) {
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Start `program` with `args` on the desktop named `desktop` (this
/// process's own when `None`), in `job`, with exactly `env` as its
/// environment and `dir` as its working directory.
pub fn spawn_on_desktop(
    desktop: Option<&str>,
    job: HANDLE,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    dir: &Path,
) -> io::Result<Child> {
    let mut command: Vec<u16> = command_line(program, args)
        .encode_utf16()
        .chain([0])
        .collect();
    let mut desktop: Option<Vec<u16>> = desktop.map(|d| d.encode_utf16().chain([0]).collect());
    let dir: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();

    // The environment block: each "KEY=VALUE\0", sorted, ended by a second
    // NUL, as CreateProcessW expects for a Unicode environment.
    let mut sorted: Vec<&(String, String)> = env.iter().collect();
    sorted.sort_by_key(|(k, _)| k.to_uppercase());
    let block: Vec<u16> = sorted
        .iter()
        .flat_map(|(k, v)| format!("{k}={v}\0").encode_utf16().collect::<Vec<_>>())
        .chain([0])
        .collect();

    let mut startup: STARTUPINFOW = unsafe { mem::zeroed() };
    startup.cb = mem::size_of::<STARTUPINFOW>() as u32;
    if let Some(desktop) = desktop.as_mut() {
        startup.lpDesktop = desktop.as_mut_ptr();
    }

    let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            ptr::null(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            // Held until it is in the job, so nothing it starts escapes. A
            // console program gets a console window of its own, where it
            // would otherwise share matinee's console and show nothing.
            CREATE_NEW_CONSOLE | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr() as *const _,
            dir.as_ptr(),
            &startup,
            &mut info,
        )
    };
    if ok == 0 {
        let e = io::Error::last_os_error();
        return Err(io::Error::new(e.kind(), format!("{program}: {e}")));
    }
    if unsafe { AssignProcessToJobObject(job, info.hProcess) } == 0 {
        let e = io::Error::last_os_error();
        unsafe {
            TerminateProcess(info.hProcess, 1);
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
        return Err(io::Error::new(
            e.kind(),
            format!("{program}: joining the session's job: {e}"),
        ));
    }
    unsafe {
        ResumeThread(info.hThread);
        CloseHandle(info.hThread);
    }
    Ok(Child {
        handle: info.hProcess,
        pid: info.dwProcessId,
        job,
    })
}

/// Whether a process in `job` descends from `root`, which may have exited.
fn job_holds_descendant(job: HANDLE, root: u32) -> bool {
    // The list's header is two counts; the ids follow, one per slot.
    let mut list = [0usize; 258];
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            list.as_mut_ptr() as *mut _,
            mem::size_of_val(&list) as u32,
            ptr::null_mut(),
        )
    };
    let count = list[0] >> 32;
    if ok == 0 || count == 0 {
        return false;
    }
    let parents = parents();
    list[1..=count.min(list.len() - 1)].iter().any(|&pid| {
        let mut pid = pid as u32;
        for _ in 0..64 {
            match parents.iter().find(|(p, _)| *p == pid) {
                Some(&(_, parent)) if parent == root => return true,
                Some(&(_, parent)) if parent != 0 && parent != pid => pid = parent,
                _ => return false,
            }
        }
        false
    })
}

/// Every process's id and its parent's, from one snapshot.
fn parents() -> Vec<(u32, u32)> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entry: PROCESSENTRY32W = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        out.push((entry.th32ProcessID, entry.th32ParentProcessID));
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    out
}

/// Whether `app` names a packaged app by its app ID,
/// `PackageFamilyName!AppId`, rather than a program.
pub fn is_app_id(app: &str) -> bool {
    app.contains('!') && !app.contains(['\\', '/']) && !app.to_lowercase().ends_with(".exe")
}

/// Ask the shell to start the packaged app `app_id`, as the Start menu
/// does, and keep the process it started: the first of the app's package to
/// start after the ask. Windows gives it its own environment and folder,
/// not the session's, and takes no arguments this way.
///
/// The shell rather than `IApplicationActivationManager`: an app that
/// matinee activates directly can crash as it starts (Calculator does, in
/// `Windows.UI.Xaml`), and one the shell starts does not.
pub fn activate(app_id: &str, args: &[String]) -> io::Result<Child> {
    if !args.is_empty() {
        return Err(io::Error::other(format!(
            "{app_id}: a packaged app takes no arguments from matinee"
        )));
    }
    let family = app_id.split('!').next().unwrap_or(app_id);
    let since = now();
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .status()
        .map_err(|e| io::Error::new(e.kind(), format!("{app_id}: asking the shell: {e}")))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(child) = started_since(family, since) {
            return Ok(child);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "{app_id}: no new process of it started; if it is already running, close it, \
                 since matinee drives only an app it started"
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The current time as a `FILETIME` count, to compare process start times.
fn now() -> u64 {
    let mut time: FILETIME = unsafe { mem::zeroed() };
    unsafe { GetSystemTimeAsFileTime(&mut time) };
    (time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64
}

/// A process of package `family` that started at or after `since`.
fn started_since(family: &str, since: u64) -> Option<Child> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: PROCESSENTRY32W = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more && found.is_none() {
        found = open_if_started(entry.th32ProcessID, family, since);
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    found
}

/// Process `pid`, if it is of package `family` and started at or after
/// `since`.
fn open_if_started(pid: u32, family: &str, since: u64) -> Option<Child> {
    let handle: HANDLE = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return None;
    }
    let child = Child {
        handle,
        pid,
        job: ptr::null_mut(),
    };
    let mut name = [0u16; 256];
    let mut len = name.len() as u32;
    if unsafe { GetPackageFamilyName(handle, &mut len, name.as_mut_ptr()) } != 0 {
        return None;
    }
    let name = String::from_utf16_lossy(&name[..(len as usize).saturating_sub(1)]);
    if !name.eq_ignore_ascii_case(family) {
        return None;
    }
    let zero: FILETIME = unsafe { mem::zeroed() };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let created = (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64;
    (created >= since).then_some(child)
}

/// A command line from a program and its arguments, each quoted if it holds
/// a space, as the C runtime parses them back.
fn command_line(program: &str, args: &[String]) -> String {
    let mut line = quote(program);
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line
}

fn quote(part: &str) -> String {
    if part.is_empty() || part.contains([' ', '\t', '"']) {
        format!("\"{}\"", part.replace('"', "\\\""))
    } else {
        part.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{command_line, is_app_id};

    #[test]
    fn arguments_with_spaces_are_quoted() {
        assert_eq!(command_line("notepad.exe", &[]), "notepad.exe");
        assert_eq!(
            command_line("app.exe", &["a b".into(), "c".into()]),
            "app.exe \"a b\" c"
        );
    }

    #[test]
    fn app_ids_are_told_from_programs() {
        assert!(is_app_id("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert!(!is_app_id("notepad.exe"));
        assert!(!is_app_id("C:\\Tools\\wow!.exe"));
        assert!(!is_app_id("tools/run!"));
    }
}
