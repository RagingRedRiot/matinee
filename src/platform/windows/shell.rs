//! The shell behind the terminal pane: one PowerShell on a pseudo console
//! for the whole story, so `cd` and variables carry from one step to the
//! next.
//!
//! It does not read the terminal: matinee sends it each command over a
//! loopback socket, and the shell reports each exit status back the same
//! way. The command itself runs with the pseudo console as its input and
//! output, so programs behave as they would for a person, and its output is
//! exactly what appears. The socket stands in for the extra file
//! descriptors bash gets on Linux, which a Windows process cannot inherit
//! by number.

use super::boundary::Boundary;
use std::{
    ffi::c_void,
    fs::File,
    hash::{BuildHasher, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::windows::io::FromRawHandle,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON},
        Pipes::CreatePipe,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
        },
    },
};

const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
/// `GetExitCodeProcess` reports this while the process runs.
const STILL_ACTIVE: u32 = 259;
/// How long PowerShell may take to start and call back.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The loop PowerShell runs: connect back to matinee, prove it is the
/// shell matinee started, then read each command (one base64 line), run
/// it, and write its status. `{port}` and `{token}` are filled in.
const LOOP: &str = r#"$ErrorActionPreference = 'Continue'
$__client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', {port})
$__stream = $__client.GetStream()
$__utf8 = New-Object System.Text.UTF8Encoding($false)
$__reader = New-Object System.IO.StreamReader($__stream, $__utf8)
$__writer = New-Object System.IO.StreamWriter($__stream, $__utf8)
$__writer.AutoFlush = $true
$__writer.WriteLine('{token}')
while ($null -ne ($__line = $__reader.ReadLine())) {
  $__command = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($__line))
  $global:LASTEXITCODE = 0
  $__ok = $true
  try { & ([scriptblock]::Create($__command)); $__ok = $? } catch { Write-Host $_.Exception.Message -ForegroundColor Red; $__ok = $false }
  $__status = if ($global:LASTEXITCODE) { $global:LASTEXITCODE } elseif ($__ok) { 0 } else { 1 }
  $__writer.WriteLine([string]$__status)
}
"#;

pub struct Shell {
    /// The pseudo console's input: keys for whatever runs in it.
    input: File,
    commands: TcpStream,
    statuses: Receiver<i32>,
    process: HANDLE,
    console: HPCON,
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
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token = format!("{:016x}", {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u32(std::process::id());
            hasher.finish()
        });
        let script = LOOP
            .replace("{port}", &port.to_string())
            .replace("{token}", &token);
        let encoded = base64(
            &script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        );
        let command_line = format!(
            "{} -NoLogo -NoProfile -EncodedCommand {encoded}",
            super::SHELL
        );
        let mut environment: Vec<(String, String)> = boundary.env();
        environment.extend(env.iter().cloned());
        let (console, input, output) = open_console(rows, cols)?;
        let process = match spawn(&command_line, &environment, boundary.story_dir(), console) {
            Ok(process) => process,
            Err(e) => {
                unsafe { ClosePseudoConsole(console) };
                return Err(io::Error::new(e.kind(), format!("powershell: {e}")));
            }
        };

        let mut reader = output;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            // Ends with an error once the console has closed.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                screen.lock().unwrap().process(&buf[..n]);
            }
        });

        let commands = match accept(&listener, &token, process) {
            Ok(stream) => stream,
            Err(e) => {
                unsafe {
                    TerminateProcess(process, 1);
                    CloseHandle(process);
                    ClosePseudoConsole(console);
                }
                return Err(e);
            }
        };
        let (status_tx, statuses) = mpsc::channel();
        let status_reader = BufReader::new(commands.try_clone()?);
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
        // The console clears the screen as PowerShell attaches; let that
        // through before the prompt is drawn.
        std::thread::sleep(Duration::from_millis(300));
        Ok(Shell {
            input,
            commands,
            statuses,
            process,
            console,
        })
    }

    /// Start a command. Its status arrives through [`Shell::finished`].
    pub fn run(&mut self, command: &str) -> io::Result<()> {
        let mut line = base64(command.as_bytes());
        line.push('\n');
        self.commands.write_all(line.as_bytes())
    }

    /// The exit status of the last command, once it has finished.
    pub fn finished(&mut self) -> io::Result<Option<i32>> {
        match self.statuses.try_recv() {
            Ok(status) => Ok(Some(status)),
            Err(mpsc::TryRecvError::Empty) => match exit_code(self.process) {
                Some(code) => Err(io::Error::other(format!(
                    "the terminal's shell exited ({code})"
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
        self.input.write_all(bytes)
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.process, 1);
            CloseHandle(self.process);
            ClosePseudoConsole(self.console);
        }
    }
}

/// A pseudo console of `rows` by `cols`, with the file to write its input
/// to and the file to read its output from.
fn open_console(rows: u16, cols: u16) -> io::Result<(HPCON, File, File)> {
    let (mut input_read, mut input_write) = (0 as HANDLE, 0 as HANDLE);
    let (mut output_read, mut output_write) = (0 as HANDLE, 0 as HANDLE);
    unsafe {
        if CreatePipe(&mut input_read, &mut input_write, std::ptr::null(), 0) == 0
            || CreatePipe(&mut output_read, &mut output_write, std::ptr::null(), 0) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let size = COORD {
        X: cols as i16,
        Y: rows as i16,
    };
    let mut console: HPCON = 0;
    let result = unsafe { CreatePseudoConsole(size, input_read, output_write, 0, &mut console) };
    unsafe {
        // The console keeps its own copies of its ends.
        CloseHandle(input_read);
        CloseHandle(output_write);
    }
    if result < 0 {
        unsafe {
            CloseHandle(input_write);
            CloseHandle(output_read);
        }
        return Err(io::Error::other(format!(
            "CreatePseudoConsole failed (0x{result:08x}); Windows 10 1809 or newer is needed"
        )));
    }
    let (input, output) = unsafe {
        (
            File::from_raw_handle(input_write as _),
            File::from_raw_handle(output_read as _),
        )
    };
    Ok((console, input, output))
}

/// Start `command_line` attached to `console`, with `environment` as its
/// whole environment, in `dir`. Returns the process handle.
fn spawn(
    command_line: &str,
    environment: &[(String, String)],
    dir: &std::path::Path,
    console: HPCON,
) -> io::Result<HANDLE> {
    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    let mut attributes = vec![0u8; size];
    let list = attributes.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    unsafe {
        if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            console as *const c_void,
            std::mem::size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
        {
            let e = io::Error::last_os_error();
            DeleteProcThreadAttributeList(list);
            return Err(e);
        }
    }
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    // Null standard handles: the child gets the console's, not ours.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = list;
    let mut command: Vec<u16> = command_line.encode_utf16().chain([0]).collect();
    let mut sorted: Vec<&(String, String)> = environment.iter().collect();
    sorted.sort_by_key(|(k, _)| k.to_uppercase());
    let block: Vec<u16> = sorted
        .iter()
        .flat_map(|(k, v)| format!("{k}={v}\0").encode_utf16().collect::<Vec<_>>())
        .chain([0])
        .collect();
    let dir: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr() as *const c_void,
            dir.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    };
    let error = io::Error::last_os_error();
    unsafe { DeleteProcThreadAttributeList(list) };
    if ok == 0 {
        return Err(error);
    }
    unsafe { CloseHandle(info.hThread) };
    Ok(info.hProcess)
}

/// Wait for the shell to connect and send `token`, failing if it exits or
/// takes too long.
fn accept(listener: &TcpListener, token: &str, process: HANDLE) -> io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Some(code) = exit_code(process) {
                    return Err(io::Error::other(format!(
                        "powershell exited ({code}) before the terminal was ready"
                    )));
                }
                if start.elapsed() > CONNECT_TIMEOUT {
                    return Err(io::Error::other(
                        "powershell did not connect to the terminal in time",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    let mut first = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut first)?;
    if first.trim().trim_start_matches('\u{feff}') != token {
        return Err(io::Error::other(
            "something other than the terminal's shell connected to it",
        ));
    }
    stream.set_read_timeout(None)?;
    Ok(stream)
}

/// The process's exit code, once it has exited.
fn exit_code(process: HANDLE) -> Option<u32> {
    let mut code = 0u32;
    let ok = unsafe { GetExitCodeProcess(process, &mut code) };
    (ok != 0 && code != STILL_ACTIVE).then_some(code)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

use std::os::windows::ffi::OsStrExt;

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
