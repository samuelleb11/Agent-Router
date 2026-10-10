//! Telling machines to sync now, through the tunnel each agent already holds open, and
//! keeping what each machine last said it holds. docs/library.md, "Push" and "Each machine's
//! state".
//!
//! The router asks an agent for one tunneled route, `POST /agent/library/sync`, and it
//! carries nothing: the agent pulls the manifest and the blobs itself, checks every digest,
//! and answers with its report. So all a router can make a machine write into `~/.claude` is
//! what its Library lists, through the agent's own checks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::store::{self, Heard, Version};
use super::sync::{Line, Outcome, Report};
use crate::machines::{Machine, Relay, Seen};
use crate::proxy::AppState;
use crate::refusal::{failed, refuse, JsonBody};

/// The agent's route (`agent::routes`).
pub const PATH: &str = "/agent/library/sync";

/// A sync downloads what changed: a slow machine on a slow link gets this long.
const WAIT: Duration = Duration::from_secs(120);

/// What one machine said when told to sync.
#[derive(Clone, Debug, Serialize)]
pub struct Told {
    pub machine: String,
    pub label: String,
    /// `synced`, `predates`, `opted_out`, `offline` or `failed`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Told {
    fn offline(seen: &Seen) -> Told {
        Told {
            machine: seen.id.clone(),
            label: seen.label.clone(),
            outcome: "offline",
            report: None,
            why: Some(format!("{} is offline: it syncs when it connects again", seen.label)),
        }
    }
}

/// Tell one connected machine to sync, and keep what it says.
pub async fn tell(state: &Arc<AppState>, machine: &Arc<Machine>) -> Told {
    let (id, label) = (machine.hello.machine.clone(), machine.hello.label.clone());
    let told = |outcome, report, why: Option<String>| Told { machine: id.clone(), label: label.clone(), outcome, report, why };
    let response = match machine.request(&Method::POST, PATH, Vec::new(), &[], WAIT).await {
        Ok(response) => response,
        Err(Relay::Offline(_)) => return told("offline", None, Some(format!("{label} is offline: it syncs when it connects again"))),
        Err(Relay::Failed(why)) => return told("failed", None, Some(why)),
        Err(Relay::Unknown) => return told("failed", None, Some(format!("{label} did not answer"))),
    };
    let status = response.status();
    let Ok(Ok(body)) = tokio::time::timeout(WAIT, axum::body::to_bytes(response.into_body(), 16 << 20)).await else {
        return told("failed", None, Some(format!("{label} did not finish answering")));
    };
    let (outcome, heard, report, why) = match status {
        StatusCode::OK => match serde_json::from_slice::<Report>(&body) {
            Ok(report) => ("synced", Some(Heard::Report(report.clone())), Some(report), None),
            Err(e) => ("failed", None, None, Some(format!("{label} answered with something other than a report: {e}"))),
        },
        StatusCode::NOT_FOUND => (
            "predates",
            Some(Heard::Predates),
            None,
            Some(format!("{label}'s agent was installed before the Library: run the installer on it again")),
        ),
        StatusCode::CONFLICT => ("opted_out", Some(Heard::OptedOut), None, Some(words(&body))),
        other => ("failed", None, None, Some(format!("{label} answered {other}: {}", words(&body)))),
    };
    if let Some(heard) = heard {
        if let Err(e) = keep(state, &id, &label, heard).await {
            tracing::warn!(machine = %label, "what it said of the Library was not kept: {e:#}");
        }
    }
    told(outcome, report, why)
}

/// Tell every connected machine to sync. Every machine seen and not connected is answered
/// as offline.
pub async fn tell_all(state: &Arc<AppState>) -> Vec<Told> {
    let online = state.machines.connected();
    let mut told = futures_util::future::join_all(online.iter().map(|m| tell(state, m))).await;
    let reached: HashSet<String> = online.iter().map(|m| m.hello.machine.clone()).collect();
    told.extend(state.machines.seen().iter().filter(|s| !s.online && !reached.contains(&s.id)).map(Told::offline));
    told
}

/// After a version goes live, or an item is archived: every connected agent is told to
/// sync, and nobody waits for them.
pub fn push(state: &Arc<AppState>) {
    let state = state.clone();
    tokio::spawn(async move {
        for told in tell_all(&state).await.into_iter().filter(|t| t.outcome != "offline") {
            tracing::info!(machine = %told.label, outcome = told.outcome, why = ?told.why, "told to sync the Library");
        }
    });
}

#[derive(Deserialize)]
struct SyncNow {
    #[serde(default)]
    machine: Option<String>,
}

/// `POST /api/library/sync`: tell every machine to sync now, or with `{"machine": id}` that
/// one alone, and answer what each said.
pub async fn sync_now(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let only = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        match serde_json::from_slice::<SyncNow>(&body) {
            Ok(asked) => asked.machine,
            Err(e) => return refuse(StatusCode::BAD_REQUEST, format!("send nothing, or {{\"machine\": <id>}}: {e}")),
        }
    };
    let told = match only {
        None => tell_all(&state).await,
        Some(id) => match state.machines.online(&id) {
            Some(machine) => vec![tell(&state, &machine).await],
            None => match state.machines.seen().iter().find(|s| s.id == id) {
                Some(seen) => vec![Told::offline(seen)],
                None => return refuse(StatusCode::NOT_FOUND, format!("no machine {id} has connected to this router")),
            },
        },
    };
    Json(json!({ "ok": true, "machines": told })).into_response()
}

/// What an agent sends after a sync it ran on its own.
#[derive(Deserialize)]
pub struct Reported {
    machine: String,
    #[serde(default)]
    label: Option<String>,
    report: Report,
}

/// `POST /api/library/report`: an agent's report of a sync it ran on its own — at a
/// connection, or on its timer. A sync the router asked for answers with its report instead.
pub async fn report(State(state): State<Arc<AppState>>, JsonBody(reported): JsonBody<Reported>) -> Response {
    if reported.machine.trim().is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "a report names the machine it comes from");
    }
    let label = reported.label.unwrap_or_default();
    match keep(&state, &reported.machine, &label, Heard::Report(reported.report)).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => failed(e),
    }
}

async fn keep(state: &Arc<AppState>, machine: &str, label: &str, heard: Heard) -> anyhow::Result<()> {
    let (path, machine, label) = (state.config.db_path.clone(), machine.to_string(), label.to_string());
    tokio::task::spawn_blocking(move || {
        let conn = store::writer(&path)?;
        store::hear(&conn, &machine, (!label.is_empty()).then_some(label.as_str()), &heard, crate::db::now_ms())
    })
    .await?
}

/// Where a machine's Library stands, as `/api/machines` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Standing {
    /// `in_step`, `behind`, `failed`, `replaced`, `predates`, `opted_out`, or `not_yet` for a
    /// machine that has said nothing of the Library.
    pub state: &'static str,
    /// Live items its last sync did not leave at their live version.
    pub behind: usize,
    pub failed: usize,
    /// Items its last sync put back over a copy edited there, or not placed by the Library.
    pub replaced: Vec<String>,
    /// When it last said anything.
    pub at_ms: Option<i64>,
    /// Each other client's standing on the machine, by its name: `kimi-code`. The fields
    /// above are Claude Code's.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub clients: std::collections::BTreeMap<String, Standing>,
}

impl Standing {
    pub fn not_yet() -> Standing {
        Standing { state: "not_yet", behind: 0, failed: 0, replaced: Vec::new(), at_ms: None, clients: Default::default() }
    }
}

/// Where a machine stands, from what it last said and what is live now. A version that went
/// live after its last sync is one it is behind on, whether or not it is online to hear.
pub fn standing(heard: Option<&(i64, Heard)>, live: &[Version]) -> Standing {
    let Some((at_ms, heard)) = heard else { return Standing::not_yet() };
    let said = |state| Standing { state, at_ms: Some(*at_ms), ..Standing::not_yet() };
    let report = match heard {
        Heard::OptedOut => return said("opted_out"),
        Heard::Predates => return said("predates"),
        Heard::Report(report) => report,
    };
    let mut standing = of_lines(&report.items, live, *at_ms);
    standing.clients = report.clients.iter().map(|(client, lines)| (client.clone(), of_lines(lines, live, *at_ms))).collect();
    standing
}

/// One client's standing, from its lines of a report.
fn of_lines(lines: &[Line], live: &[Version], at_ms: i64) -> Standing {
    let (mut behind, mut failed) = (0, 0);
    for version in live {
        let line = lines.iter().find(|l| l.kind == version.kind && l.name == version.name);
        match line {
            Some(l) if l.version == version.number && matches!(l.outcome, Outcome::InStep | Outcome::Placed | Outcome::Replaced) => {}
            Some(l) if l.version == version.number && l.outcome == Outcome::Failed => failed += 1,
            _ => behind += 1,
        }
    }
    let replaced: Vec<String> =
        lines.iter().filter(|l| l.outcome == Outcome::Replaced).map(|l| format!("{}/{}", l.kind, l.name)).collect();
    let state = if failed > 0 {
        "failed"
    } else if behind > 0 {
        "behind"
    } else if !replaced.is_empty() {
        "replaced"
    } else {
        "in_step"
    };
    Standing { state, behind, failed, replaced, at_ms: Some(at_ms), clients: Default::default() }
}

/// Every machine's standing that has said anything, by machine id.
pub async fn standings(state: &Arc<AppState>) -> HashMap<String, Standing> {
    let state = state.clone();
    let read = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = state.reads.get()?;
        Ok((store::heard(&conn)?, store::live(&conn)?))
    })
    .await;
    match read {
        Ok(Ok((heard, live))) => heard.iter().map(|(id, said)| (id.clone(), standing(Some(said), &live))).collect(),
        Ok(Err(e)) => {
            tracing::warn!("machines' Library standing unread: {e:#}");
            HashMap::new()
        }
        Err(e) => {
            tracing::warn!("machines' Library standing unread: {e}");
            HashMap::new()
        }
    }
}

fn words(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{header, Request};
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::api_refusals::Lab;
    use crate::library::sync::Line;
    use crate::library::Kind;

    /// Serve `lab` on a socket: an agent's connection is an upgrade, which takes one.
    async fn serve(lab: &Lab) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
        addr
    }

    /// Connect a machine answering with `routes`, as `agent run` connects one, and wait
    /// until the router counts it online. Aborting the handle drops its connection.
    async fn connect(lab: &Lab, addr: SocketAddr, id: &str, routes: axum::Router) -> tokio::task::JoinHandle<()> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{addr}{}", crate::tunnel::PATH).into_client_request().unwrap();
        let password = lab.password.expect("a hosted router");
        request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {password}").parse().unwrap());
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (socket, _) = tokio_tungstenite::client_async(request, tcp).await.unwrap();
        let hello = crate::tunnel::Hello {
            machine: id.into(),
            label: format!("{id}'s label"),
            platform: std::env::consts::OS.into(),
            version: "test".into(),
            build: "test".into(),
        };
        let held = tokio::spawn(async move {
            let _ = crate::agent::serve(socket, &hello, routes).await;
        });
        until(|| lab.state.machines.seen().iter().any(|m| m.id == id && m.online)).await;
        held
    }

    async fn until(what: impl Fn() -> bool) {
        let waited = async {
            while !what() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        // Wide, for a busy CI runner: the bound only keeps what never comes from hanging.
        tokio::time::timeout(Duration::from_secs(60), waited).await.expect("it came to pass");
    }

    /// A machine whose agent syncs when told, answering with `report`, and counting.
    fn syncing(report: Report) -> (axum::Router, Arc<AtomicUsize>) {
        let told = Arc::new(AtomicUsize::new(0));
        let count = told.clone();
        let routes = axum::Router::new().route(
            PATH,
            post(move || {
                count.fetch_add(1, Ordering::SeqCst);
                let report = report.clone();
                async move { Json(report) }
            }),
        );
        (routes, told)
    }

    fn report(lines: &[(&str, i64, Outcome)]) -> Report {
        Report {
            at_ms: 1,
            items: lines
                .iter()
                .map(|(name, version, outcome)| Line {
                    kind: Kind::Skill,
                    name: name.to_string(),
                    version: *version,
                    outcome: *outcome,
                    kept: None,
                    why: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    async fn call(lab: &Lab, path: &str, body: impl Into<Body>) -> (StatusCode, Value) {
        let mut req = Request::post(path)
            .header(header::HOST, "127.0.0.1")
            .header(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap()))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.into())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn machines(lab: &Lab) -> Value {
        let mut req = Request::get("/api/machines")
            .header(header::HOST, "127.0.0.1")
            .header(header::AUTHORIZATION, format!("Bearer {}", lab.password.unwrap()))
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = lab.app.clone().oneshot(req).await.unwrap();
        serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
    }

    fn row<'a>(machines: &'a Value, id: &str) -> &'a Value {
        machines.as_array().unwrap().iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("{id} in {machines}"))
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n")
    }

    fn told<'a>(answer: &'a Value, id: &str) -> &'a Value {
        answer["machines"].as_array().unwrap().iter().find(|m| m["machine"] == id).unwrap_or_else(|| panic!("{id} in {answer}"))
    }

    #[tokio::test]
    async fn sync_now_reaches_every_connected_agent() {
        let lab = Lab::hosted("push-every").await;
        let addr = serve(&lab).await;
        let (first, first_told) = syncing(report(&[("bench", 1, Outcome::Placed)]));
        let (second, second_told) = syncing(report(&[("bench", 1, Outcome::InStep)]));
        let _a = connect(&lab, addr, "first", first).await;
        let _b = connect(&lab, addr, "second", second).await;

        let (status, answer) = call(&lab, "/api/library/sync", Body::empty()).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!((told(&answer, "first")["outcome"].as_str(), told(&answer, "second")["outcome"].as_str()), (Some("synced"), Some("synced")));
        assert_eq!(told(&answer, "first")["report"]["items"][0]["outcome"], "placed");
        assert_eq!((first_told.load(Ordering::SeqCst), second_told.load(Ordering::SeqCst)), (1, 1));

        // Named, only that one is told.
        let (status, answer) = call(&lab, "/api/library/sync", r#"{"machine":"second"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["machines"].as_array().unwrap().len(), 1);
        assert_eq!((first_told.load(Ordering::SeqCst), second_told.load(Ordering::SeqCst)), (1, 2));

        let (status, answer) = call(&lab, "/api/library/sync", r#"{"machine":"nobody"}"#).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(answer["error"].as_str().unwrap().contains("no machine nobody"), "{answer}");
        let (status, _) = call(&lab, "/api/library/sync", "not json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sync_now_names_an_agent_that_predates_the_library() {
        let lab = Lab::hosted("push-predates").await;
        let addr = serve(&lab).await;
        // An agent from before the Library has no such route: its router answers 404.
        let _old = connect(&lab, addr, "old", axum::Router::new()).await;
        // One installed with --no-library refuses in words.
        let opted_out = axum::Router::new().route(
            PATH,
            post(|| async { refuse(StatusCode::CONFLICT, "this machine takes no part in the Library: it was installed with --no-library") }),
        );
        let _out = connect(&lab, addr, "out", opted_out).await;

        let (_, answer) = call(&lab, "/api/library/sync", Body::empty()).await;
        let old = told(&answer, "old");
        assert_eq!(old["outcome"], "predates");
        assert!(old["why"].as_str().unwrap().contains("run the installer on it again"), "{old}");
        let out = told(&answer, "out");
        assert_eq!(out["outcome"], "opted_out");
        assert!(out["why"].as_str().unwrap().contains("--no-library"), "{out}");

        let listed = machines(&lab).await;
        assert_eq!(row(&listed, "old")["library"]["state"], "predates");
        assert_eq!(row(&listed, "out")["library"]["state"], "opted_out");
    }

    #[tokio::test]
    async fn sync_now_names_an_offline_machine() {
        let lab = Lab::hosted("push-offline").await;
        let addr = serve(&lab).await;
        let (routes, _) = syncing(report(&[]));
        let held = connect(&lab, addr, "gone", routes).await;
        held.abort();
        until(|| lab.state.machines.seen().iter().any(|m| m.id == "gone" && !m.online)).await;

        let (status, answer) = call(&lab, "/api/library/sync", r#"{"machine":"gone"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(told(&answer, "gone")["outcome"], "offline");
        assert!(told(&answer, "gone")["why"].as_str().unwrap().contains("is offline"));
        let (_, answer) = call(&lab, "/api/library/sync", Body::empty()).await;
        assert_eq!(told(&answer, "gone")["outcome"], "offline", "every machine seen is answered for");
    }

    #[tokio::test]
    async fn a_version_going_live_is_pushed_to_connected_agents() {
        let lab = Lab::hosted("push-live").await;
        let addr = serve(&lab).await;
        let (routes, told) = syncing(report(&[]));
        let _held = connect(&lab, addr, "listening", routes).await;
        let seen = || told.load(Ordering::SeqCst);

        let (status, _) = call(&lab, "/api/library/items/skill/bench/versions", skill_md("bench", "One.")).await;
        assert_eq!(status, StatusCode::CREATED);
        until(|| seen() == 1).await;
        call(&lab, "/api/library/items/skill/bench/versions", skill_md("bench", "Two.")).await;
        until(|| seen() == 2).await;
        let (status, _) = call(&lab, "/api/library/items/skill/bench/versions/1/live", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        until(|| seen() == 3).await;
        call(&lab, "/api/library/items/skill/bench/archive", Body::empty()).await;
        until(|| seen() == 4).await;

        // A version that only waits for approval reaches no machine, so none is told.
        let conn = store::writer(&lab.state.config.db_path).unwrap();
        let waiting = crate::library::bundle::Bundle::check(
            Kind::Skill,
            "vault",
            vec![crate::library::bundle::File { path: "SKILL.md".into(), bytes: skill_md("vault", "Waits.").into_bytes(), executable: false }],
        )
        .unwrap();
        store::publish(&conn, &waiting, &store::Provenance::Token { token: "ci".into() }, false, 9).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(seen(), 4);
    }

    #[tokio::test]
    async fn a_machines_library_state_survives_a_restart() {
        let lab = Lab::hosted("push-restart").await;
        call(&lab, "/api/library/items/skill/bench/versions", skill_md("bench", "One.")).await;
        // The agent reports a sync it ran on its own, as it does at a connection.
        let reported = json!({ "machine": "kept", "label": "kept's label", "report": report(&[("bench", 1, Outcome::Placed)]) });
        let (status, _) = call(&lab, "/api/library/report", reported.to_string()).await;
        assert_eq!(status, StatusCode::OK);
        let addr = serve(&lab).await;
        let (routes, _) = syncing(report(&[]));
        let _held = connect(&lab, addr, "kept", routes).await;
        let before = machines(&lab).await;
        assert_eq!(row(&before, "kept")["library"]["state"], "in_step");

        // A router started again over the same store knows what the machine said, as soon
        // as the machine is back and before it has synced again.
        let again = lab.restarted().await;
        let addr = serve(&again).await;
        let (routes, told) = syncing(report(&[]));
        let _back = connect(&again, addr, "kept", routes).await;
        let after = machines(&again).await;
        assert_eq!(row(&after, "kept")["library"], row(&before, "kept")["library"]);
        assert_eq!(told.load(Ordering::SeqCst), 0, "it said nothing new");

        let (status, answer) = call(&again, "/api/library/report", r#"{"machine":"","report":{"at_ms":1,"items":[]}}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        let (status, answer) = call(&again, "/api/library/report", "not json").await;
        assert!(status.is_client_error() && answer["error"].is_string(), "{status} {answer}");
    }

    #[tokio::test]
    async fn an_offline_machine_reads_as_behind_after_a_version_went_live() {
        let lab = Lab::hosted("push-behind").await;
        call(&lab, "/api/library/items/skill/bench/versions", skill_md("bench", "One.")).await;
        let addr = serve(&lab).await;
        let (routes, _) = syncing(report(&[("bench", 1, Outcome::InStep)]));
        let held = connect(&lab, addr, "away", routes).await;
        let (_, answer) = call(&lab, "/api/library/sync", r#"{"machine":"away"}"#).await;
        assert_eq!(told(&answer, "away")["outcome"], "synced");
        assert_eq!(row(&machines(&lab).await, "away")["library"]["state"], "in_step");

        held.abort();
        until(|| lab.state.machines.seen().iter().any(|m| m.id == "away" && !m.online)).await;
        call(&lab, "/api/library/items/skill/bench/versions", skill_md("bench", "Two.")).await;
        call(&lab, "/api/library/items/skill/vault/versions", skill_md("vault", "New.")).await;

        let listed = machines(&lab).await;
        let away = row(&listed, "away");
        assert_eq!((away["online"].as_bool(), away["library"]["state"].as_str(), away["library"]["behind"].as_u64()), (Some(false), Some("behind"), Some(2)));

        // How a machine's last report reads against what is live now.
        let heard = (5, Heard::Report(report(&[("bench", 2, Outcome::Replaced)])));
        let live = vec![Version {
            kind: Kind::Skill,
            name: "bench".into(),
            number: 2,
            digest: "d".into(),
            size: 1,
            description: "Two.".into(),
            provenance: store::Provenance::Upload,
            state: store::State::Live,
            created_at: 1,
            decided_at: None,
        }];
        let read = standing(Some(&heard), &live);
        assert_eq!((read.state, read.replaced.as_slice()), ("replaced", ["skill/bench".to_string()].as_slice()));
        assert_eq!(standing(None, &live).state, "not_yet");
    }

    fn bench(number: i64) -> Version {
        Version {
            kind: Kind::Skill,
            name: "bench".into(),
            number,
            digest: "d".into(),
            size: 1,
            description: "Two.".into(),
            provenance: store::Provenance::Upload,
            state: store::State::Live,
            created_at: 1,
            decided_at: None,
        }
    }

    /// A machine's report says each client's sync apart, and its standing keeps each.
    #[test]
    fn a_machines_library_state_is_kept_per_client() {
        let mut said = report(&[("bench", 2, Outcome::InStep)]);
        said.clients.insert("kimi-code".into(), report(&[("bench", 1, Outcome::InStep)]).items);
        let read = standing(Some(&(5, Heard::Report(said.clone()))), &[bench(2)]);
        assert_eq!(read.state, "in_step", "Claude Code's");
        assert_eq!(read.clients["kimi-code"].state, "behind", "Kimi Code's, a version back");
        assert_eq!(read.clients["kimi-code"].behind, 1);

        // As /api/machines gives it, and as a report travels and is kept.
        let json = serde_json::to_value(&read).unwrap();
        assert_eq!(json["clients"]["kimi-code"]["state"], "behind");
        let kept: Report = serde_json::from_value(serde_json::to_value(&said).unwrap()).unwrap();
        assert_eq!(kept, said);
        // A report from before there were clients reads with none, and says none.
        let old: Report = serde_json::from_value(serde_json::json!({ "at_ms": 1, "items": [] })).unwrap();
        assert!(old.clients.is_empty());
        assert!(serde_json::to_value(standing(Some(&(1, Heard::Report(old))), &[])).unwrap().get("clients").is_none());
    }
}
