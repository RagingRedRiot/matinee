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

use std::{io, mem, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess,
            PROCESS_INFORMATION, ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
        },
    },
};

/// `GetExitCodeProcess` reports this code while the process is still running.
const STILL_ACTIVE: u32 = 259;

/// A launched program: its process handle and id.
pub struct Child {
    handle: HANDLE,
    pid: u32,
}

impl Child {
    /// The process id.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// `Some(description)` once the process has exited, for a diagnostic.
    pub fn exited(&mut self) -> io::Result<Option<String>> {
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((code != STILL_ACTIVE).then(|| format!("exit code {code}")))
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

/// Start `program` with `args` on the desktop named `desktop`, in `job`,
/// with exactly `env` as its environment and `dir` as its working
/// directory.
pub fn spawn_on_desktop(
    desktop: &str,
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
    let mut desktop: Vec<u16> = desktop.encode_utf16().chain([0]).collect();
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
    startup.lpDesktop = desktop.as_mut_ptr();

    let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            ptr::null(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            // Held until it is in the job, so nothing it starts escapes.
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
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
    })
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
    use super::command_line;

    #[test]
    fn arguments_with_spaces_are_quoted() {
        assert_eq!(command_line("notepad.exe", &[]), "notepad.exe");
        assert_eq!(
            command_line("app.exe", &["a b".into(), "c".into()]),
            "app.exe \"a b\" c"
        );
    }
}
