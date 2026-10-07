//! Running Windows PowerShell under a Job Object, the way spike 3 settled (spec §4.4):
//! create suspended, assign to a job (no KILL_ON_JOB_CLOSE, so GUI children a command
//! launches outlive it), resume, drain stdout/stderr on threads, kill the whole tree on
//! timeout, and bound the wait for a survivor still holding a pipe.
//!
//! The command goes in through `-Command`, quoted the way `std::process::Command::arg`
//! quotes on Windows; `-EncodedCommand` was rejected in the spike because its error
//! stream comes back as CLIXML and its length ceiling is far lower.

use std::io::Read;
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

/// The daemon's environment, UTF-16, NUL-separated, double-NUL-terminated, **without**
/// `PSModulePath`: a value inherited from a PowerShell 7 parent makes 5.1 load the wrong
/// modules and fail (spike 3). A caller's extra vars are merged in.
fn environment_block(extra: &serde_json::Value) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("PSModulePath"))
        .collect();
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
pub(crate) struct Spawned {
    // Field (= drop) order matters: the job handle must close LAST. A Job Object with
    // no KILL_ON_JOB_CLOSE keeps its surviving processes alive after its last handle
    // closes, but only once the process/thread handles are gone first; closing the job
    // handle while the dead parent's handles are still open takes the children with it
    // (observed on Windows 11; spike 3 closed the job last too).
    out_read: Option<OwnedHandle>,
    err_read: Option<OwnedHandle>,
    process: OwnedHandle,
    thread: OwnedHandle,
    job: OwnedHandle,
    pub(crate) pid: u32,
}

/// Create (suspended), assign to a fresh job, but do not resume yet.
pub(crate) fn spawn(
    command: &str,
    cwd: Option<&str>,
    extra_env: &serde_json::Value,
) -> Result<Spawned, String> {
    let out = make_pipe()?;
    let err = make_pipe()?;

    // SAFETY: CreateJobObjectW with null attrs/name returns an owned handle or null.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(format!("CreateJobObject failed ({})", last_error()));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job as _) };
    // No KILL_ON_JOB_CLOSE: a GUI app the command starts must survive this call ending,
    // as on macOS (launchd) and Linux (KillMode=process). We kill the tree explicitly
    // on timeout instead.
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
    si.hStdOutput = raw(&out.write);
    si.hStdError = raw(&err.write);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: all pointers are to live, correctly-sized buffers; bInheritHandles=true so
    // the child gets the inheritable write ends of the pipes.
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

    // Drop our copies of the write ends, so EOF arrives when the child (and any survivor
    // holding them) closes them.
    drop(out.write);
    drop(err.write);

    Ok(Spawned {
        job,
        process,
        thread,
        out_read: Some(out.read),
        err_read: Some(err.read),
        pid: pi.dwProcessId,
    })
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

    // SAFETY: a valid suspended thread handle.
    unsafe { ResumeThread(raw(&sp.thread)) };

    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    // SAFETY: a valid process handle.
    let waited = unsafe { WaitForSingleObject(raw(&sp.process), ms) };
    let timed_out = waited == WAIT_TIMEOUT;
    if timed_out {
        // SAFETY: a valid job handle; kills the whole tree.
        unsafe { TerminateJobObject(raw(&sp.job), 137) };
        // SAFETY: wait for the forced exit so the code is final.
        unsafe { WaitForSingleObject(raw(&sp.process), 5000) };
    }

    let mut code: u32 = 0;
    // SAFETY: out-pointer to a local.
    unsafe { GetExitCodeProcess(raw(&sp.process), &mut code) };

    // Bound the drain: a surviving grandchild can hold the write end open after the
    // command returned. Wait, then cancel the blocked reads.
    let (stdout, stdout_truncated, stderr, stderr_truncated) =
        join_bounded(out_t, err_t, out_raw, err_raw, Duration::from_secs(2));

    let exit_code = if timed_out { 137 } else { code as i32 };
    Outcome {
        exit_code,
        timed_out,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        duration_ms: started.elapsed().as_millis() as u64,
        pid: sp.pid,
    }
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
