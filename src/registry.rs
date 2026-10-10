//! Which Claude Code processes are alive, which conversation each one holds, and what
//! each says it is doing — read from the CLI's own session registry.
//!
//! Every CLI process registers itself in `~/.claude/sessions/<pid>.json` (under
//! `$CLAUDE_CONFIG_DIR` when that is set) a second or so after it starts, keeps the
//! record current while it runs, and unlinks it on the way out. The record carries the
//! conversation the process holds *now* (it is rewritten in place on /clear, resume,
//! fork), and, on current builds, a `status` the CLI's own state machine writes:
//! `busy` from the moment a prompt is dequeued until the turn — and any background
//! agents it left running — is over, `waiting` while a permission prompt or a question
//! is open, `idle`, and `shell` (terminal sessions: idle with a background command).
//! That is everything the transcript cannot say: whether anything is alive to say it,
//! and whether the model is generating while the transcript sits silent.
//!
//! Three things make the directory less simple than it looks, and shaped this reader:
//!
//!   * A record can outlive its process. A SIGKILL or a crash leaves it behind until the
//!     next CLI start sweeps it, and a pid that has since been reused is never swept. So
//!     a record counts only when its pid is alive *and* is the same process: the start
//!     time the record carries (`procStart`, `ps lstart` rendered in UTC) must match the
//!     live process's own, to the second.
//!   * Records are written in place, truncate-then-write, so a read can land between the
//!     two. A parse failure is retried once after 25 ms, then the last good read stands.
//!   * The directory also holds each process's peer-socket key (`<pid>.<hash>.key`), which
//!     is a secret. Only names that are digits and `.json` are ever opened.
//!
//! This module is READ-ONLY. It never writes, renames or deletes anything there: the CLI
//! sweeps dead records itself, and a live process whose record is deleted never writes it
//! again.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a snapshot is reused. The endpoints poll every 200 ms and the notifier every
/// second; this keeps them to a handful of directory reads a second between them, while
/// staying well inside the latency a phone can notice.
const SNAPSHOT_TTL: Duration = Duration::from_millis(250);
/// The CLI's own retry delay for a record caught mid-write.
const TORN_RETRY: Duration = Duration::from_millis(25);
/// `procStart` has one-second resolution, and the kernel's start time has microseconds.
const START_TOLERANCE_SECS: i64 = 1;
/// Remembered start-time verdicts; cleared wholesale when exceeded, since a verdict is
/// only a saving, never a source of truth.
const VERDICTS_KEPT: usize = 4096;

/// What the CLI says its process is doing. Only the four values the CLI accepts are
/// named; anything else is carried through as `Unknown` rather than guessed at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegStatus {
    Busy,
    Waiting,
    Idle,
    Shell,
    Unknown(String),
}

impl RegStatus {
    fn parse(s: &str) -> Self {
        match s {
            "busy" => RegStatus::Busy,
            "waiting" => RegStatus::Waiting,
            "idle" => RegStatus::Idle,
            "shell" => RegStatus::Shell,
            other => RegStatus::Unknown(other.to_owned()),
        }
    }

    /// One of the values the CLI actually writes, which is what makes it trustworthy.
    #[allow(dead_code)]
    pub fn is_known(&self) -> bool {
        !matches!(self, RegStatus::Unknown(_))
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        match self {
            RegStatus::Busy => "busy",
            RegStatus::Waiting => "waiting",
            RegStatus::Idle => "idle",
            RegStatus::Shell => "shell",
            RegStatus::Unknown(s) => s,
        }
    }
}

/// One verified-live CLI process, as its record describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcRecord {
    /// From the file name, which the CLI derives from the pid: it wins over the body.
    pub pid: u32,
    pub session_id: String,
    pub cwd: Option<String>,
    /// When the process registered, in epoch ms — later than the process start by the
    /// registration lag (0.8–6 s measured).
    pub started_at_ms: Option<i64>,
    pub version: Option<String>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    /// Absent on a record written before the first status change, and on headless CLIs
    /// older than 2.1.278. Absent never means idle.
    pub status: Option<RegStatus>,
    /// A coarse label for a `waiting` status: `permission prompt`, `input needed`,
    /// `dialog open`, …
    pub waiting_for: Option<String>,
    /// When `status` was last written. Moves only on a status change; there is no
    /// heartbeat, so it says nothing about liveness.
    pub status_updated_ms: Option<i64>,
    pub updated_ms: Option<i64>,
    pub name: Option<String>,
    /// The process's parent, when the platform tells us.
    pub ppid: Option<u32>,
}

/// Every live CLI process, keyed by the conversation it holds.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The directory exists and is readable. When false, nothing can be said about
    /// open/closed from the registry (regime C).
    pub available: bool,
    /// Verified-live, de-duplicated, with the router's own children removed.
    pub by_session: HashMap<String, ProcRecord>,
    /// The conversations the router's own children hold (the side question's fork),
    /// each with when that process registered. Not sessions, but their transcripts are
    /// real files written right now, so the list needs to know which ones to leave out.
    pub spawned: HashMap<String, Option<i64>>,
    pub taken_ms: i64,
}

/// Claude Code's configuration directory: `$CLAUDE_CONFIG_DIR`, or `~/.claude`. The
/// registry and the transcripts both live under it, so both must honour the variable the
/// same way or the router reads one session's registry beside another's transcripts.
pub fn claude_dir() -> PathBuf {
    claude_dir_from(
        std::env::var_os("CLAUDE_CONFIG_DIR"),
        crate::setup::home_dir().ok().map(OsString::from),
    )
}

fn claude_dir_from(config: Option<OsString>, home: Option<OsString>) -> PathBuf {
    match config.filter(|c| !c.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => PathBuf::from(home.unwrap_or_default()).join(".claude"),
    }
}

/// Where the CLI keeps its session records.
pub fn sessions_dir() -> PathBuf {
    claude_dir().join("sessions")
}

/// The live processes, from a snapshot at most `SNAPSHOT_TTL` old.
///
/// Blocking: it reads a directory and may sleep 25 ms on a record caught mid-write, so
/// async handlers call it from `spawn_blocking`. Concurrent callers share one read.
#[allow(dead_code)]
pub fn snapshot() -> Arc<Snapshot> {
    static CACHE: OnceLock<Mutex<(Option<(Instant, Arc<Snapshot>)>, Reader)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new((None, Reader::new())));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, snap)) = &guard.0 {
        if at.elapsed() < SNAPSHOT_TTL {
            return snap.clone();
        }
    }
    let snap = Arc::new(guard.1.read(&sessions_dir(), now_ms(), &System));
    guard.0 = Some((Instant::now(), snap.clone()));
    snap
}

/// One uncached read of `dir`, for tests and one-off checks: no remembered records, no
/// remembered verdicts.
#[allow(dead_code)]
pub fn snapshot_in(dir: &Path, now_ms: i64) -> Snapshot {
    Reader::new().read(dir, now_ms, &System)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What the process table says about a pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcState {
    Dead,
    /// Alive. The start time (epoch seconds) and parent, where the platform gives them.
    Alive {
        start_secs: Option<i64>,
        ppid: Option<u32>,
    },
}

/// The process table, behind a seam so the verification rules can be tested against a
/// table that says whatever a test needs.
trait Procs {
    fn info(&self, pid: u32) -> ProcState;
}

/// The real process table.
struct System;

impl Procs for System {
    #[cfg(target_os = "macos")]
    fn info(&self, pid: u32) -> ProcState {
        let Ok(p) = libc::c_int::try_from(pid) else {
            return ProcState::Dead;
        };
        if p <= 0 {
            return ProcState::Dead;
        }
        // proc_pidinfo, not `ps`: one syscall rather than a fork, and the start time comes
        // back in epoch seconds with no time zone or locale to get wrong.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: the buffer is a properly sized, writable proc_bsdinfo.
        let n = unsafe {
            libc::proc_pidinfo(
                p,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            )
        };
        if n == size {
            // A zombie has exited and only waits for its parent to reap it.
            if info.pbi_status == libc::SZOMB {
                return ProcState::Dead;
            }
            return ProcState::Alive {
                start_secs: Some(info.pbi_start_tvsec as i64),
                ppid: Some(info.pbi_ppid),
            };
        }
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return ProcState::Dead;
        }
        // Refused rather than absent (another user's process): alive, details unknown.
        kill_probe(p)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn info(&self, pid: u32) -> ProcState {
        let Ok(p) = libc::c_int::try_from(pid) else {
            return ProcState::Dead;
        };
        if p <= 0 {
            return ProcState::Dead;
        }
        kill_probe(p)
    }

    /// The process's exit code says whether it runs, its creation time is the start to
    /// match `procStart` against, and the toolhelp snapshot names its parent.
    #[cfg(windows)]
    fn info(&self, pid: u32) -> ProcState {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        if pid == 0 {
            return ProcState::Dead;
        }
        // SAFETY: plain Win32 calls on a handle this function opens and closes.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                // Refused rather than absent: another user's process, alive.
                return if GetLastError() == ERROR_ACCESS_DENIED {
                    ProcState::Alive { start_secs: None, ppid: None }
                } else {
                    ProcState::Dead
                };
            }
            let mut code = 0u32;
            let running = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
            let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
            let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
            let start_secs = (GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0).then(|| {
                // 100 ns ticks since 1601-01-01, to seconds since 1970-01-01.
                let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
                (ticks / 10_000_000) as i64 - 11_644_473_600
            });
            CloseHandle(handle);
            if !running {
                return ProcState::Dead;
            }
            ProcState::Alive { start_secs, ppid: parent_of(pid) }
        }
    }
}

/// A process's parent, from a snapshot of the process table.
#[cfg(windows)]
fn parent_of(pid: u32) -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    // SAFETY: the snapshot handle is checked and closed; the entry is sized as required.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            if entry.th32ProcessID == pid {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

/// Liveness alone: signal 0 delivers nothing, and EPERM still proves the pid exists.
#[cfg(unix)]
fn kill_probe(pid: libc::c_int) -> ProcState {
    // SAFETY: signal 0 performs the permission and existence checks only; pid > 0.
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) {
        ProcState::Alive {
            start_secs: None,
            ppid: None,
        }
    } else {
        ProcState::Dead
    }
}

/// A record as read, with the start time it claims — which is how it is verified, not
/// part of what it says about the session.
#[derive(Clone, Debug)]
struct Raw {
    rec: ProcRecord,
    proc_start: Option<String>,
}

/// A reader of the registry directory, with what it remembers between reads: the last
/// good parse of each record (for torn reads) and each record's start-time verdict.
struct Reader {
    last_good: HashMap<u32, Raw>,
    /// (pid, procStart) -> (the live process's start seconds when judged, the verdict).
    verdicts: HashMap<(u32, String), (Option<i64>, bool)>,
    warned: bool,
    /// Processes whose parent or grandparent is this pid were started by the router
    /// itself (the `/btw` side question's fork) and are not sessions anyone opened.
    self_pid: u32,
    /// Every file this reader opened, so a test can prove it never touched a key.
    #[cfg(test)]
    opened: Vec<PathBuf>,
}

impl Reader {
    fn new() -> Self {
        Reader {
            last_good: HashMap::new(),
            verdicts: HashMap::new(),
            warned: false,
            self_pid: std::process::id(),
            #[cfg(test)]
            opened: Vec::new(),
        }
    }

    fn read(&mut self, dir: &Path, now_ms: i64, procs: &dyn Procs) -> Snapshot {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Snapshot {
                available: false,
                by_session: HashMap::new(),
                spawned: HashMap::new(),
                taken_ms: now_ms,
            };
        };

        let mut records: Vec<Raw> = Vec::new();
        let mut seen: Vec<u32> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(record_pid) else {
                continue;
            };
            seen.push(pid);
            if let Some(r) = self.load(&entry.path(), pid) {
                records.push(r);
            }
        }
        // A record that is gone is gone: its last good parse must not outlive it.
        self.last_good.retain(|pid, _| seen.contains(pid));

        // Liveness first, then identity.
        struct Judged {
            rec: ProcRecord,
            /// None: nothing to compare (no procStart, or no start time from the platform).
            matches: Option<bool>,
        }
        let mut alive: Vec<Judged> = Vec::new();
        for raw in records {
            let ProcState::Alive { start_secs, ppid } = procs.info(raw.rec.pid) else {
                continue;
            };
            let matches = self.verdict(&raw, start_secs);
            let mut rec = raw.rec;
            rec.ppid = ppid;
            alive.push(Judged { rec, matches });
        }

        // Every live pid failing its start-time match says more about us than about them:
        // a parser that misreads the format, or a clock read in the wrong zone, would make
        // every session look closed at once. Fall back to liveness alone and say so.
        let judged: Vec<bool> = alive.iter().filter_map(|j| j.matches).collect();
        let distrust = judged.len() >= 2 && judged.iter().all(|m| !m);
        if distrust && !self.warned {
            self.warned = true;
            tracing::warn!(
                records = judged.len(),
                "no live session record matches its process start time; verifying by liveness only"
            );
        }

        let mut by_session: HashMap<String, ProcRecord> = HashMap::new();
        let mut spawned: HashMap<String, Option<i64>> = HashMap::new();
        for j in alive {
            if j.matches == Some(false) && !distrust {
                continue; // the pid was reused by some other process
            }
            if self.spawned_by_us(&j.rec, procs) {
                spawned.insert(j.rec.session_id.clone(), j.rec.started_at_ms);
                continue;
            }
            let rec = j.rec;
            // Two live processes can name one conversation for a moment (a resume racing
            // an exit): the one that spoke last is the one holding it.
            let newer = by_session.get(&rec.session_id).is_none_or(|cur| {
                (rec.status_updated_ms, rec.updated_ms) > (cur.status_updated_ms, cur.updated_ms)
            });
            if newer {
                by_session.insert(rec.session_id.clone(), rec);
            }
        }
        Snapshot {
            available: true,
            by_session,
            spawned,
            taken_ms: now_ms,
        }
    }

    /// Read and parse one record, riding out a write caught halfway.
    fn load(&mut self, path: &Path, pid: u32) -> Option<Raw> {
        for attempt in 0..2 {
            if attempt == 1 {
                std::thread::sleep(TORN_RETRY);
            }
            #[cfg(test)]
            self.opened.push(path.to_path_buf());
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                // Unlinked between the listing and the read: the process just exited.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.last_good.remove(&pid);
                    return None;
                }
                Err(_) => continue,
            };
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) if v.is_object() => {
                    let rec = parse_record(&v, pid);
                    match &rec {
                        Some(r) => {
                            self.last_good.insert(pid, r.clone());
                        }
                        None => {
                            self.last_good.remove(&pid);
                        }
                    }
                    return rec;
                }
                _ => continue,
            }
        }
        self.last_good.get(&pid).cloned()
    }

    /// Whether the live process is the one that wrote the record. Remembered per
    /// (pid, procStart, observed start), so a reused pid — whose start time differs — is
    /// judged afresh rather than inheriting its predecessor's verdict. A `procStart` this
    /// parser cannot read is no verdict at all (liveness decides), not a mismatch: a format
    /// change in the CLI must not close every session, and the guard in `read` cannot
    /// catch that when only one session is running.
    fn verdict(&mut self, raw: &Raw, start_secs: Option<i64>) -> Option<bool> {
        let proc_start = raw.proc_start.clone()?;
        let observed = start_secs?;
        let key = (raw.rec.pid, proc_start.clone());
        if let Some((seen, verdict)) = self.verdicts.get(&key) {
            if *seen == Some(observed) {
                return Some(*verdict);
            }
        }
        let recorded = parse_proc_start(&proc_start)? / 1000;
        let verdict = (recorded - observed).abs() <= START_TOLERANCE_SECS;
        if self.verdicts.len() >= VERDICTS_KEPT {
            self.verdicts.clear();
        }
        self.verdicts.insert(key, (Some(observed), verdict));
        Some(verdict)
    }

    /// A process whose parent or grandparent is the router: the side question's
    /// `claude -p --resume … --fork-session` registers like any session.
    fn spawned_by_us(&self, rec: &ProcRecord, procs: &dyn Procs) -> bool {
        let Some(ppid) = rec.ppid else { return false };
        if ppid == self.self_pid {
            return true;
        }
        matches!(procs.info(ppid), ProcState::Alive { ppid: Some(gp), .. } if gp == self.self_pid)
    }
}

/// A pid from a record's file name: digits, then `.json`, and nothing else. Keys
/// (`<pid>.<hash>.key`), their temp files and the heartbeat never match.
fn record_pid(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".json")?;
    if stem.is_empty() || stem.len() > 10 || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse::<u32>().ok().filter(|&p| p > 0)
}

/// The fields of a record, each optional but the conversation it holds. A value of the
/// wrong type reads as absent rather than failing the record: an unexpected field must
/// not make a live session look closed.
fn parse_record(v: &Value, pid: u32) -> Option<Raw> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let num = |k: &str| {
        v.get(k).and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
        })
    };
    let session_id = text("sessionId").filter(|s| !s.is_empty())?;
    Some(Raw {
        proc_start: text("procStart"),
        rec: ProcRecord {
            pid,
            cwd: text("cwd"),
            started_at_ms: num("startedAt"),
            version: text("version"),
            kind: text("kind"),
            entrypoint: text("entrypoint"),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .map(RegStatus::parse),
            waiting_for: text("waitingFor"),
            status_updated_ms: num("statusUpdatedAt"),
            updated_ms: num("updatedAt"),
            name: text("name"),
            ppid: None,
            session_id,
        },
    })
}

/// `Tue Sep 29 09:34:36 2026` — `ps -o lstart=` as the CLI runs it, with `TZ=UTC` and
/// `LC_ALL=C` — in epoch milliseconds. The weekday is not checked, and the day may be
/// padded with a space or a zero: only the parsed instant is compared, never the string.
pub fn parse_proc_start(s: &str) -> Option<i64> {
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() != 5 {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS.iter().position(|m| *m == f[1])? as i64 + 1;
    let day: i64 = f[2].parse().ok()?;
    let year: i64 = f[4].parse().ok()?;
    let t: Vec<i64> = f[3]
        .split(':')
        .map(|x| x.parse().ok())
        .collect::<Option<Vec<i64>>>()?;
    if t.len() != 3 || !(1..=31).contains(&day) || t[0] > 23 || t[1] > 59 || t[2] > 60 {
        return None;
    }
    let days = days_from_civil(year, mon, day);
    Some((days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2]) * 1000)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm),
/// so no date crate is needed for UTC arithmetic.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of `days_from_civil`: (year, month, day).
#[allow(dead_code)]
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::os::unix::fs::PermissionsExt;

    /// `ps -o lstart=` in UTC, as the CLI stores it.
    fn ctime(secs: i64) -> String {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        format!(
            "{} {} {:>2} {:02}:{:02}:{:02} {y}",
            DAYS[(days + 4).rem_euclid(7) as usize],
            MONTHS[(m - 1) as usize],
            d,
            rem / 3600,
            rem / 60 % 60,
            rem % 60
        )
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ar-registry-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn record(session: &str, proc_start: &str, status: Option<&str>, status_at: i64) -> String {
        let mut v = serde_json::json!({
            "pid": 1, "sessionId": session, "cwd": "/w", "startedAt": 1_790_674_477_325i64,
            "procStart": proc_start, "version": "2.1.284", "kind": "interactive",
            "entrypoint": "claude-vscode", "statusUpdatedAt": status_at, "updatedAt": status_at,
            "name": "derived-name", "peerProtocol": 1,
        });
        if let Some(s) = status {
            v["status"] = serde_json::json!(s);
        }
        v.to_string()
    }

    /// A process table a test controls.
    struct Fake(HashMap<u32, ProcState>);

    impl Procs for Fake {
        fn info(&self, pid: u32) -> ProcState {
            self.0.get(&pid).copied().unwrap_or(ProcState::Dead)
        }
    }

    fn alive(start: i64, ppid: u32) -> ProcState {
        ProcState::Alive {
            start_secs: Some(start),
            ppid: Some(ppid),
        }
    }

    #[cfg(target_os = "macos")]
    fn own_start_secs() -> i64 {
        match System.info(std::process::id()) {
            ProcState::Alive {
                start_secs: Some(s),
                ..
            } => s,
            other => panic!("our own process must be readable: {other:?}"),
        }
    }

    #[test]
    fn proc_start_is_read_as_utc() {
        // Ground truth from peer.rs: pid 26643 reported this.
        assert_eq!(
            parse_proc_start("Thu Sep 10 07:35:03 2026"),
            Some(1_789_025_703_000)
        );
        // 4352's record and `TZ=UTC LC_ALL=C ps -o lstart=` agree on this; plain `ps`
        // printed 05:34:36, the local time, which must never be what we compare.
        assert_eq!(
            parse_proc_start("Tue Sep 29 09:34:36 2026"),
            Some(1_790_674_476_000)
        );
        // Day padding either way reads the same instant.
        assert_eq!(
            parse_proc_start("Tue Sep  1 09:34:36 2026"),
            parse_proc_start("Tue Sep 01 09:34:36 2026")
        );
        assert!(parse_proc_start("").is_none());
        assert!(parse_proc_start("2026-09-10T07:35:03Z").is_none());
        assert!(parse_proc_start("Thu Sep 10 25:35:03 2026").is_none());
        // And the test's own formatter round-trips through it.
        for secs in [
            0i64,
            1_789_025_703,
            1_790_674_476,
            951_782_400, /* 2000-02-29 */
        ] {
            assert_eq!(
                parse_proc_start(&ctime(secs)),
                Some(secs * 1000),
                "{}",
                ctime(secs)
            );
        }
    }

    #[test]
    fn the_config_dir_follows_claude_config_dir() {
        let home = Some(OsString::from("/Users/x"));
        assert_eq!(
            claude_dir_from(None, home.clone()),
            PathBuf::from("/Users/x/.claude")
        );
        assert_eq!(
            claude_dir_from(Some(OsString::from("")), home.clone()),
            PathBuf::from("/Users/x/.claude")
        );
        assert_eq!(
            claude_dir_from(Some(OsString::from("/cfg")), home),
            PathBuf::from("/cfg")
        );
    }

    #[test]
    fn only_pid_json_names_are_records() {
        assert_eq!(record_pid("4352.json"), Some(4352));
        for not in [
            "4352.500170752ab115d9ee6df309d9d49d7aafa493e56afb7849401611b7a469086d.key",
            "4352.500170752ab115d9ee6df309d9d49d7aafa493e56afb7849401611b7a469086d.key.tmp.1a2b",
            ".fleetview-heartbeat",
            "4352.json.tmp",
            "x4352.json",
            ".json",
            "0.json",
            "99999999999.json",
        ] {
            assert_eq!(record_pid(not), None, "{not}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn records_are_read_and_verified_and_keys_are_never_opened() {
        let dir = tempdir("fixtures");
        let me = std::process::id();
        let start = own_start_secs();
        std::fs::write(
            dir.join(format!("{me}.json")),
            record("sess-live", &ctime(start), Some("busy"), 10),
        )
        .unwrap();
        // Torn: a record caught between truncate and write.
        std::fs::write(
            dir.join("4000000.json"),
            "{\"pid\":4000000,\"sessionId\":\"sess-torn",
        )
        .unwrap();
        // Names that are not records, one of them the secret key, unreadable to anyone.
        let key = dir.join(format!(
            "{me}.500170752ab115d9ee6df309d9d49d7aafa493e56afb7849401611b7a469086d.key"
        ));
        std::fs::write(&key, "{\"peerToken\":\"secret\"}").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::write(dir.join(".fleetview-heartbeat"), "x").unwrap();
        std::fs::write(
            dir.join(format!("{me}.json.tmp")),
            record("sess-tmp", &ctime(start), None, 1),
        )
        .unwrap();

        let mut reader = Reader::new();
        reader.self_pid = 1; // this test process is the session here, not the router
        let snap = reader.read(&dir, 42, &System);
        assert!(snap.available);
        assert_eq!(snap.taken_ms, 42);
        assert_eq!(snap.by_session.len(), 1, "{:?}", snap.by_session.keys());
        let r = &snap.by_session["sess-live"];
        assert_eq!(r.pid, me, "the file name wins");
        assert_eq!(r.status, Some(RegStatus::Busy));
        assert_eq!(r.started_at_ms, Some(1_790_674_477_325));
        assert_eq!(
            (
                r.cwd.as_deref(),
                r.version.as_deref(),
                r.entrypoint.as_deref()
            ),
            (Some("/w"), Some("2.1.284"), Some("claude-vscode"))
        );
        assert!(r.ppid.is_some(), "macOS gives the parent");
        assert!(
            reader
                .opened
                .iter()
                .all(|p| p.extension().and_then(|e| e.to_str()) == Some("json")),
            "only records were opened: {:?}",
            reader.opened
        );
        assert!(!reader.opened.contains(&key));

        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_record_keeps_its_last_good_read() {
        let dir = tempdir("torn");
        let path = dir.join("777.json");
        std::fs::write(
            &path,
            record("sess-a", "Tue Sep 29 09:34:36 2026", Some("idle"), 5),
        )
        .unwrap();
        let procs = Fake(HashMap::from([(777, alive(1_790_674_476, 1))]));
        let mut reader = Reader::new();
        assert_eq!(
            reader.read(&dir, 0, &procs).by_session["sess-a"].status,
            Some(RegStatus::Idle)
        );

        // Caught mid-write on both attempts: the last good parse stands in.
        std::fs::write(&path, "").unwrap();
        let snap = reader.read(&dir, 1, &procs);
        assert_eq!(snap.by_session["sess-a"].status, Some(RegStatus::Idle));

        // With nothing remembered, a torn record is skipped, not guessed at.
        assert!(Reader::new().read(&dir, 2, &procs).by_session.is_empty());

        // Gone is gone: once the file is unlinked its memory goes too.
        std::fs::remove_file(&path).unwrap();
        assert!(reader.read(&dir, 3, &procs).by_session.is_empty());
        assert!(reader.last_good.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn our_own_process_verifies_against_its_real_start_time() {
        let dir = tempdir("own");
        let me = std::process::id();
        std::fs::write(
            dir.join(format!("{me}.json")),
            record("sess-own", &ctime(own_start_secs()), Some("idle"), 1),
        )
        .unwrap();
        let snap = snapshot_in(&dir, 0);
        // snapshot_in excludes processes the router spawned; this one *is* the router,
        // and its parent is cargo, so it stays.
        assert!(
            snap.by_session.contains_key("sess-own"),
            "{:?}",
            snap.by_session
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reaped_childs_pid_is_dead() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(System.info(pid), ProcState::Dead);

        let dir = tempdir("reaped");
        std::fs::write(
            dir.join(format!("{pid}.json")),
            record("sess-dead", "Tue Sep 29 09:34:36 2026", Some("busy"), 1),
        )
        .unwrap();
        assert!(
            snapshot_in(&dir, 0).by_session.is_empty(),
            "a stale record is not a session"
        );
        assert!(
            dir.join(format!("{pid}.json")).exists(),
            "and it is never deleted"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_zombie_is_dead() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        // Exited but not yet reaped: the kernel still has its entry.
        let deadline = Instant::now() + Duration::from_secs(5);
        while System.info(pid) != ProcState::Dead && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(System.info(pid), ProcState::Dead);
        child.wait().unwrap();
    }

    #[test]
    fn a_start_time_we_cannot_read_is_no_verdict() {
        // A lone session whose procStart is in some format this parser does not know:
        // judged by liveness, not closed.
        let dir = tempdir("format");
        std::fs::write(
            dir.join("951.json"),
            record("lone", "2026-09-29T09:34:36Z", Some("idle"), 1),
        )
        .unwrap();
        let procs = Fake(HashMap::from([(951, alive(1_790_674_476, 1))]));
        assert!(Reader::new()
            .read(&dir, 0, &procs)
            .by_session
            .contains_key("lone"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_live_pid_with_the_wrong_start_is_someone_else() {
        let dir = tempdir("reused");
        let me = std::process::id();
        // Our pid, but a start an hour before ours: the pid was reused.
        std::fs::write(
            dir.join(format!("{me}.json")),
            record("sess-old", &ctime(own_start_secs() - 3600), Some("busy"), 1),
        )
        .unwrap();
        assert!(snapshot_in(&dir, 0).by_session.is_empty());
        // Within a second either way is the same process: lstart has 1 s resolution.
        std::fs::write(
            dir.join(format!("{me}.json")),
            record("sess-old", &ctime(own_start_secs() + 1), Some("busy"), 1),
        )
        .unwrap();
        assert_eq!(snapshot_in(&dir, 0).by_session.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn processes_the_router_spawned_are_not_sessions() {
        let dir = tempdir("ppid");
        let ps = "Tue Sep 29 09:34:36 2026";
        let t = 1_790_674_476;
        std::fs::write(
            dir.join("501.json"),
            record("fork-child", ps, Some("busy"), 1),
        )
        .unwrap();
        std::fs::write(
            dir.join("502.json"),
            record("fork-grandchild", ps, Some("busy"), 1),
        )
        .unwrap();
        std::fs::write(
            dir.join("503.json"),
            record("real-session", ps, Some("idle"), 1),
        )
        .unwrap();
        let procs = Fake(HashMap::from([
            (100, alive(t, 1)),   // the router
            (501, alive(t, 100)), // claude -p, run by the router
            (600, alive(t, 100)), // a shell the router ran
            (502, alive(t, 600)), // claude -p under that shell
            (503, alive(t, 7)),   // an editor's session
        ]));
        let mut reader = Reader::new();
        reader.self_pid = 100;
        let snap = reader.read(&dir, 0, &procs);
        assert_eq!(
            snap.by_session.keys().collect::<Vec<_>>(),
            vec!["real-session"]
        );
        // Named, so the list can leave the forks' own transcripts out while they run.
        let mut forks: Vec<_> = snap.spawned.keys().cloned().collect();
        forks.sort();
        assert_eq!(forks, ["fork-child", "fork-grandchild"]);
        assert_eq!(snap.spawned["fork-child"], Some(1_790_674_477_325));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_real_child_of_ours_is_excluded() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let start = match System.info(pid) {
            ProcState::Alive {
                start_secs: Some(s),
                ppid,
            } => {
                assert_eq!(ppid, Some(std::process::id()));
                s
            }
            other => panic!("{other:?}"),
        };
        let dir = tempdir("child");
        std::fs::write(
            dir.join(format!("{pid}.json")),
            record("fork", &ctime(start), Some("busy"), 1),
        )
        .unwrap();
        assert!(
            snapshot_in(&dir, 0).by_session.is_empty(),
            "our own child is our /btw fork"
        );
        let _ = child.kill();
        let _ = child.wait();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn when_every_live_pid_fails_the_start_check_the_parser_is_suspect() {
        let dir = tempdir("guard");
        let ps = "Tue Sep 29 09:34:36 2026";
        for (pid, s) in [(701, "a"), (702, "b"), (703, "c")] {
            std::fs::write(
                dir.join(format!("{pid}.json")),
                record(s, ps, Some("idle"), 1),
            )
            .unwrap();
        }
        // Every live start four hours off — a time zone read the wrong way, say.
        let off = 1_790_674_476 - 4 * 3600;
        let procs = Fake(HashMap::from([
            (701, alive(off, 1)),
            (702, alive(off, 1)),
            (703, alive(off, 1)),
        ]));
        let mut reader = Reader::new();
        let snap = reader.read(&dir, 0, &procs);
        assert_eq!(
            snap.by_session.len(),
            3,
            "liveness only, rather than every session closed"
        );
        assert!(reader.warned);

        // One real match among them: the mismatches are genuine reused pids.
        let procs = Fake(HashMap::from([
            (701, alive(1_790_674_476, 1)),
            (702, alive(off, 1)),
            (703, alive(off, 1)),
        ]));
        let snap = Reader::new().read(&dir, 0, &procs);
        assert_eq!(snap.by_session.keys().collect::<Vec<_>>(), vec!["a"]);

        // A single mismatch is never enough to distrust ourselves.
        std::fs::remove_file(dir.join("702.json")).unwrap();
        std::fs::remove_file(dir.join("703.json")).unwrap();
        let procs = Fake(HashMap::from([(701, alive(off, 1))]));
        assert!(Reader::new().read(&dir, 0, &procs).by_session.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_conversation_held_twice_goes_to_the_newest_status() {
        let dir = tempdir("dupe");
        let ps = "Tue Sep 29 09:34:36 2026";
        std::fs::write(dir.join("801.json"), record("same", ps, Some("idle"), 100)).unwrap();
        std::fs::write(dir.join("802.json"), record("same", ps, Some("busy"), 200)).unwrap();
        let t = 1_790_674_476;
        let procs = Fake(HashMap::from([(801, alive(t, 1)), (802, alive(t, 1))]));
        let snap = Reader::new().read(&dir, 0, &procs);
        assert_eq!(snap.by_session["same"].pid, 802);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_strange_status_is_not_idle() {
        let dir = tempdir("status");
        let ps = "Tue Sep 29 09:34:36 2026";
        std::fs::write(dir.join("901.json"), record("fresh", ps, None, 1)).unwrap();
        std::fs::write(
            dir.join("902.json"),
            record("odd", ps, Some("compacting"), 1),
        )
        .unwrap();
        let t = 1_790_674_476;
        let procs = Fake(HashMap::from([(901, alive(t, 1)), (902, alive(t, 1))]));
        let snap = Reader::new().read(&dir, 0, &procs);
        assert_eq!(snap.by_session["fresh"].status, None);
        let odd = snap.by_session["odd"].status.clone().unwrap();
        assert_eq!(odd, RegStatus::Unknown("compacting".into()));
        assert!(!odd.is_known());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_directory_means_no_registry() {
        let snap = snapshot_in(Path::new("/nonexistent/agent-router/sessions"), 7);
        assert!(!snap.available && snap.by_session.is_empty());
    }

    #[test]
    fn civil_dates_round_trip() {
        for days in [-719_468i64, -1, 0, 1, 10_957, 20_725, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
