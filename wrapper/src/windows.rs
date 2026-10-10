//! The wrapper on Windows. The control socket becomes a named pipe, and a job object and a
//! console handler stand in for the `signals` module. Everything else is shared with unix:
//! the relay, the recorder and its records, and `serve_connection`, which reads the frame,
//! delivers it and writes the receipt on both.
//!
//! # The pipe
//!
//! `\\.\pipe\agent-router-<user>-<session id>` (see `pipe_name`) carries exactly what the unix
//! socket carries. One connection takes one frame: a line of at most 1 MiB with its newline,
//! delivered whole or not at all. Back comes `{"ok":true}` once it is in the CLI's stdin, or
//! `{"ok":false}`, then the end.
//!
//! Every instance is made with a DACL that grants this process's user everything and nobody
//! else anything, with inherited entries blocked, and it turns away clients on other
//! machines. An elevated wrapper also labels it high integrity, no-write-up, so that only
//! elevated processes of that user can type into an elevated CLI. The first instance of a
//! name is made with `FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails when anyone already has that
//! name. A name in use is never joined, and that is the one difference from unix: there a
//! leftover socket is unlinked and bound over, but a pipe leaves nothing behind (it goes when
//! its server does), so a name in use is a live process's. Until the listener has its name it
//! tries again every 3 s, as the unix one rebinds.
//!
//! Once it has the name, it never lets go of it on purpose, since another process could take
//! a name that lapsed. The listener always keeps one instance waiting. The next one is made,
//! under the same name, before a connected one goes to its thread. Whatever fails is tried
//! again at the next look, 200 ms on, with the instance that listens kept meanwhile. With
//! nothing left listening, the name is only ever taken afresh, as a first instance, never by
//! joining a pipe this process may no longer hold.
//!
//! Each connection has an instance of its own, closed once the receipt is written: the client
//! reads the receipt, then the end (`ERROR_BROKEN_PIPE`). Never `DisconnectNamedPipe`, which
//! throws away what the client has not read yet. An instance stopped while waiting is asked
//! a few times whether a client got in, so that one is still answered. Two things a socket
//! does not need: a read of nothing (a client's zero-byte write) is not the end here, and a
//! read or write that stalls for 10 s is called off, since every connection holds one of the
//! 255 instances a name can have.
//!
//! A pipe cannot be renamed the way a socket file is. So when the CLI names a new session
//! (after `/clear`), the move makes the new name's first instance itself, under the name lock
//! and before it renames the records, and hands it to the listener. From then on the listener
//! listens there and lets the old name go. Taking the new name and moving the records stay
//! one step that happens whole or not at all, as the rename is on unix.
//!
//! # Ending with the editor
//!
//! The editor ends a session with "end stdin, wait 2 s, kill", and on Windows the kill is
//! `TerminateProcess`: no handler runs in this process, nothing is passed on, nothing is
//! cleaned up. The EOF is relayed as on unix, and that is all an idle CLI needs. A busy CLI is
//! held by a job object that kills what is in it when its last handle closes, and this
//! process holds the only handle. However this process ends, the CLI ends with it, as it
//! would have unwrapped when the editor killed it. The job is the one libuv gives every child
//! the editor's Node spawns: only the CLI is in it, and what the CLI starts is its own
//! business (a `.cmd` or `.bat` real binary is the exception, see `tie_to`). The pipe goes
//! with the process, and the records stay behind for the router's janitor, as a killed
//! wrapper's do on unix.
//!
//! The relay and the stdin pump are shared too, with one Windows difference: std reads a
//! zero-byte write on a pipe as nothing, which on unix only the end of the stream reads as.
//! So on Windows nothing read is the end only once the writer is gone (see `ended`).
//!
//! Console events reach every process on the console, the CLI included. Ctrl+C and Ctrl+Break
//! are the CLI's to act on, so this process does not die of them first. A console closing
//! holds this process until the CLI has gone and the way out has run, or until the system ends
//! it; so do a logoff and a shutdown, which only reach a process a service started.

use std::ffi::c_void;
use std::io::{self, ErrorKind, Read, Write};
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, SetHandleInformation, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE,
    ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_SEM_TIMEOUT, FALSE, HANDLE,
    HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STATUS_PENDING, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PeekNamedPipe, WaitNamedPipeW, NMPWAIT_NOWAIT,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::{
    lock, owner_only_sddl, pipe_name, pipe_name_part, record_path, serve_connection,
    sessions_dir_from, valid_id, Conn, Current, Sink, Up,
};

/// Read and write, overlapped so that waiting for a client can stop to look around.
const OPEN_MODE: u32 = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;

/// A byte stream, as a socket is, and only for clients on this machine. Windows wants the
/// same type on every instance of a name; this is the same on all of them.
const PIPE_MODE: u32 = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;

/// Each direction's buffer, a hint the system grows past for a bigger frame. A writer of
/// more than this waits for the reader, as it would on a socket.
const BUFFER: u32 = 64 * 1024;

/// How long a client's `WaitNamedPipeW(NMPWAIT_USE_DEFAULT_WAIT)` waits for a free instance:
/// the router's own connect timeout. Every instance of a name must give the same value.
const DEFAULT_WAIT_MS: u32 = 3_000;

/// How often a listener with no client looks at the name in force, as the unix one polls
/// its accept, and how often one with nothing to listen on tries again.
const POLL_MS: u32 = 200;

/// How many times a listener that stops asks whether a client got in meanwhile (see
/// `Listener::stop`).
const STOP_LOOKS: usize = 3;

/// The user's sessions directory, `%LOCALAPPDATA%\agent-router\sessions` (see
/// `sessions_dir_from`), decided once.
pub fn sessions_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        sessions_dir_from(
            std::env::var_os("LOCALAPPDATA"),
            std::env::var_os("USERPROFILE"),
        )
    })
    .as_deref()
}

/// The pipe name a session's channel answers under, for the records in `dir`: the
/// documented name for the user's own sessions directory, a lab's own for any other (see
/// `pipe_name`). `None` when the user name cannot be had.
fn pipe_name_in(dir: &Path, session_id: &str) -> Option<String> {
    let lab = (sessions_dir() != Some(dir)).then(|| {
        pipe_name_part(
            &dir.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
        )
    });
    Some(pipe_name(user()?, lab.as_deref(), session_id))
}

/// This process's user name, as `GetUserNameW` gives it, cut down to a piece of a pipe name;
/// decided once.
fn user() -> Option<&'static str> {
    static USER: OnceLock<Option<String>> = OnceLock::new();
    USER.get_or_init(|| {
        // UNLEN, the longest user name, and its terminating nul.
        let mut buf = [0u16; 257];
        let mut len = buf.len() as u32;
        // SAFETY: a buffer of `len` characters.
        if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        let name = buf.split(|&c| c == 0).next().unwrap_or_default();
        Some(pipe_name_part(&String::from_utf16_lossy(name)))
    })
    .as_deref()
}

/// The security descriptor every instance is made with (see `owner_only_sddl`). Built once
/// and never freed; it is needed for as long as the process serves.
struct OwnerOnly(PSECURITY_DESCRIPTOR);

// SAFETY: a pointer to memory that nothing writes or frees once it is built.
unsafe impl Send for OwnerOnly {}
// SAFETY: as above.
unsafe impl Sync for OwnerOnly {}

/// The owner-only descriptor, or `None` if it cannot be built, and then there is no pipe:
/// the default descriptor would let everyone read from it.
fn owner_only() -> Option<&'static OwnerOnly> {
    static SD: OnceLock<Option<OwnerOnly>> = OnceLock::new();
    SD.get_or_init(|| {
        let (sid, elevated) = token_user()?;
        let sddl = wide(&owner_only_sddl(&sid, elevated));
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: a nul-terminated SDDL string, and a place for the descriptor it allocates.
        let built = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        } != 0;
        (built && !sd.is_null()).then_some(OwnerOnly(sd))
    })
    .as_ref()
}

/// This process's user as a SID string (`S-1-5-21-…`), and whether its token is elevated.
fn token_user() -> Option<(String, bool)> {
    let mut token: HANDLE = null_mut();
    // SAFETY: this process's pseudo-handle, and a place for the token handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    // SAFETY: a handle OpenProcessToken just gave us, owned by nothing else.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut len = 0u32;
    // SAFETY: a size query: no buffer, and `len` told how much one needs.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut len) };
    if len == 0 {
        return None;
    }
    // In u64s, so that the TOKEN_USER at its start is aligned for the pointer in it.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: a buffer of at least `len` bytes.
    let filled = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    } != 0;
    if !filled {
        return None;
    }
    // SAFETY: the buffer holds a TOKEN_USER, and the SID it points to lies within the buffer.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = null_mut();
    // SAFETY: a valid SID, and a place for the string it allocates (freed below).
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return None;
    }
    // SAFETY: a nul-terminated string ConvertSidToStringSidW allocated.
    let sid = unsafe { from_wide_nul(text) };
    // SAFETY: allocated with LocalAlloc by ConvertSidToStringSidW, and not used after this.
    unsafe { LocalFree(text.cast()) };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    // SAFETY: a TOKEN_ELEVATION, and its size.
    let known = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    } != 0;
    // Not known is taken as elevated, which can only narrow who may write. A process that is
    // not, and so cannot set that label, then makes no pipe: a plain relay.
    Some((sid, !known || elevation.TokenIsElevated != 0))
}

/// One instance of the pipe `name`. `first`: the name's first instance, which fails if any
/// process has the name already; otherwise one more instance of a name this process holds.
fn create_instance(name: &str, first: bool) -> io::Result<OwnedHandle> {
    let security = owner_only().ok_or_else(|| io::Error::other("no owner-only descriptor"))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.0,
        bInheritHandle: FALSE,
    };
    let open = OPEN_MODE
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let name = wide(name);
    // SAFETY: a nul-terminated name and security attributes, both alive for the call.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open,
            PIPE_MODE,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            DEFAULT_WAIT_MS,
            &attributes,
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a handle CreateNamedPipeW just returned, owned by nothing else.
    Ok(unsafe { OwnedHandle::from_raw_handle(pipe) })
}

/// Whether some process serves a pipe named `name`, with an instance free or all of them
/// busy. Asked without connecting, so it costs whoever serves it nothing.
fn pipe_in_use(name: &str) -> bool {
    let name = wide(name);
    // SAFETY: a nul-terminated name, alive for the call.
    if unsafe { WaitNamedPipeW(name.as_ptr(), NMPWAIT_NOWAIT) } != 0 {
        return true;
    }
    is(&io::Error::last_os_error(), ERROR_SEM_TIMEOUT)
}

/// An unnamed manual-reset event, for one overlapped operation at a time.
fn manual_event() -> io::Result<OwnedHandle> {
    // SAFETY: default security, no name.
    let event = unsafe { CreateEventW(null(), TRUE, FALSE, null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a handle CreateEventW just returned, owned by nothing else.
    Ok(unsafe { OwnedHandle::from_raw_handle(event) })
}

/// The listeners serving in this process, by records directory, each with what moves have
/// handed it: each new name with that name's first instance, newest last.
static LISTENERS: Mutex<Vec<Listening>> = Mutex::new(Vec::new());

struct Listening {
    dir: PathBuf,
    handed: Vec<(String, OwnedHandle)>,
}

/// A listener's place in [`LISTENERS`], for as long as it listens.
struct Registered(PathBuf);

impl Registered {
    fn new(dir: &Path) -> Registered {
        lock(&LISTENERS).push(Listening {
            dir: dir.to_path_buf(),
            handed: Vec::new(),
        });
        Registered(dir.to_path_buf())
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        // A name handed over and never taken up goes with it, and is free again.
        lock(&LISTENERS).retain(|l| l.dir != self.0);
    }
}

/// What moves have handed the listener for `dir` since it last looked, oldest first.
fn take_handed(dir: &Path) -> Vec<(String, OwnedHandle)> {
    lock(&LISTENERS)
        .iter_mut()
        .find(|l| l.dir == dir)
        .map(|l| std::mem::take(&mut l.handed))
        .unwrap_or_default()
}

/// Move a session's records and its pipe name from `old` to `new`: all of it, or none.
///
/// The records are renamed as on unix. A pipe cannot be renamed, so the move makes the new
/// name's first instance instead, which fails if any process has that name, and hands it to
/// the listener, which takes it up and lets the old name go. A name in use is refused, as on
/// unix a socket already under the new name is. With no listener yet, the name is only
/// looked at: the listener takes whatever name is in force when it starts.
pub fn move_records(dir: &Path, old: &str, new: &str) -> io::Result<()> {
    if new == old || valid_id(new.as_bytes()).is_none() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a session id to move to",
        ));
    }
    let taken = || io::Error::new(ErrorKind::AlreadyExists, "that name is taken");
    let mut listeners = lock(&LISTENERS);
    let listening = listeners.iter_mut().find(|l| l.dir == dir);
    let claimed = match (pipe_name_in(dir, new), listening.is_some()) {
        (Some(name), true) => Some(create_instance(&name, true).map_err(|e| {
            if is(&e, ERROR_ACCESS_DENIED) {
                taken()
            } else {
                e
            }
        })?),
        (Some(name), false) if pipe_in_use(&name) => return Err(taken()),
        _ => None,
    };
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for ext in ["ctl", "pending"] {
        let from = record_path(dir, old, ext);
        let to = record_path(dir, new, ext);
        match std::fs::rename(&from, &to) {
            Ok(()) => moved.push((from, to)),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                for (from, to) in moved.into_iter().rev() {
                    let _ = std::fs::rename(&to, &from);
                }
                // The new name's instance closes with `claimed`, and the name is free again.
                return Err(e);
            }
        }
    }
    if let (Some(listening), Some(pipe)) = (listening, claimed) {
        listening.handed.push((new.to_string(), pipe));
    }
    Ok(())
}

/// Accept injected frames on the session's pipe and write them into the child's stdin.
///
/// Every failure here is swallowed, as on unix: the session works without it.
pub fn serve_control(dir: &Path, current: &Current, sink: &Sink, up: &Up) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    // Until it has the name (another process may hold it, or no instance can be had), the
    // listener tries again every 3 s, as the unix one rebinds. Once it has it, it never
    // returns.
    loop {
        serve_once(dir, current, sink, up);
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn serve_once(dir: &Path, current: &Current, sink: &Sink, up: &Up) {
    // The first instance under the name in force, made under the name lock, with this
    // listener registered for moves in the same step: a move either hands it the new name or
    // came before it took one.
    let (mut name, first, _registered) = {
        let slot = lock(current);
        let Some(id) = slot.as_deref() else {
            return;
        };
        let Some(pipe) = pipe_name_in(dir, id) else {
            return;
        };
        let Ok(first) = create_instance(&pipe, true) else {
            return;
        };
        (id.to_string(), first, Registered::new(dir))
    };
    // From here on the name is never let go of on purpose. Whatever fails is tried again at
    // the next look, with the instance that listens kept listening meanwhile: let go, the
    // name could be taken by another process before this one had it back.
    let mut listener = Listener::new(first).ok();
    loop {
        let connected = match listener.as_mut().map(|l| l.poll(POLL_MS)) {
            Some(Ok(connected)) => connected,
            // Broken: let go of it, a client that got in still served, and make another.
            Some(Err(_)) => {
                if let Some(late) = listener.take().and_then(Listener::stop) {
                    serve(late, sink, up);
                }
                None
            }
            None => {
                std::thread::sleep(Duration::from_millis(u64::from(POLL_MS)));
                None
            }
        };
        let listening = listener.is_some() && connected.is_none();
        let renewal = renew(dir, current, &name, listening, connected.is_some());
        // Instances handed over for names since moved on: each still answers a client that
        // got in before it closes.
        for pipe in renewal.superseded {
            if let Some(late) = Listener::new(pipe).ok().and_then(Listener::stop) {
                serve(late, sink, up);
            }
        }
        match renewal
            .next
            .map(|next| next.and_then(|(id, pipe)| Ok((id, Listener::new(pipe)?))))
        {
            Some(Ok((id, next))) => {
                name = id;
                // The old listener is spent if it just connected. Otherwise it was listening
                // under a name that has moved on, and a client that got in meanwhile is still
                // served.
                if let Some(late) = listener.replace(next).and_then(Listener::stop) {
                    serve(late, sink, up);
                }
            }
            // Not to be had this time, tried again at the next look. A listener that just
            // connected is spent and goes; one still listening carries on.
            Some(Err(_)) if connected.is_some() => listener = None,
            Some(Err(_)) | None => {}
        }
        if let Some(conn) = connected {
            serve(conn, sink, up);
        }
    }
}

/// What the listener is to do next, decided under the name lock.
struct Renewal {
    /// The instance to listen on from now on, and the session id it is under. `None`: carry
    /// on where it is.
    next: Option<io::Result<(String, OwnedHandle)>>,
    /// Instances moves handed over for names since moved on.
    superseded: Vec<OwnedHandle>,
}

/// The instance to listen on next, chosen under the name lock:
///
///   * the newest one a move handed over;
///   * a first one under the name in force, when that moved on with no hand-over;
///   * once the instance listening has a client, the same name's next. The client holds the
///     name until it is served, so the new instance can only join this process's own pipe;
///   * with nothing listening (a failure before), a first one under the name: never one
///     that joins a pipe this process may no longer hold, whose DACL would be another's.
fn renew(dir: &Path, current: &Current, name: &str, listening: bool, connected: bool) -> Renewal {
    let slot = lock(current);
    let mut handed = take_handed(dir);
    let newest = handed.pop();
    let superseded = handed.into_iter().map(|(_, pipe)| pipe).collect();
    let instance = |id: &str, first: bool| -> io::Result<(String, OwnedHandle)> {
        let pipe = pipe_name_in(dir, id).ok_or(ErrorKind::NotFound)?;
        Ok((id.to_string(), create_instance(&pipe, first)?))
    };
    let next = match (newest, slot.as_deref()) {
        (Some(newest), _) => Some(Ok(newest)),
        (None, Some(id)) if id != name => Some(instance(id, true)),
        (None, _) if connected => Some(instance(name, false)),
        (None, _) if !listening => Some(instance(name, true)),
        (None, _) => None,
    };
    Renewal { next, superseded }
}

/// One connection, served on a thread of its own as on unix. A thread that cannot be had
/// costs this one connection (closed, so its client reads the end), never the listener.
fn serve(pipe: OwnedHandle, sink: &Sink, up: &Up) {
    let Ok(conn) = PipeConn::new(pipe) else {
        return;
    };
    let sink = Arc::clone(sink);
    let up = Arc::clone(up);
    let _ = std::thread::Builder::new().spawn(move || serve_connection(conn, &sink, &up));
}

/// Where an overlapped connect stands.
#[derive(Clone, Copy, PartialEq)]
enum Wait {
    /// Started; the system may still write into the OVERLAPPED.
    Pending,
    /// A client is there, its instance not handed out yet.
    Connected,
    /// Nothing outstanding.
    Over,
}

/// The instance waiting for the next client: an overlapped `ConnectNamedPipe`, waited on in
/// steps so that the listener can look up in between.
struct Listener {
    pipe: Option<OwnedHandle>,
    event: OwnedHandle,
    /// Lent to the system for the connect, and freed only once that is shown to be over.
    ov: *mut OVERLAPPED,
    state: Wait,
}

impl Listener {
    /// Start waiting for a client on `pipe`.
    fn new(pipe: OwnedHandle) -> io::Result<Listener> {
        let event = manual_event()?;
        let mut listener = Listener {
            pipe: Some(pipe),
            event,
            ov: Box::into_raw(Box::new(OVERLAPPED::default())),
            state: Wait::Over,
        };
        listener.listen()?;
        Ok(listener)
    }

    /// Ask for the next client with `ConnectNamedPipe`, which is then outstanding or has
    /// found one there already. Only with nothing outstanding (`Over`).
    fn listen(&mut self) -> io::Result<()> {
        // SAFETY: nothing is outstanding, so the system is done with the OVERLAPPED.
        unsafe {
            *self.ov = OVERLAPPED {
                hEvent: self.event.as_raw_handle(),
                ..OVERLAPPED::default()
            };
        }
        // The state is set from what the call says, never before it. Only ERROR_IO_PENDING
        // leaves an operation outstanding. After any other answer the OVERLAPPED can still
        // read STATUS_PENDING, since a call that fails on the spot never writes it, and waiting
        // on that would wait for ever.
        // SAFETY: an instance of ours, and an OVERLAPPED that outlives the connect (see Drop).
        if unsafe { ConnectNamedPipe(self.raw(), self.ov) } != 0 {
            self.state = Wait::Connected;
            return Ok(());
        }
        let e = io::Error::last_os_error();
        self.state = match e.raw_os_error().map(|code| code as u32) {
            Some(ERROR_IO_PENDING) => Wait::Pending,
            // A client got in before this, which is a good connection. Or it got in and left
            // again: served all the same, so what it wrote goes in as on unix (nothing, for a
            // bare connect), with nobody left to read the receipt.
            Some(ERROR_PIPE_CONNECTED | ERROR_NO_DATA) => Wait::Connected,
            _ => return Err(e),
        };
        Ok(())
    }

    fn raw(&self) -> HANDLE {
        self.pipe
            .as_ref()
            .map_or(INVALID_HANDLE_VALUE, |p| p.as_raw_handle())
    }

    /// Wait up to `ms` for a client: its instance once there is one, `None` meanwhile.
    fn poll(&mut self, ms: u32) -> io::Result<Option<OwnedHandle>> {
        match self.state {
            Wait::Connected => {
                self.state = Wait::Over;
                return Ok(self.pipe.take());
            }
            Wait::Over => return Ok(None),
            Wait::Pending => {}
        }
        // SAFETY: an event of ours.
        match unsafe { WaitForSingleObject(self.event.as_raw_handle(), ms) } {
            // An outstanding connect only ever ends in a client or in an error.
            WAIT_OBJECT_0 => {
                self.settle()?;
                Ok(self.pipe.take())
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Stop waiting, and give back the instance if a client got in first, so that it is still
    /// served.
    ///
    /// An instance whose wait is called off still takes clients until it is closed. So it is
    /// asked again, a few times, whether one got in. A client that gets in after the last look
    /// reads the end with no receipt, which the router takes for "not delivered", and nothing
    /// was.
    fn stop(mut self) -> Option<OwnedHandle> {
        self.pipe.as_ref()?;
        for _ in 0..STOP_LOOKS {
            match self.state {
                Wait::Connected => return self.pipe.take(),
                Wait::Pending => {
                    // SAFETY: the instance and OVERLAPPED of the connect outstanding.
                    unsafe { CancelIoEx(self.raw(), self.ov) };
                    // Over without an error: a client got in before the cancel.
                    if self.settle().is_ok() {
                        return self.pipe.take();
                    }
                    if self.state == Wait::Pending {
                        // Never shown to be over: nothing more is asked of it (see Drop).
                        return None;
                    }
                }
                Wait::Over => {}
            }
            if self.listen().is_err() {
                return None;
            }
        }
        // A connect still outstanding is called off and waited out as `self` drops.
        match self.state {
            Wait::Connected => self.pipe.take(),
            Wait::Pending | Wait::Over => None,
        }
    }

    /// Wait for the outstanding connect to be over: how it ended.
    fn settle(&mut self) -> io::Result<()> {
        let mut transferred = 0u32;
        // SAFETY: the instance and OVERLAPPED the connect was started with.
        let ok = unsafe { GetOverlappedResult(self.raw(), self.ov, &mut transferred, TRUE) } != 0;
        let result = if ok {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
        if !still_pending(self.ov) {
            self.state = Wait::Over;
        }
        result
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if self.state == Wait::Pending {
            // SAFETY: as in `stop`.
            unsafe { CancelIoEx(self.raw(), self.ov) };
            let _ = self.settle();
        }
        if self.state == Wait::Pending {
            // Never shown to be over, so the system may still write into it: leaked, rather
            // than freed under it.
            return;
        }
        // SAFETY: made by Box::into_raw in `new`, and the system is done with it.
        drop(unsafe { Box::from_raw(self.ov) });
    }
}

/// Whether the system has yet to finish with `ov`, read as `HasOverlappedIoCompleted` does:
/// the low 32 bits, where the system writes the status.
fn still_pending(ov: *const OVERLAPPED) -> bool {
    // SAFETY: a live OVERLAPPED; volatile, since the system writes it from outside.
    let status = unsafe { std::ptr::read_volatile(&raw const (*ov).Internal) };
    status as u32 == STATUS_PENDING as u32
}

/// One client's connection: an instance of its own, read and written with overlapped I/O
/// waited out to the end, which is what a blocking socket does.
struct PipeConn {
    /// `None` once hung up.
    pipe: Option<OwnedHandle>,
    event: OwnedHandle,
}

impl PipeConn {
    fn new(pipe: OwnedHandle) -> io::Result<PipeConn> {
        Ok(PipeConn {
            pipe: Some(pipe),
            event: manual_event()?,
        })
    }

    /// One overlapped read or write, waited out to the end, or called off once it has stalled
    /// for [`STALL_MS`]. The OVERLAPPED is on this thread's stack and the buffer on its
    /// caller's: should the operation ever not be shown to be over, this thread never returns,
    /// so neither is freed while the system may still write into it. It costs one connection's
    /// thread, never the session.
    fn complete(
        &mut self,
        start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL,
    ) -> io::Result<usize> {
        let pipe = self
            .pipe
            .as_ref()
            .ok_or(ErrorKind::NotConnected)?
            .as_raw_handle();
        let mut ov = OVERLAPPED {
            hEvent: self.event.as_raw_handle(),
            ..OVERLAPPED::default()
        };
        if start(pipe, &raw mut ov) == 0 {
            let e = io::Error::last_os_error();
            if !is(&e, ERROR_IO_PENDING) {
                // Never started: nothing writes into `ov`.
                return Err(e);
            }
            // SAFETY: an event of ours, which the system sets once the operation is over.
            if unsafe { WaitForSingleObject(self.event.as_raw_handle(), STALL_MS) } != WAIT_OBJECT_0
            {
                // Stalled: called off, and the wait below sees the cancellation through.
                // SAFETY: the operation just started on this instance, with this OVERLAPPED.
                unsafe { CancelIoEx(pipe, &raw const ov) };
            }
        }
        let mut transferred = 0u32;
        // SAFETY: the instance and OVERLAPPED the operation was started with.
        let ok = unsafe { GetOverlappedResult(pipe, &raw const ov, &mut transferred, TRUE) } != 0;
        let result = if ok {
            Ok(transferred as usize)
        } else {
            Err(io::Error::last_os_error())
        };
        if still_pending(&raw const ov) {
            loop {
                std::thread::park();
            }
        }
        result
    }
}

/// How long one read or write on a connection may stall before it is called off. A client
/// that goes quiet would hold a thread here for ever, and one of the 255 instances a name
/// can have; the router writes its frame as soon as it connects and reads the receipt at
/// once, so only a client that is stuck ever waits this long.
const STALL_MS: u32 = 10_000;

impl Read for PipeConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let at = buf.as_mut_ptr();
        loop {
            // SAFETY: `len` bytes at `at`, alive until the read is over (see `complete`).
            let read = self.complete(|pipe, ov| unsafe { ReadFile(pipe, at, len, null_mut(), ov) });
            return match read {
                // A client's zero-byte write reads as nothing, which on a socket never happens.
                // Taken for the end, it would deliver half a frame as a whole one.
                Ok(0) => continue,
                // The client closed its end: the end of the stream, as EOF is on a socket.
                Err(e) if is(&e, ERROR_BROKEN_PIPE) => Ok(0),
                other => other,
            };
        }
    }
}

impl Write for PipeConn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let at = buf.as_ptr();
        // SAFETY: `len` bytes at `at`, alive until the write is over (see `complete`).
        self.complete(|pipe, ov| unsafe { WriteFile(pipe, at, len, null_mut(), ov) })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Conn for PipeConn {
    /// Closing the instance is the hang-up. The client still reads what was written, then
    /// `ERROR_BROKEN_PIPE`, which reads as the end. Not `DisconnectNamedPipe`, which would
    /// throw away a receipt the client has not read yet.
    fn hang_up(&mut self) {
        self.pipe = None;
    }
}

/// Before the CLI starts. The editor's pipes came to this process inheritable, and Windows
/// hands every inheritable handle to a child: cleared, the CLI gets only the stdio it is
/// given (see `main`). And the console's events are left to the CLI (see the module docs),
/// from before there is one, so none comes between its start and its tie.
pub fn prepare() {
    for handle in [
        std::io::stdin().as_raw_handle(),
        std::io::stdout().as_raw_handle(),
    ] {
        // SAFETY: this process's own standard handles; only whether a child inherits them
        // changes. A failure (no such handle) leaves it as it was.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    }
    // SAFETY: a handler that only answers or waits.
    unsafe { SetConsoleCtrlHandler(Some(on_console_event), TRUE) };
}

/// From now on the CLI ends with this process, however this process ends (see the module
/// docs). `real` is what was started for it.
///
/// The job is the one libuv puts every child it spawns in, which is how the editor's Node
/// starts an unwrapped CLI: killed when its last handle closes, no crash dialog left waiting
/// for a click, and only the process put in it. This process is in the editor's job, but what
/// it starts breaks away from that job silently, so without a job of its own the CLI would be
/// in none.
///
/// A script is the exception. It runs in cmd.exe with the CLI as cmd's child, which would
/// break away the same and be left running with nobody reading it. So for a script the whole
/// tree stays in, and only a process that asks to leave may. That leaves a race: should
/// cmd.exe start the CLI before it is put in the job, the CLI stays out. And Ctrl+C makes
/// cmd.exe ask "Terminate batch job (Y/N)?", answered from the CLI's stdin. The CLI the
/// editor ships is `claude.exe`; a script is supported, not recommended.
pub fn tie_to(child: &Child, real: &Path) {
    let script = real
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd"));
    let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    if !script {
        flags |= JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
    }
    // SAFETY: plain calls on handles this process owns. A failure leaves the CLI as it was:
    // untied, it sees its stdin close when the editor kills this process.
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            debug(&format!(
                "the CLI is not tied: no job ({})",
                io::Error::last_os_error()
            ));
            return;
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = flags;
        let tied = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0
            && AssignProcessToJobObject(job, child.as_raw_handle()) != 0;
        // Tied, the handle is never closed: it closes as this process ends, however that
        // happens, and the CLI is killed with it.
        if !tied {
            // A job this one cannot nest in (before Windows 8), or a CLI already gone.
            debug(&format!(
                "the CLI is not tied to this process: {}",
                io::Error::last_os_error()
            ));
            CloseHandle(job);
        }
    }
}

/// A line on stderr, with `AGENT_ROUTER_WRAPPER_DEBUG` set, as `main` writes its own.
fn debug(what: &str) {
    if std::env::var_os("AGENT_ROUTER_WRAPPER_DEBUG").is_some() {
        eprintln!("claude-wrapper: {what}");
    }
}

/// Whether reading nothing from `handle` was the end of the stream. On a pipe it can also be
/// the writer's zero-byte write, which on unix never shows; the end is the writer gone, which
/// a peek reports as ERROR_BROKEN_PIPE. On anything that is not a pipe, nothing is the end.
pub fn ended(handle: RawHandle) -> bool {
    let mut available = 0u32;
    // SAFETY: a peek at nothing, on a handle of this process's.
    let peeked = unsafe {
        PeekNamedPipe(
            handle,
            null_mut(),
            0,
            null_mut(),
            &mut available,
            null_mut(),
        )
    };
    peeked == 0
}

/// Console events come to every process on the console, the CLI as well as this one.
unsafe extern "system" fn on_console_event(event: u32) -> BOOL {
    match event {
        // The CLI's call: it has the same event. Should it end, this process follows the way
        // it always does, with the CLI's exit code.
        CTRL_C_EVENT | CTRL_BREAK_EVENT => TRUE,
        // The console is closing, and the system ends this process once this returns. Held
        // here, it ends as the CLI does instead, the way out run first, unless the system's
        // deadline (5 s) comes before. A logoff or a shutdown reaches only a process a service
        // started (an interactive one is ended without it), and is held the same way.
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        _ => FALSE,
    }
}

/// `s` as a nul-terminated UTF-16 string.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A nul-terminated UTF-16 string the system handed over.
///
/// # Safety
/// `p` points to a nul-terminated UTF-16 string.
unsafe fn from_wide_nul(p: *const u16) -> String {
    let mut len = 0;
    // SAFETY: the caller's promise: every unit up to the nul is readable.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units, read above.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// Whether `e` is the Win32 error `code`.
fn is(e: &io::Error, code: u32) -> bool {
    e.raw_os_error() == Some(code as i32)
}

#[cfg(test)]
mod tests {
    //! Compiled for Windows only, and run there with `cargo test -p claude-wrapper`. Every
    //! pipe here is named for a directory of its own (see `pipe_name_in`), never as a real
    //! session's.

    use super::*;
    use crate::{read_frame, Recorder, Shared};
    use std::os::windows::fs::OpenOptionsExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;

    const A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const B: &str = "bbbbbbbb-0000-4000-8000-000000000002";
    const C: &str = "cccccccc-0000-4000-8000-000000000003";

    /// A records directory of its own, and with it pipe names of its own.
    struct Lab(PathBuf);

    impl Lab {
        fn new() -> Lab {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "cwwin-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Lab(dir)
        }

        fn pipe(&self, id: &str) -> String {
            pipe_name_in(&self.0, id).expect("a user name")
        }
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The router's side of it: the pipe opened as a file, as std can, with the server
    /// allowed to identify the client and no more.
    fn connect(name: &str) -> io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name)
    }

    /// One frame in the way the router sends it; the reply, read to the end.
    fn inject(name: &str, frame: &str) -> io::Result<String> {
        let mut pipe = connect(name)?;
        pipe.write_all(format!("{frame}\n").as_bytes())?;
        let mut reply = String::new();
        pipe.read_to_string(&mut reply)?;
        Ok(reply)
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "never happened: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn accept(listener: &mut Listener) -> OwnedHandle {
        for _ in 0..50 {
            if let Some(pipe) = listener.poll(POLL_MS).unwrap() {
                return pipe;
            }
        }
        panic!("no client within 10 s");
    }

    #[test]
    fn the_user_and_the_owner_only_descriptor_can_be_had() {
        assert!(user().is_some_and(|u| !u.is_empty()), "{:?}", user());
        assert!(owner_only().is_some());
        assert!(token_user().is_some_and(|(sid, _)| sid.starts_with("S-1-")));
    }

    #[test]
    fn a_lab_directory_publishes_under_names_of_its_own() {
        let lab = Lab::new();
        let tag = pipe_name_part(&lab.0.file_name().unwrap().to_string_lossy());
        let name = lab.pipe(A);
        assert_eq!(
            name,
            format!(r"\\.\pipe\agent-router-{}-lab-{tag}-{A}", user().unwrap())
        );
        let dir = sessions_dir().expect("LOCALAPPDATA");
        assert!(dir.ends_with(r"agent-router\sessions"), "{}", dir.display());
        assert_eq!(
            pipe_name_in(dir, A).unwrap(),
            format!(r"\\.\pipe\agent-router-{}-{A}", user().unwrap())
        );
    }

    #[test]
    fn a_name_is_taken_once_and_joined_only_by_its_own_server() {
        let lab = Lab::new();
        let name = lab.pipe(A);
        assert!(!pipe_in_use(&name));
        let first = create_instance(&name, true).unwrap();
        assert!(pipe_in_use(&name));
        assert!(
            create_instance(&name, true).is_err(),
            "a second first instance"
        );
        let second = create_instance(&name, false).unwrap();
        drop((first, second));
        assert!(!pipe_in_use(&name), "gone with its last instance");
    }

    #[test]
    fn a_frame_comes_in_and_the_receipt_goes_back_before_the_end() {
        let lab = Lab::new();
        let name = lab.pipe(A);
        let mut listener = Listener::new(create_instance(&name, true).unwrap()).unwrap();
        let client = std::thread::spawn(move || inject(&name, r#"{"type":"user"}"#).unwrap());
        let mut conn = PipeConn::new(accept(&mut listener)).unwrap();
        assert_eq!(read_frame(&mut conn).unwrap(), b"{\"type\":\"user\"}\n");
        conn.write_all(b"{\"ok\":true}\n").unwrap();
        conn.hang_up();
        assert_eq!(
            client.join().unwrap(),
            "{\"ok\":true}\n",
            "the receipt, then the end"
        );
    }

    #[test]
    fn a_bare_connect_reads_as_an_empty_frame() {
        let lab = Lab::new();
        let name = lab.pipe(A);
        let mut listener = Listener::new(create_instance(&name, true).unwrap()).unwrap();
        drop(connect(&name).unwrap());
        let mut conn = PipeConn::new(accept(&mut listener)).unwrap();
        assert_eq!(
            read_frame(&mut conn).unwrap(),
            b"",
            "nothing, which types nothing"
        );
    }

    #[test]
    fn a_listener_that_stops_lets_its_name_go() {
        let lab = Lab::new();
        let name = lab.pipe(A);
        let listener = Listener::new(create_instance(&name, true).unwrap()).unwrap();
        assert!(pipe_in_use(&name));
        assert!(listener.stop().is_none(), "nobody came");
        assert!(!pipe_in_use(&name));
    }

    #[test]
    fn a_client_that_got_in_before_the_listener_stopped_is_still_handed_out() {
        let lab = Lab::new();
        let name = lab.pipe(A);
        let listener = Listener::new(create_instance(&name, true).unwrap()).unwrap();
        let client = connect(&name).unwrap();
        let pipe = listener
            .stop()
            .expect("the client's instance, to be served");
        drop((client, pipe));
        assert!(!pipe_in_use(&name));
    }

    #[test]
    fn a_second_move_before_the_listener_looks_keeps_both_instances_for_it() {
        let lab = Lab::new();
        let registered = Registered::new(&lab.0);
        move_records(&lab.0, A, B).unwrap();
        move_records(&lab.0, B, C).unwrap();
        let handed = take_handed(&lab.0);
        let ids: Vec<&str> = handed.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, [B, C], "oldest first: the listener takes the newest");
        assert!(pipe_in_use(&lab.pipe(B)) && pipe_in_use(&lab.pipe(C)));
        drop((handed, registered));
    }

    #[test]
    fn a_move_hands_the_listener_the_new_names_first_instance() {
        let lab = Lab::new();
        std::fs::write(record_path(&lab.0, A, "ctl"), "ctl\n").unwrap();
        let registered = Registered::new(&lab.0);
        move_records(&lab.0, A, B).unwrap();
        assert!(
            pipe_in_use(&lab.pipe(B)),
            "the new name is held from the move on"
        );
        assert!(record_path(&lab.0, B, "ctl").exists());
        assert!(!record_path(&lab.0, A, "ctl").exists());
        let mut handed = take_handed(&lab.0);
        let (id, pipe) = handed.pop().expect("handed over");
        assert!(handed.is_empty());
        assert_eq!(id, B);
        drop((pipe, registered));
        assert!(!pipe_in_use(&lab.pipe(B)));
    }

    #[test]
    fn a_move_refuses_a_name_in_use_and_keeps_the_old_one() {
        let lab = Lab::new();
        std::fs::write(record_path(&lab.0, A, "pending"), "p\n").unwrap();
        let someone_elses = create_instance(&lab.pipe(B), true).unwrap();
        // With no listener here yet, the name is looked at; with one, it is tried.
        let refused = move_records(&lab.0, A, B).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::AlreadyExists);
        let registered = Registered::new(&lab.0);
        let refused = move_records(&lab.0, A, B).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::AlreadyExists);
        assert!(take_handed(&lab.0).is_empty());
        assert_eq!(
            std::fs::read(record_path(&lab.0, A, "pending")).unwrap(),
            b"p\n"
        );
        assert!(!record_path(&lab.0, B, "pending").exists());
        drop((someone_elses, registered));
    }

    #[test]
    fn a_hook_names_the_session_from_a_windows_transcript_path() {
        let line = format!(
            r#"{{"type":"control_request","request_id":"h-1","request":{{"subtype":"hook_callback","callback_id":"hook_0","input":{{"session_id":"{A}","transcript_path":"C:\\Users\\me\\.claude\\projects\\C--work\\{A}.jsonl","cwd":"C:\\work","hook_event_name":"PreToolUse"}}}}}}"#
        );
        assert_eq!(crate::hook_session(line.as_bytes()).as_deref(), Some(A));
    }

    /// The whole listener, around a child that takes its stdin and does nothing with it.
    #[test]
    fn the_listener_answers_under_the_session_and_follows_it_when_it_moves() {
        let lab = Lab::new();
        let mut child = Command::new("findstr")
            .arg("never-matches-anything")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let sink: Sink = Arc::new(Shared::new(child.stdin.take()));
        let up: Up = Arc::new(Shared::new(Some(std::io::stdout())));
        let current: Current = Arc::new(Mutex::new(Some(A.to_string())));
        {
            let dir = lab.0.clone();
            let (current, sink, up) = (Arc::clone(&current), Arc::clone(&sink), Arc::clone(&up));
            std::thread::spawn(move || serve_control(&dir, &current, &sink, &up));
        }
        let turn = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        wait_until("the pipe under A", || pipe_in_use(&lab.pipe(A)));
        assert_eq!(inject(&lab.pipe(A), turn).unwrap(), "{\"ok\":true}\n");
        assert_eq!(
            inject(&lab.pipe(A), turn).unwrap(),
            "{\"ok\":true}\n",
            "one name, many clients"
        );
        assert_eq!(
            inject(&lab.pipe(A), "").unwrap(),
            "{\"ok\":false}\n",
            "an empty line types nothing"
        );

        // The CLI names another session, as after /clear.
        let mut recorder = Recorder::new(&lab.0, Arc::clone(&current));
        let init = format!(
            r#"{{"type":"system","subtype":"init","cwd":"C:\\lab","session_id":"{B}","tools":[]}}"#
        );
        recorder.observe(format!("{init}\n").as_bytes());
        assert_eq!(lock(&current).as_deref(), Some(B));
        assert!(
            pipe_in_use(&lab.pipe(B)),
            "answering under the new name from the move on"
        );
        wait_until("the old name to go", || !pipe_in_use(&lab.pipe(A)));
        assert_eq!(inject(&lab.pipe(B), turn).unwrap(), "{\"ok\":true}\n");
        assert!(
            connect(&lab.pipe(A)).is_err(),
            "nothing answers under the old name"
        );

        sink.close();
        let _ = child.kill();
        let _ = child.wait();
    }
}
