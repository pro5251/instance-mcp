//! Running Windows PowerShell under a Job Object, the way spike 3 settled (spec §4.4):
//! create suspended, assign to a job (no KILL_ON_JOB_CLOSE, so GUI children a command
//! launches outlive it), resume, drain stdout/stderr on threads, kill the whole tree on
//! timeout, and bound the wait for a survivor still holding a pipe.
//!
//! The command goes in through `-Command`, quoted the way `std::process::Command::arg`
//! quotes on Windows; `-EncodedCommand` was rejected in the spike because its error
//! stream comes back as CLIXML and its length ceiling is far lower.

use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, WaitForSingleObject, CREATE_NO_WINDOW,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOW,
};
use windows_sys::Win32::System::IO::CancelIoEx;

/// Windows PowerShell 5.1, by its full path so PATH cannot redirect it.
fn powershell_path() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Run `[Console]::OutputEncoding = UTF-8; $ProgressPreference = SilentlyContinue`
/// before the command, so stdout/stderr are UTF-8 and progress records do not leak
/// into stderr (spike 3).
const PREAMBLE: &str =
    "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $ProgressPreference = 'SilentlyContinue'; ";

/// The usable `-Command` length. The OS limit is 32767 chars for the whole command
/// line; the rest (the exe path and the fixed flags) leaves room to spare, and spike 3
/// ran 32500-char commands.
pub(crate) const MAX_COMMAND_CHARS: usize = 32000;

pub(crate) struct Outcome {
    pub(crate) exit_code: i32,
    pub(crate) timed_out: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
    pub(crate) duration_ms: u64,
    pub(crate) pid: u32,
}

/// MSVCRT argument quoting, identical to Rust std's `Command` on Windows, so a command
/// with quotes, backslashes, spaces and Unicode reaches PowerShell byte-for-byte
/// (verified round-trip in spike 3).
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

fn command_line(command: &str) -> String {
    let exe = powershell_path();
    let full = format!("{PREAMBLE}{command}");
    format!(
        "{} -NoProfile -NonInteractive -Command {}",
        quote(&exe),
        quote(&full)
    )
}

/// Windows PowerShell 5.1's own module search path. A `PSModulePath` inherited from a
/// PowerShell 7 parent points 5.1 at PS7's modules and it fails to load its own (spike
/// 3); but an *empty* `PSModulePath` is just as bad — 5.1 can no longer find built-in
/// modules like Microsoft.PowerShell.Security, so Get-Acl / Get-ExecutionPolicy and the
/// like break. So we set it to 5.1's default rather than dropping it.
fn ps5_module_path() -> String {
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let mut parts = Vec::new();
    if let Ok(up) = std::env::var("USERPROFILE") {
        parts.push(format!(r"{up}\Documents\WindowsPowerShell\Modules"));
    }
    parts.push(format!(r"{program_files}\WindowsPowerShell\Modules"));
    parts.push(format!(
        r"{sysroot}\system32\WindowsPowerShell\v1.0\Modules"
    ));
    parts.join(";")
}

/// The daemon's environment, UTF-16, NUL-separated, double-NUL-terminated, with
/// `PSModulePath` forced to 5.1's default (see `ps5_module_path`). A caller's extra vars
/// are merged in (and may override `PSModulePath` deliberately).
fn environment_block(extra: &serde_json::Value) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("PSModulePath"))
        .collect();
    vars.push(("PSModulePath".to_string(), ps5_module_path()));
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                vars.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
                vars.push((k.clone(), s.to_string()));
            }
        }
    }
    vars.sort_by_key(|(k, _)| k.to_ascii_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

struct Pipe {
    read: OwnedHandle,
    write: OwnedHandle,
}

fn make_pipe() -> Result<Pipe, String> {
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1, // child must inherit the write end
    };
    // SAFETY: out-pointers to locals; sa is valid for the call.
    if unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } == 0 {
        return Err(format!("CreatePipe failed ({})", last_error()));
    }
    // The read end stays ours: clear inheritance so the child cannot hold it open.
    // SAFETY: a handle we own.
    unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };
    // SAFETY: fresh handles from CreatePipe, owned from here.
    Ok(Pipe {
        read: unsafe { OwnedHandle::from_raw_handle(read as _) },
        write: unsafe { OwnedHandle::from_raw_handle(write as _) },
    })
}

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

fn raw(h: &OwnedHandle) -> HANDLE {
    use std::os::windows::io::AsRawHandle;
    h.as_raw_handle() as HANDLE
}

/// Drain a pipe on its own thread, capped at `cap` bytes but always read to EOF so the
/// child never blocks on a full pipe (spike 3 / the Linux bash tool do the same).
fn drain(read: OwnedHandle, cap: usize) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(read);
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut truncated = false;
        loop {
            match file.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf.len() < cap {
                        let take = n.min(cap - buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        truncated |= take < n;
                    } else {
                        truncated = true;
                    }
                }
            }
        }
        (buf, truncated)
    })
}

/// A created-suspended PowerShell in its own job, ready to resume.
pub(crate) struct Child {
    // Drop order matters: the job handle closes LAST. A job with no KILL_ON_JOB_CLOSE
    // keeps its surviving processes alive after its last handle closes, but only once
    // the process/thread handles are gone first; closing the job handle while the dead
    // parent's handles are still open takes the children with it (Windows 11; spike 3
    // closed the job last too).
    pub(crate) process: OwnedHandle,
    thread: OwnedHandle,
    pub(crate) job: OwnedHandle,
    pub(crate) pid: u32,
}

/// A sync run keeps the pipe read ends to drain in-process.
pub(crate) struct Spawned {
    out_read: Option<OwnedHandle>,
    err_read: Option<OwnedHandle>,
    child: Child,
}

fn new_job() -> Result<OwnedHandle, String> {
    // SAFETY: CreateJobObjectW with null attrs/name returns an owned handle or null.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(format!("CreateJobObject failed ({})", last_error()));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job as _) };
    // No KILL_ON_JOB_CLOSE: a GUI app the command starts must survive the command ending,
    // as on macOS (launchd) and Linux (KillMode=process); the tree is killed explicitly
    // on timeout/cancel instead.
    let info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: info is a valid, fully-initialised struct of the given class/size.
    if unsafe {
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(format!("SetInformationJobObject failed ({})", last_error()));
    }
    Ok(job)
}

/// Create PowerShell suspended with the given inheritable stdout/stderr write handles,
/// in a fresh job, resumed is left to the caller.
fn create_suspended(
    command: &str,
    cwd: Option<&str>,
    extra_env: &serde_json::Value,
    stdout_w: HANDLE,
    stderr_w: HANDLE,
) -> Result<Child, String> {
    let job = new_job()?;
    let mut cmdline: Vec<u16> = command_line(command)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let env = environment_block(extra_env);
    let cwd_wide: Option<Vec<u16>> =
        cwd.map(|c| c.encode_utf16().chain(std::iter::once(0)).collect());

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = INVALID_HANDLE_VALUE;
    si.hStdOutput = stdout_w;
    si.hStdError = stderr_w;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: live, correctly-sized buffers; bInheritHandles=true so the child gets the
    // inheritable write handles.
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            env.as_ptr() as *const _,
            cwd_wide.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(format!(
            "could not start PowerShell: CreateProcess failed ({})",
            last_error()
        ));
    }
    // SAFETY: valid handles from CreateProcess, owned from here.
    let process = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as _) };
    let thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as _) };

    // SAFETY: the process is suspended; assign before it runs so children are captured.
    if unsafe { AssignProcessToJobObject(raw(&job), raw(&process)) } == 0 {
        // SAFETY: a valid job handle; kills anything already placed in it.
        unsafe { TerminateJobObject(raw(&job), 1) };
        return Err(format!(
            "could not put PowerShell under a Job Object ({}); refusing to run without reliable \
             process-tree control",
            last_error()
        ));
    }
    Ok(Child {
        process,
        thread,
        job,
        pid: pi.dwProcessId,
    })
}

/// Resume a suspended child.
pub(crate) fn resume(child: &Child) {
    // SAFETY: a valid suspended thread handle.
    unsafe { ResumeThread(raw(&child.thread)) };
}

/// Create (suspended) for a synchronous run, with pipes we drain in-process.
pub(crate) fn spawn(
    command: &str,
    cwd: Option<&str>,
    extra_env: &serde_json::Value,
) -> Result<Spawned, String> {
    let out = make_pipe()?;
    let err = make_pipe()?;
    let child = create_suspended(command, cwd, extra_env, raw(&out.write), raw(&err.write))?;
    // Drop our copies of the write ends, so EOF arrives when the child (and any survivor
    // holding them) closes them.
    drop(out.write);
    drop(err.write);
    Ok(Spawned {
        out_read: Some(out.read),
        err_read: Some(err.read),
        child,
    })
}

/// Create (suspended) for a background job, with stdout/stderr going straight to files
/// (the source of truth exec_poll reads by offset, like the macOS job logs). Not resumed.
pub(crate) fn spawn_to_files(
    command: &str,
    cwd: Option<&str>,
    extra_env: &serde_json::Value,
    out_path: &std::path::Path,
    err_path: &std::path::Path,
) -> Result<Child, String> {
    let out = create_inheritable_file(out_path)?;
    let err = create_inheritable_file(err_path)?;
    create_suspended(command, cwd, extra_env, raw(&out), raw(&err))
    // out/err write handles drop here; the child holds its own inherited copies.
}

/// Resume, wait up to `timeout`, drain both pipes (bounded), and report. On timeout the
/// whole job is terminated (exit 137). A survivor still holding a pipe is bounded by
/// `drain_cap` before the reader is cancelled.
pub(crate) fn run(mut sp: Spawned, timeout: Duration, cap: usize) -> Outcome {
    let started = Instant::now();
    let out_read = sp.out_read.take().unwrap();
    let err_read = sp.err_read.take().unwrap();
    let out_raw = raw(&out_read);
    let err_raw = raw(&err_read);
    let out_t = drain(out_read, cap);
    let err_t = drain(err_read, cap);

    resume(&sp.child);

    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    // SAFETY: a valid process handle.
    let waited = unsafe { WaitForSingleObject(raw(&sp.child.process), ms) };
    let timed_out = waited == WAIT_TIMEOUT;
    if timed_out {
        terminate(&sp.child, 137);
    }
    let code = exit_code_of(&sp.child.process);

    // Bound the drain: a surviving grandchild can hold the write end open after the
    // command returned. Wait, then cancel the blocked reads.
    let (stdout, stdout_truncated, stderr, stderr_truncated) =
        join_bounded(out_t, err_t, out_raw, err_raw, Duration::from_secs(2));

    Outcome {
        exit_code: if timed_out { 137 } else { code },
        timed_out,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        duration_ms: started.elapsed().as_millis() as u64,
        pid: sp.child.pid,
    }
}

/// Best-effort graceful stop (there is no SIGTERM on Windows): post WM_CLOSE to any
/// top-level windows the job's root process owns, then wait up to `grace` for it to
/// exit. A console-less PowerShell has no window, so this usually returns false and the
/// caller forces termination. Returns true only if the tree exited on its own.
pub(crate) fn try_graceful_stop(child: &Child, grace: Duration) -> bool {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM};
    use windows_sys::Win32::System::Threading::GetProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };
    // SAFETY: a valid process handle.
    let pid = unsafe { GetProcessId(raw(&child.process)) };
    unsafe extern "system" fn close_if_pid(
        hwnd: windows_sys::Win32::Foundation::HWND,
        want: LPARAM,
    ) -> BOOL {
        let mut wpid = 0u32;
        // SAFETY: out-pointer to a local.
        unsafe { GetWindowThreadProcessId(hwnd, &mut wpid) };
        if wpid == want as u32 {
            // SAFETY: posting a documented message to a window handle.
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
        1
    }
    // SAFETY: the callback only reads the LPARAM and posts messages.
    unsafe { EnumWindows(Some(close_if_pid), pid as LPARAM) };
    wait(child, grace.as_millis().min(u32::MAX as u128) as u32)
}

/// Wait up to `ms` for the process to exit; true if it did.
pub(crate) fn wait(child: &Child, ms: u32) -> bool {
    // SAFETY: a valid process handle.
    unsafe { WaitForSingleObject(raw(&child.process), ms) != WAIT_TIMEOUT }
}

/// Kill the whole tree with the given exit code, then wait for it to settle.
pub(crate) fn terminate(child: &Child, code: u32) {
    // SAFETY: a valid job handle; kills the whole tree.
    unsafe { TerminateJobObject(raw(&child.job), code) };
    // SAFETY: wait for the forced exit so the code is final.
    unsafe { WaitForSingleObject(raw(&child.process), 5000) };
}

pub(crate) fn exit_code_of(process: &OwnedHandle) -> i32 {
    let mut code: u32 = 0;
    // SAFETY: out-pointer to a local.
    unsafe { GetExitCodeProcess(raw(process), &mut code) };
    code as i32
}

/// Create a file for writing whose handle the child may inherit (stdout/stderr target).
fn create_inheritable_file(path: &std::path::Path) -> Result<OwnedHandle, String> {
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_ALWAYS, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: wide is NUL-terminated; sa is valid for the call.
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &sa,
            CREATE_ALWAYS,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(format!(
            "cannot create {}: {}",
            path.display(),
            last_error()
        ));
    }
    // SAFETY: a fresh valid handle, owned from here.
    Ok(unsafe { OwnedHandle::from_raw_handle(h as _) })
}

fn join_bounded(
    out_t: std::thread::JoinHandle<(Vec<u8>, bool)>,
    err_t: std::thread::JoinHandle<(Vec<u8>, bool)>,
    out_raw: HANDLE,
    err_raw: HANDLE,
    cap: Duration,
) -> (Vec<u8>, bool, Vec<u8>, bool) {
    let deadline = Instant::now() + cap;
    while !(out_t.is_finished() && err_t.is_finished()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !out_t.is_finished() || !err_t.is_finished() {
        // SAFETY: cancel any read blocked on these handles; the drain threads then see
        // an error and finish.
        unsafe {
            CancelIoEx(out_raw, std::ptr::null());
            CancelIoEx(err_raw, std::ptr::null());
        }
    }
    let (stdout, out_trunc) = out_t.join().unwrap_or_default();
    let (stderr, err_trunc) = err_t.join().unwrap_or_default();
    (stdout, out_trunc, stderr, err_trunc)
}
