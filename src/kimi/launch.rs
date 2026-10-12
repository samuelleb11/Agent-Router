//! `agent-router kimi`: a hosted Kimi Code session from a terminal (AGEN-0113).
//!
//!   agent-router kimi [--model <id>] [prompt]
//!
//! It asks the router to start a hosted session in the current directory on this machine
//! (`POST /api/live/start`, `kimi::live`), which the machine agent runs through Kimi Code's
//! own server. Then it works with that session as the console does: each line typed is a
//! prompt, each reply and tool call is printed as it lands, an approval is asked here, and
//! Ctrl-C interrupts the turn. The session is the agent's, so the console and the phones can
//! drive it too, and the link to it is printed first.
//!
//! The terminal UI, `kimi`, stays watched only: it keeps its session in its own process, which
//! takes no outside control.

use std::io::Write;

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Bytes;
use axum::http::{header, Method, StatusCode};
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::library::sync::Router;

const USAGE: &str = "usage: agent-router kimi [--model <id>] [prompt]

Starts a hosted Kimi Code session in this directory, through this machine's agent, and works
with it here: type a prompt and press return; answer y or n when it asks to run something;
Ctrl-C stops the turn, and again to leave. The console and the phones can drive it too.";

/// The model a session starts on when none is named: the router's K3.
const MODEL: &str = "k3";

/// Where the router is, and this machine's id there.
struct Reach {
    router: Router,
    /// The console, for the link to the session.
    console: String,
    machine: Option<String>,
}

/// This machine's agent's router, else the local router on loopback.
fn reach() -> Result<Reach> {
    match crate::agent::connection() {
        Ok(c) => Ok(Reach { router: Router::new(&c.server, &c.password)?, console: c.server, machine: c.machine }),
        Err(_) => {
            let port = std::env::var("AGENT_ROUTER_PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(8787);
            let local = format!("http://127.0.0.1:{port}");
            // Loopback is not asked for a password on a local router.
            Ok(Reach { router: Router::new(&local, "")?, console: local, machine: None })
        }
    }
}

pub async fn main(args: &[String]) -> Result<()> {
    rustls::crypto::ring::default_provider().install_default().ok();
    let mut model = MODEL.to_string();
    let mut words = Vec::new();
    let mut args = args.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model" => model = args.next().ok_or_else(|| anyhow!("--model wants a model id\n\n{USAGE}"))?.clone(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with('-') => bail!("unknown option {other:?}\n\n{USAGE}"),
            word => words.push(word.to_string()),
        }
    }
    let reach = reach()?;
    let cwd = std::env::current_dir()?;
    let session = start(&reach.router, reach.machine.as_deref(), &cwd.display().to_string(), &model).await?;
    println!("Kimi Code session on {model}, hosted by this machine, in {}", cwd.display());
    println!("  {}/live?session={session}", reach.console.trim_end_matches('/'));
    println!("Type a prompt and press return. Ctrl-C stops the turn; again, or Ctrl-D, leaves.\n");
    let first = (!words.is_empty()).then(|| words.join(" "));
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    let interrupts = futures_util::stream::unfold((), |()| async {
        tokio::signal::ctrl_c().await.ok()?;
        Some(((), ()))
    });
    run(&reach.router, &session, first, input, &mut std::io::stdout(), interrupts).await
}

/// Ask the router for its answer to `body` at `path`, as JSON; a refusal in its own words.
async fn call(router: &Router, path: &str, body: Value) -> Result<Value> {
    let json = vec![(header::CONTENT_TYPE, "application/json".to_string())];
    let (status, _, answer) = router.send(Method::POST, path, json, Bytes::from(body.to_string())).await?;
    let answer: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = answer.get("reason").or_else(|| answer.get("error")).and_then(Value::as_str).unwrap_or("no reason given");
        bail!("{why} ({status})");
    }
    Ok(answer)
}

/// Start a hosted session in `cwd` on `model`, on the machine `machine` (None: the router's
/// own). Says its id.
async fn start(router: &Router, machine: Option<&str>, cwd: &str, model: &str) -> Result<String> {
    let answer = call(router, "/api/live/start", json!({ "machine": machine, "cwd": cwd, "model": model }))
        .await
        .context("could not start a hosted Kimi Code session")?;
    answer.get("session").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| anyhow!("the router named no session"))
}

/// One event of a live stream: its name (`message` for an item), and its data.
struct Event {
    name: String,
    data: String,
}

/// The session's live stream, event by event.
async fn events(router: &Router, session: &str) -> Result<impl Stream<Item = Event>> {
    let response = router.open(Method::GET, &format!("/api/live/stream?session={session}")).await?;
    if response.status() != StatusCode::OK {
        bail!("the router answered {} for the session's stream", response.status());
    }
    let body = http_body_util::BodyStream::new(response.into_body());
    Ok(futures_util::stream::unfold((body, String::new()), |(mut body, mut buffer)| async move {
        loop {
            if let Some(end) = buffer.find("\n\n") {
                let block: String = buffer.drain(..end + 2).collect();
                let mut event = Event { name: "message".into(), data: String::new() };
                for line in block.lines() {
                    if let Some(name) = line.strip_prefix("event:") {
                        event.name = name.trim().to_string();
                    } else if let Some(data) = line.strip_prefix("data:") {
                        event.data.push_str(data.strip_prefix(' ').unwrap_or(data));
                    }
                }
                if event.data.is_empty() {
                    continue;
                }
                return Some((event, (body, buffer)));
            }
            match body.next().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        buffer.push_str(&String::from_utf8_lossy(&data).replace("\r\n", "\n"));
                    }
                }
                _ => return None,
            }
        }
    }))
}

/// How an item of the feed reads in a terminal; None for what is not shown.
fn shown(item: &Value) -> Option<String> {
    let s = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default();
    match (s("role"), s("kind")) {
        ("assistant", "text") => Some(s("text").to_string()),
        ("assistant", "tool_use") => Some(format!("● {}: {}", s("name"), s("text"))),
        ("user", "tool_result") => {
            let lines: Vec<&str> = s("text").lines().collect();
            let mut out: Vec<String> = lines.iter().take(5).map(|l| format!("  ⎿ {l}")).collect();
            if lines.len() > 5 {
                out.push(format!("  ⎿ … {} more lines", lines.len() - 5));
            }
            Some(if out.is_empty() { "  ⎿ (no output)".to_string() } else { out.join("\n") })
        }
        ("user", "text") => Some(format!("> {}", s("text"))),
        ("user", "interrupt") => Some("(interrupted)".to_string()),
        _ => None,
    }
}

/// Work with the hosted session `session`: send `first`, then each line of `input`, print
/// what it does to `out`, ask its approvals, and interrupt it on each of `interrupts`. Ends
/// when the input ends, when an interrupt finds nothing running, or when the stream does.
async fn run(
    router: &Router,
    session: &str,
    first: Option<String>,
    input: impl AsyncBufRead + Unpin,
    out: &mut impl Write,
    interrupts: impl Stream<Item = ()>,
) -> Result<()> {
    let events = events(router, session).await?;
    tokio::pin!(events);
    tokio::pin!(interrupts);
    let mut lines = input.lines();
    if let Some(prompt) = first {
        call(router, "/api/live/send", json!({ "session": session, "text": prompt })).await?;
    }
    // The approval asked and not yet answered, and the last one asked, so a status repeated
    // every second asks once.
    let mut asking: Option<String> = None;
    let mut asked: Option<String> = None;
    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                let data: Value = serde_json::from_str(&event.data).unwrap_or(Value::Null);
                match event.name.as_str() {
                    "status" => {
                        let on = &data["waiting_on"];
                        let request = on["request_id"].as_str().map(str::to_owned);
                        if on["kind"] == "permission" && request.is_some() && request != asked {
                            let what = on["summary"].as_str().or(on["tool"].as_str()).unwrap_or("a tool");
                            write!(out, "Allow {}: {what}? [y/N] ", on["tool"].as_str().unwrap_or("it"))?;
                            out.flush()?;
                            asking.clone_from(&request);
                            asked = request;
                        }
                    }
                    "error" => bail!("the session's stream ended: {}", event.data),
                    _ => {
                        if let Some(line) = shown(&data) {
                            writeln!(out, "{line}")?;
                            out.flush()?;
                        }
                    }
                }
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let line = line.trim();
                if let Some(request) = asking.take() {
                    let allow = matches!(line.to_ascii_lowercase().as_str(), "y" | "yes");
                    let answer = json!({ "session": session, "allow": allow, "request_id": request });
                    if let Err(e) = call(router, "/api/live/answer", answer).await {
                        writeln!(out, "(not answered: {e:#})")?;
                    }
                } else if !line.is_empty() {
                    if let Err(e) = call(router, "/api/live/send", json!({ "session": session, "text": line })).await {
                        writeln!(out, "(not sent: {e:#})")?;
                    }
                }
            }
            Some(()) = interrupts.next() => {
                if call(router, "/api/live/interrupt", json!({ "session": session })).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::response::sse::{Event as SseEvent, Sse};
    use axum::routing::{get, post};
    use axum::Json;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    /// A stand-in of the router's live routes: what it was asked, and a session that calls
    /// Bash, waits for its approval, then answers.
    #[derive(Clone, Default)]
    struct Standin {
        heard: Arc<Mutex<Vec<(String, Value)>>>,
        answered: Arc<tokio::sync::Notify>,
    }

    impl Standin {
        fn heard(&self) -> Vec<(String, Value)> {
            self.heard.lock().unwrap().clone()
        }
    }

    async fn standin() -> (Router, Standin) {
        let s = Standin::default();
        let note = |path: &'static str| {
            move |State(s): State<Standin>, Json(body): Json<Value>| async move {
                s.heard.lock().unwrap().push((path.to_string(), body));
                if path == "/api/live/answer" {
                    s.answered.notify_one();
                }
                Json(json!({ "ok": true, "session": "session_hosted" }))
            }
        };
        let stream = |State(s): State<Standin>| async move {
            let item = |v: Value| Ok::<_, Infallible>(SseEvent::default().data(v.to_string()));
            let events = async_stream::stream! {
                yield item(json!({ "role": "user", "kind": "text", "text": "Run echo, please." }));
                yield item(json!({ "role": "assistant", "kind": "tool_use", "name": "Bash", "text": "echo hello-from-standin" }));
                let status = json!({ "state": "ask", "waiting_on": {
                    "kind": "permission", "tool": "Bash", "request_id": "approval_1", "summary": "Running: echo hello-from-standin" } });
                // Sent twice, as the stream repeats a status each second: asked once.
                yield Ok(SseEvent::default().event("status").data(status.to_string()));
                yield Ok(SseEvent::default().event("status").data(status.to_string()));
                s.answered.notified().await;
                yield item(json!({ "role": "user", "kind": "tool_result", "name": "Bash", "text": "hello-from-standin\n" }));
                yield item(json!({ "role": "assistant", "kind": "text", "text": "The command said hello." }));
                std::future::pending::<()>().await;
            };
            Sse::new(events)
        };
        let app = axum::Router::new()
            .route("/api/live/start", post(note("/api/live/start")))
            .route("/api/live/send", post(note("/api/live/send")))
            .route("/api/live/answer", post(note("/api/live/answer")))
            .route("/api/live/interrupt", post(note("/api/live/interrupt")))
            .route("/api/live/stream", get(stream))
            .with_state(s.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Router::new(&base, "the router's password").unwrap(), s)
    }

    /// What `run` printed, read while it runs.
    #[derive(Clone, Default)]
    struct Screen(Arc<Mutex<Vec<u8>>>);

    impl Write for Screen {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Screen {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
        async fn shows(&self, what: &str) {
            let start = std::time::Instant::now();
            while !self.text().contains(what) {
                assert!(start.elapsed() < std::time::Duration::from_secs(10), "never showed {what:?}:\n{}", self.text());
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    }

    #[tokio::test]
    async fn the_kimi_launcher_starts_a_hosted_session_in_its_directory() {
        let (router, standin) = standin().await;
        let id = start(&router, Some("m-1"), "/Users/u/code/demo", "k3").await.unwrap();
        assert_eq!(id, "session_hosted");
        let heard = standin.heard();
        assert_eq!(heard[0].0, "/api/live/start");
        assert_eq!(heard[0].1, json!({ "machine": "m-1", "cwd": "/Users/u/code/demo", "model": "k3" }));

        // Its first prompt, from the command line, is sent once the stream is open; the
        // input's end ends it.
        let mut screen = Screen::default();
        let input = tokio::io::BufReader::new(&b""[..]);
        run(&router, &id, Some("Run echo, please.".into()), input, &mut screen, futures_util::stream::pending()).await.unwrap();
        assert_eq!(standin.heard()[1], ("/api/live/send".to_string(), json!({ "session": id, "text": "Run echo, please." })));
    }

    #[tokio::test]
    async fn the_kimi_launcher_prints_replies_and_answers_approvals() {
        let (router, standin) = standin().await;
        let (mut typed, input) = tokio::io::duplex(1024);
        let screen = Screen::default();
        let running = {
            let (router, mut screen) = (router, screen.clone());
            tokio::spawn(async move {
                run(&router, "session_hosted", None, tokio::io::BufReader::new(input), &mut screen, futures_util::stream::pending())
                    .await
            })
        };

        screen.shows("Allow Bash: Running: echo hello-from-standin? [y/N] ").await;
        assert!(screen.text().contains("> Run echo, please.\n● Bash: echo hello-from-standin\n"), "{}", screen.text());
        assert_eq!(screen.text().matches("Allow Bash").count(), 1, "asked once");
        tokio::io::AsyncWriteExt::write_all(&mut typed, b"y\n").await.unwrap();

        screen.shows("The command said hello.").await;
        assert!(screen.text().contains("  ⎿ hello-from-standin\nThe command said hello.\n"), "{}", screen.text());
        let answer = standin.heard().into_iter().find(|(p, _)| p == "/api/live/answer").unwrap().1;
        assert_eq!(answer, json!({ "session": "session_hosted", "allow": true, "request_id": "approval_1" }));

        // Another line is a prompt; the input's end leaves.
        tokio::io::AsyncWriteExt::write_all(&mut typed, b"And again\n").await.unwrap();
        drop(typed);
        running.await.unwrap().unwrap();
        assert_eq!(standin.heard().last().unwrap().1, json!({ "session": "session_hosted", "text": "And again" }));
    }
}
