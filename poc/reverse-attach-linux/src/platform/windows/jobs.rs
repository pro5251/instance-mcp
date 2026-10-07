//! Background `exec_start` / `exec_poll` / `exec_list` / `exec_cancel` on Windows, with
//! the macOS contract (spec §4.5): spawn and return a job_id immediately; stdout/stderr
//! are tee'd to files that `exec_poll` reads by byte offset; terminal metadata is kept a
//! while; cancel signals the job. Each job runs a PowerShell under its own Job Object
//! (no KILL_ON_JOB_CLOSE), reusing `proc`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::proc::{self, Child};
use crate::tools::tool_result;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    Exited,
    Killed,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Exited => "exited",
            State::Killed => "killed",
        }
    }
}

struct Inner {
    state: State,
    exit_code: Option<i32>,
    finished_at: Option<SystemTime>,
}

struct Job {
    id: String,
    pid: u32,
    command: String,
    cwd: String,
    started_at: SystemTime,
    out_path: PathBuf,
    err_path: PathBuf,
    /// Held so the job (and its process tree) can be signalled; taken when reaped so the
    /// handles close and a survivor is left alone.
    child: Mutex<Option<Child>>,
    inner: Mutex<Inner>,
}

impl Job {
    fn snapshot(&self) -> (State, Option<i32>, Option<SystemTime>) {
        let i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (i.state, i.exit_code, i.finished_at)
    }
    fn finish(&self, state: State, exit_code: i32) {
        let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if i.state == State::Running {
            i.state = state;
            i.exit_code = Some(exit_code);
            i.finished_at = Some(SystemTime::now());
        }
    }
}

struct Registry {
    jobs: Mutex<HashMap<String, std::sync::Arc<Job>>>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Registry {
        jobs: Mutex::new(HashMap::new()),
    })
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    format!("job-{}-{}", now_secs(), SEQ.fetch_add(1, Ordering::Relaxed))
}

/// `%LOCALAPPDATA%\oab-imcp-winpoc\jobs`. Ticket 10 tightens the DACL; here we just make
/// it. Falls back to the temp dir if LOCALAPPDATA is unset.
fn jobs_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("oab-imcp-winpoc").join("jobs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Remove log files older than 7 days (macOS keeps the files, GCs by age). Best-effort.
fn gc_old_logs(dir: &Path) {
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.modified().map(|m| m < cutoff).unwrap_or(false) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// tools
// ---------------------------------------------------------------------------

fn iso(t: SystemTime) -> String {
    // Seconds since the epoch is enough for the POC; callers sort by it, not parse it.
    format!(
        "{}",
        t.duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    )
}

pub(crate) fn tool_exec_start(args: &Value) -> Result<Value, (i64, String)> {
    let bad = |e: String| (-32602, e);
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| bad("command is required".to_string()))?;
    if command.chars().count() > proc::MAX_COMMAND_CHARS {
        return Err(bad(format!(
            "command too long (max {})",
            proc::MAX_COMMAND_CHARS
        )));
    }
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cwd = match args.get("cwd").and_then(Value::as_str) {
        Some(c) => Some(super::exec::resolve_cwd(c).map_err(bad)?),
        None => None,
    };
    let env = args.get("env").cloned().unwrap_or(Value::Null);

    let dir = jobs_dir();
    gc_old_logs(&dir);
    let id = new_id();
    let out_path = dir.join(format!("{id}.out"));
    let err_path = dir.join(format!("{id}.err"));

    let child = proc::spawn_to_files(command, cwd.as_deref(), &env, &out_path, &err_path)
        .map_err(|e| (-32000, e))?;
    let pid = child.pid;
    proc::resume(&child);

    let cwd_str = cwd.unwrap_or_default();
    let job = std::sync::Arc::new(Job {
        id: id.clone(),
        pid,
        command: command.to_string(),
        cwd: cwd_str.clone(),
        started_at: SystemTime::now(),
        out_path,
        err_path,
        child: Mutex::new(Some(child)),
        inner: Mutex::new(Inner {
            state: State::Running,
            exit_code: None,
            finished_at: None,
        }),
    });
    registry()
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), job.clone());

    // Monitor: wait for exit (enforcing the optional timeout), record the result, and
    // release the handles so a surviving GUI child is left alone.
    std::thread::spawn(move || {
        let deadline =
            (timeout_secs > 0).then(|| Instant::now() + Duration::from_secs(timeout_secs));
        loop {
            let got = {
                let guard = job.child.lock().unwrap_or_else(|e| e.into_inner());
                match guard.as_ref() {
                    None => return, // cancelled and reaped elsewhere
                    Some(c) => proc::wait(c, 200),
                }
            };
            if got {
                if let Some(c) = job.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    job.finish(State::Exited, proc::exit_code_of(&c.process));
                }
                return;
            }
            if let Some(d) = deadline {
                if Instant::now() >= d {
                    if let Some(c) = job.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        proc::terminate(&c, 137);
                        job.finish(State::Killed, 137);
                    }
                    return;
                }
            }
        }
    });

    Ok(tool_result(json!({
        "job_id": id,
        "pid": pid,
        "state": "running",
        "cwd": cwd_str
    })))
}

/// Read a log file from byte offset `from`; returns (bytes, next_offset).
fn read_log(path: &Path, from: u64) -> (Vec<u8>, u64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return (Vec::new(), from);
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if size <= from {
        return (Vec::new(), size);
    }
    if f.seek(SeekFrom::Start(from)).is_err() {
        return (Vec::new(), from);
    }
    let mut buf = Vec::new();
    let _ = f.take(size - from).read_to_end(&mut buf);
    let next = from + buf.len() as u64;
    (buf, next)
}

pub(crate) fn tool_exec_poll(args: &Value) -> Result<Value, (i64, String)> {
    let id = args
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "job_id is required".to_string()))?;
    let job = registry()
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| {
            (
                -32602,
                format!("unknown job_id: {id} (its metadata may have been garbage-collected)"),
            )
        })?;
    let out_since = args
        .get("stdout_since")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let err_since = args
        .get("stderr_since")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let (out, out_next) = read_log(&job.out_path, out_since);
    let (err, err_next) = read_log(&job.err_path, err_since);
    let (state, exit_code, _) = job.snapshot();

    let mut structured = json!({
        "job_id": id,
        "state": state.as_str(),
        "stdout": String::from_utf8_lossy(&out),
        "stderr": String::from_utf8_lossy(&err),
        "stdout_next": out_next,
        "stderr_next": err_next,
        "out_path": job.out_path.to_string_lossy(),
        "err_path": job.err_path.to_string_lossy(),
    });
    if let Some(code) = exit_code {
        structured["exit_code"] = json!(code);
    }
    gc_terminal();
    Ok(tool_result(structured))
}

pub(crate) fn tool_exec_list(_: &Value) -> Result<Value, (i64, String)> {
    let jobs: Vec<std::sync::Arc<Job>> = registry()
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    let mut running = Vec::new();
    let mut finished = Vec::new();
    for j in jobs {
        let (state, exit_code, finished_at) = j.snapshot();
        let mut o = json!({
            "job_id": j.id,
            "state": state.as_str(),
            "pid": j.pid,
            "command": j.command,
            "cwd": j.cwd,
            "started_at": iso(j.started_at),
            "stdout_bytes": std::fs::metadata(&j.out_path).map(|m| m.len()).unwrap_or(0),
            "stderr_bytes": std::fs::metadata(&j.err_path).map(|m| m.len()).unwrap_or(0),
        });
        if let Some(code) = exit_code {
            o["exit_code"] = json!(code);
        }
        if let Some(f) = finished_at {
            o["finished_at"] = json!(iso(f));
            finished.push((f, o));
        } else {
            running.push(o);
        }
    }
    // Running first, then the 10 most recently finished.
    finished.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let mut arr = running;
    arr.extend(finished.into_iter().take(10).map(|(_, o)| o));
    Ok(tool_result(json!({ "jobs": arr })))
}

pub(crate) fn tool_exec_cancel(args: &Value) -> Result<Value, (i64, String)> {
    let id = args
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "job_id is required".to_string()))?;
    let signal = args.get("signal").and_then(Value::as_str).unwrap_or("KILL");
    let job = registry()
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| (-32602, format!("unknown job_id: {id}")))?;

    let (state, _, _) = job.snapshot();
    if state != State::Running {
        // Already finished: drop it so its handles/metadata are freed (macOS does this).
        registry()
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        return Ok(tool_result(json!({
            "job_id": id, "state": state.as_str(), "dropped": true
        })));
    }
    // KILL is immediate. TERM is best-effort on Windows (no SIGTERM): try CTRL_BREAK and
    // WM_CLOSE, then force after 5 s. The monitor thread records the final state.
    let child = job.child.lock().unwrap_or_else(|e| e.into_inner()).take();
    let method;
    if let Some(c) = child {
        if signal == "TERM" {
            if proc::try_graceful_stop(&c, Duration::from_secs(5)) {
                method = "WM_CLOSE (graceful)";
                job.finish(State::Killed, 143);
            } else {
                proc::terminate(&c, 143); // 128 + SIGTERM, after the grace window
                method = "WM_CLOSE then terminate";
                job.finish(State::Killed, 143);
            }
        } else {
            proc::terminate(&c, 137);
            method = "terminate (KILL)";
            job.finish(State::Killed, 137);
        }
    } else {
        method = "already-exiting";
    }
    Ok(tool_result(json!({
        "job_id": id, "signalled": true, "method": method, "state": "killed"
    })))
}

/// Drop terminal jobs whose metadata has been kept for over ~10 minutes (macOS retention).
fn gc_terminal() {
    let mut jobs = registry().jobs.lock().unwrap_or_else(|e| e.into_inner());
    let now = SystemTime::now();
    jobs.retain(|_, j| {
        let (_, _, finished_at) = j.snapshot();
        match finished_at {
            Some(f) => now
                .duration_since(f)
                .map(|d| d < Duration::from_secs(600))
                .unwrap_or(true),
            None => true,
        }
    });
}
