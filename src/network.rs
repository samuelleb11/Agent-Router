//! Listening beyond loopback: a stored setting, and a switch that can be thrown while
//! the router is running.
//!
//! The router always serves `127.0.0.1`. Widening it does not move that socket — it
//! adds a second listener on the wider address, and narrowing takes that listener away
//! again. Two sockets rather than one because `SO_REUSEPORT` (which `lifecycle` already
//! needs for handover) lets them share the port, and the kernel steers by the most
//! specific match: loopback connections keep going to the loopback socket, everything
//! else to the wide one. So the switch can be thrown mid-session without restarting
//! anything, without unbinding the port, and without cutting a single in-flight stream.
//!
//! Where the address comes from, in order: `AGENT_ROUTER_BIND` when it is set — an
//! explicit override, for a hand-launch or a dev box — then the stored `bind` setting,
//! then loopback. The setting is the part that survives a reboot. Before there was one,
//! a wide bind was only ever a property of how the process happened to be started, and
//! launchd hands a login item an almost empty environment: every reboot came back up on
//! loopback, whatever the day before had been set to.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use tokio::sync::{mpsc, oneshot};

/// The settings key, in the same table as the password hash. Holds an IP address, or
/// the empty string for loopback only.
pub const KEY: &str = "bind";

/// How long a departing router waits for the streams on its network listener. Long
/// enough for a request to finish, short enough that a handover is not held up by a
/// live view someone left open on a phone.
const DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The stored address, if one was stored and it actually widens anything.
pub fn stored(db_path: &Path) -> Option<IpAddr> {
    let raw = crate::db::setting(db_path, KEY)?;
    let ip: IpAddr = raw.trim().parse().ok()?;
    (!ip.is_loopback()).then_some(ip)
}

/// Remember the answer, so whatever starts the router next comes up the same way.
pub fn store(db_path: &Path, ip: Option<IpAddr>) -> anyhow::Result<()> {
    let value = ip.map(|ip| ip.to_string()).unwrap_or_default();
    crate::db::set_setting(db_path, KEY, &value)
}

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

enum Command {
    Set {
        want: Option<IpAddr>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Stop accepting and wait for what is already open to finish. Sent once, on the
    /// way out.
    Drain { reply: oneshot::Sender<()> },
}

/// Throws the switch. Lives in `AppState`, and deliberately holds neither a listener
/// nor the `Router` it serves: it can therefore be built before either exists, and it
/// closes no reference cycle back through the state the router is serving from.
#[derive(Clone)]
pub struct Switch {
    tx: mpsc::Sender<Command>,
    now: Arc<Mutex<Option<IpAddr>>>,
}

/// The other half, handed to [`supervise`] once the app to serve exists.
pub struct Pending {
    rx: mpsc::Receiver<Command>,
    now: Arc<Mutex<Option<IpAddr>>>,
}

pub fn switch() -> (Switch, Pending) {
    let (tx, rx) = mpsc::channel(4);
    let now = Arc::new(Mutex::new(None));
    (Switch { tx, now: now.clone() }, Pending { rx, now })
}

impl Switch {
    /// The address we answer on beyond loopback, if any.
    pub fn current(&self) -> Option<IpAddr> {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start, move or stop the wider listener, and wait to hear whether it worked.
    /// An error means nothing changed.
    pub async fn set(&self, want: Option<IpAddr>) -> Result<(), String> {
        const GONE: &str = "the listener supervisor is not running";
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Set { want, reply })
            .await
            .map_err(|_| GONE.to_string())?;
        answer.await.map_err(|_| GONE.to_string())?
    }

    /// Wait for the wider listener to finish draining, on the way out.
    ///
    /// It stops accepting on the shutdown signal by itself, at the same moment the
    /// loopback listener does. This is what makes the *rest* of the drain matter: the
    /// process must not exit while a phone or a remote agent still has a stream open,
    /// which is the whole promise `lifecycle` makes for the loopback socket.
    pub async fn drain(&self) {
        let (reply, answer) = oneshot::channel();
        if self.tx.send(Command::Drain { reply }).await.is_ok() {
            let _ = answer.await;
        }
    }
}

/// Own the wider listener and reconcile it with whatever the switch asks for.
///
/// One task, so two flips can never race each other onto the same port, and so the
/// listener has a single owner that outlives any one request.
pub fn supervise(app: Router, port: u16, pending: Pending) {
    let Pending { mut rx, now } = pending;
    tokio::spawn(async move {
        let mut running: Option<Running> = None;
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Set { want, reply } => {
                    let result = reconcile(&app, port, &mut running, want);
                    if result.is_ok() {
                        *now.lock().unwrap_or_else(|e| e.into_inner()) = want;
                        // Guard layers 2 and 3 have to follow the socket: while it is
                        // narrow they refuse every caller that is not on this machine,
                        // which is exactly the caller a wide socket exists to admit.
                        crate::guard::allow_off_host(want.is_some());
                    }
                    let _ = reply.send(result);
                }
                Command::Drain { reply } => {
                    if let Some(old) = running.take() {
                        let _ = old.stop.send(());
                        // It may already be on its way out under the same signal; both
                        // paths end at the same place, and joining is what tells us
                        // the last connection has finished.
                        //
                        // Bounded, and then dropped on the floor: a live view's stream
                        // has no natural end, so waiting for one would hold a departing
                        // router open indefinitely — and the task holds a clone of the
                        // app, which is what keeps `capture` from draining once we are
                        // this far out.
                        let mut task = old.task;
                        tokio::select! {
                            _ = &mut task => {}
                            _ = tokio::time::sleep(DRAIN_LIMIT) => {
                                // Ends the accept loop. The connection tasks it spawned
                                // are their own, and a response body with no end keeps
                                // one alive past this — which is why the exit after
                                // this point is bounded rather than tidy.
                                task.abort();
                                tracing::warn!(
                                    "a stream on the network listener is still open after {}s; \
                                     ending it rather than holding up the exit",
                                    DRAIN_LIMIT.as_secs()
                                );
                            }
                        }
                    }
                    let _ = reply.send(());
                    return;
                }
            }
        }
    });
}

struct Running {
    ip: IpAddr,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

fn reconcile(
    app: &Router,
    port: u16,
    running: &mut Option<Running>,
    want: Option<IpAddr>,
) -> Result<(), String> {
    if running.as_ref().map(|r| r.ip) == want {
        return Ok(());
    }

    // Bound before the old one is retired, so a bind that fails leaves whatever was
    // working exactly as it was.
    let fresh = match want {
        Some(ip) => {
            let addr = SocketAddr::new(ip, port);
            let listener = crate::lifecycle::bind_reuseport(addr).map_err(|e| format!("{e:#}"))?;
            let (stop, halt) = oneshot::channel();
            let app = app.clone();
            let task = tokio::spawn(async move {
                tracing::warn!("also listening on http://{addr} — REACHABLE FROM THE NETWORK");
                let served = axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                // Either the switch being thrown, or the process being asked to stop.
                // Without the second, this socket would go on accepting work for a
                // process already draining — the exact stall `lifecycle` exists to
                // avoid, and the connections it took would die with the process.
                .with_graceful_shutdown(async move {
                    tokio::select! {
                        _ = halt => {}
                        why = crate::lifecycle::shutdown_signal() => {
                            tracing::info!("{why} received; the network listener is draining too");
                        }
                    }
                })
                .await;
                match served {
                    Ok(()) => tracing::info!("stopped listening on {addr}"),
                    Err(err) => tracing::warn!(%err, "the network listener stopped"),
                }
            });
            Some(Running { ip, stop, task })
        }
        None => None,
    };

    if let Some(old) = running.take() {
        // axum releases the socket as soon as it is told to stop and only then lets
        // the connections it already holds finish, so turning this off is immediate
        // even while a phone still has a stream open.
        let _ = old.stop.send(());
    }
    *running = fresh;
    Ok(())
}

// ---------------------------------------------------------------------------
// Telling people where to point their phone
// ---------------------------------------------------------------------------

/// The address another device on this network would reach us at.
///
/// Asks the routing table which interface would carry a packet to somewhere off-link.
/// A UDP `connect` sends nothing, so this costs no traffic, needs no DNS, and works
/// with the network down — it simply finds nothing to report.
pub fn lan_address() -> Option<IpAddr> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // TEST-NET-1: reserved for documentation and never routed to anything real.
    sock.connect(("192.0.2.1", 9)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// What to type into a phone to reach this router.
pub fn lan_url(bound: Option<IpAddr>, port: u16) -> Option<String> {
    let ip = match bound {
        // A particular address was chosen: that is the one that answers.
        Some(ip) if !ip.is_unspecified() => ip,
        // `0.0.0.0` is every interface, which is no use to someone typing an address
        // — and when nothing is bound yet, this is the address it *would* be.
        _ => lan_address()?,
    };
    Some(format!("http://{}", SocketAddr::new(ip, port)))
}

/// What the dashboard and the phone apps need to know about the socket. Shared with
/// `GET /api/auth`, which is where both already look for `remote_bind`.
pub fn report(state: &crate::proxy::AppState) -> serde_json::Map<String, serde_json::Value> {
    let bound = state.network.current();
    let mut out = serde_json::Map::new();
    out.insert("remote_bind".into(), bound.is_some().into());
    out.insert(
        "listening_on".into(),
        bound.map(|ip| ip.to_string()).into(),
    );
    out.insert("lan_url".into(), lan_url(bound, state.config.bind.port()).into());
    // A hand-launch that set the variable keeps the last word on the next start, so
    // the page can say as much rather than promising more than the setting can deliver.
    out.insert(
        "env_override".into(),
        std::env::var("AGENT_ROUTER_BIND")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            .into(),
    );
    out
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

pub mod api {
    use std::net::IpAddr;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use crate::proxy::AppState;

    #[derive(Deserialize)]
    pub struct Req {
        pub enable: bool,
        /// One particular address to listen on. Absent means every interface.
        #[serde(default)]
        pub address: Option<String>,
    }

    /// Only ever reached over loopback in practice: a caller that is not on this
    /// machine can only get here when the switch is already on, and `guard` has already
    /// made them present the password.
    pub async fn set(State(state): State<Arc<AppState>>, Json(req): Json<Req>) -> Json<Value> {
        let want = match parse(&state, &req).await {
            Ok(want) => want,
            Err(why) => return Json(json!({ "error": why })),
        };

        if let Err(err) = state.network.set(want).await {
            return Json(json!({ "error": err }));
        }
        // Stored only once the socket has agreed, so what is stored is always something
        // that worked. This is the half that survives the reboot.
        if let Err(err) = super::store(&state.config.db_path, want) {
            return Json(json!({
                "error": format!("listening changed, but it could not be saved for next time: {err:#}")
            }));
        }
        // A login item registered before this was a setting may still carry the old
        // environment variable, which would outvote the setting on the next boot.
        crate::setup::forget_bind_env();

        let mut out = super::report(&state);
        out.insert("ok".into(), true.into());
        Json(Value::Object(out))
    }

    async fn parse(state: &AppState, req: &Req) -> Result<Option<IpAddr>, String> {
        if !req.enable {
            return Ok(None);
        }
        // Without a password every caller from another machine is refused anyway, so
        // widening the socket would buy nothing but exposure. Said here, where the
        // choice is being made, rather than discovered later from a phone.
        if !state.auth.configured().await {
            return Err("set a password first — until one exists, callers from other machines are refused".into());
        }
        let raw = req.address.as_deref().unwrap_or("0.0.0.0").trim().to_string();
        match raw.parse::<IpAddr>() {
            Ok(ip) if ip.is_loopback() => {
                Err(format!("{raw} is loopback, which is what this turns off"))
            }
            Ok(ip) => Ok(Some(ip)),
            Err(_) => Err(format!("{raw:?} is not an IP address")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_setting_is_no_setting_at_all() {
        let dir = std::env::temp_dir().join(format!("ar-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        crate::db::Db::open(&db).unwrap();

        assert_eq!(stored(&db), None, "nothing stored yet");

        store(&db, Some("0.0.0.0".parse().unwrap())).unwrap();
        assert_eq!(stored(&db), Some("0.0.0.0".parse().unwrap()));

        // Off is stored as an empty value rather than a loopback address, but a store
        // written by an older build could hold either, and both mean the same thing.
        store(&db, None).unwrap();
        assert_eq!(stored(&db), None);
        crate::db::set_setting(&db, KEY, "127.0.0.1").unwrap();
        assert_eq!(stored(&db), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_chosen_address_is_what_we_tell_people_to_type() {
        let ip: IpAddr = "10.30.0.186".parse().unwrap();
        assert_eq!(lan_url(Some(ip), 8787).as_deref(), Some("http://10.30.0.186:8787"));
        // `0.0.0.0` is not an address anyone can type, so it is resolved to the one
        // this machine is actually reachable at — which a machine with no route has
        // none of, and then there is nothing useful to say.
        let wide = lan_url(Some("0.0.0.0".parse().unwrap()), 8787);
        assert_eq!(wide, lan_url(None, 8787));
    }
}
