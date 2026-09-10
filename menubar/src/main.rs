//! Agent Router menu-bar readout.
//!
//! Shows the active account's quota in the macOS menu bar, refreshed from the
//! router's `/api/quota`. The router holds the credential and does the upstream call;
//! this process only ever talks to loopback, so a GUI app that runs all day never
//! holds a token.
//!
//! Runs as an accessory app (LSUIElement) — menu bar only, no Dock icon, no window.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

const HOST: &str = "127.0.0.1";
const POLL: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct Quota {
    five_hour: Option<f64>,
    seven_day: Option<f64>,
    five_hour_resets: Option<String>,
    seven_day_resets: Option<String>,
    severity: String,
    /// Which source the numbers came from: the usage endpoint, or the rate-limit
    /// headers of the last proxied request.
    source: String,
    error: Option<String>,
}

impl Quota {
    /// What shows in the menu bar itself. Kept short: this competes for space with
    /// every other menu-bar item.
    fn title(&self) -> String {
        if self.error.is_some() {
            return "AR ⚠︎".into();
        }
        let pct = |v: Option<f64>| v.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into());
        // S = the 5-hour session window, W = the 7-day window.
        format!("{}S {} · W {}", self.glyph(), pct(self.five_hour), pct(self.seven_day))
    }

    /// A marker only when it is worth noticing; an always-on glyph is just noise.
    fn glyph(&self) -> &'static str {
        match self.severity.as_str() {
            "critical" => "\u{26A0}\u{FE0E} ",
            "warning" => "\u{25B3} ",
            _ => "",
        }
    }
}

fn port() -> u16 {
    std::env::var("AGENT_ROUTER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8787)
}

/// What a fetch attempt produced. The distinction matters in the UI: a refused
/// connection means the router is not running, which is the user's problem to fix,
/// while a bad status is ours.
enum Fetch {
    Body(String),
    Unreachable,
    Status(String),
}

/// Minimal HTTP GET. A dependency-free request to loopback is not worth an HTTP
/// client crate, and keeps this binary small enough to be a good menu-bar citizen.
fn get(path: &str) -> Fetch {
    let addr = format!("{HOST}:{}", port());
    let Ok(mut stream) = TcpStream::connect(&addr) else {
        return Fetch::Unreachable;
    };
    if stream.set_read_timeout(Some(Duration::from_secs(12))).is_err() {
        return Fetch::Unreachable;
    }
    // HTTP/1.1 with an explicit close: the server echoes the request version, so
    // asking in 1.0 and then matching a 1.1 status line never succeeds.
    if write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {HOST}:{}\r\nConnection: close\r\n\r\n",
        port()
    )
    .is_err()
    {
        return Fetch::Unreachable;
    }
    let mut raw = String::new();
    if stream.read_to_string(&mut raw).is_err() {
        return Fetch::Unreachable;
    }
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        return Fetch::Status("no response".into());
    };
    // Read the status code out of the line rather than matching a version prefix.
    let code = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?");
    if code == "200" {
        Fetch::Body(body.to_string())
    } else {
        Fetch::Status(format!("router returned {code}"))
    }
}

fn poll() -> Quota {
    let body = match get("/api/quota") {
        Fetch::Body(b) => b,
        Fetch::Unreachable => {
            return Quota { error: Some("router not running".into()), ..Default::default() }
        }
        Fetch::Status(s) => return Quota { error: Some(s), ..Default::default() },
    };
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return Quota { error: Some("bad response".into()), ..Default::default() };
    };
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Quota { error: Some(err.to_string()), ..Default::default() };
    }
    let win = |k: &str, f: &str| v.get(k).and_then(|w| w.get(f)).cloned();
    Quota {
        five_hour: win("five_hour", "utilization").and_then(|x| x.as_f64()),
        seven_day: win("seven_day", "utilization").and_then(|x| x.as_f64()),
        five_hour_resets: win("five_hour", "resets_at").and_then(|x| x.as_str().map(str::to_owned)),
        seven_day_resets: win("seven_day", "resets_at").and_then(|x| x.as_str().map(str::to_owned)),
        severity: v
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string(),
        source: v
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unavailable")
            .to_string(),
        error: (v.get("source").and_then(Value::as_str) == Some("unavailable"))
            .then(|| {
                v.get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("no quota data yet")
                    .to_string()
            }),
    }
}

/// "2026-09-13T21:00:00+00:00" -> "resets in 3d 20h", without pulling in a date crate.
fn resets_in(iso: &Option<String>) -> String {
    let Some(iso) = iso else { return "resets —".into() };
    let Some(secs) = epoch_secs(iso) else { return "resets —".into() };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let d = secs - now;
    if d <= 0 {
        return "resetting now".into();
    }
    let (h, m) = (d / 3600, (d % 3600) / 60);
    if h >= 24 {
        format!("resets in {}d {}h", h / 24, h % 24)
    } else if h > 0 {
        format!("resets in {h}h {m}m")
    } else {
        format!("resets in {m}m")
    }
}

/// Parse the fixed-shape ISO-8601 the usage endpoint returns, as UTC. The store
/// fallback supplies "@<unix-seconds>" instead, since that is what the rate-limit
/// headers carry.
fn epoch_secs(iso: &str) -> Option<i64> {
    if let Some(epoch) = iso.strip_prefix('@') {
        return epoch.parse::<i64>().ok();
    }
    let b = iso.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| iso.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    // Days from the civil calendar (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

fn main() {
    let event_loop = EventLoopBuilder::new().build();

    let menu = Menu::new();
    let five = MenuItem::new("5-hour  —", false, None);
    let seven = MenuItem::new("7-day  —", false, None);
    let account = MenuItem::new("—", false, None);
    let open = MenuItem::new("Open dashboard", true, None);
    let refresh = MenuItem::new("Refresh now", true, None);
    let quit = MenuItem::new("Quit", true, None);
    menu.append_items(&[
        &five,
        &seven,
        &PredefinedMenuItem::separator(),
        &account,
        &PredefinedMenuItem::separator(),
        &open,
        &refresh,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .ok();

    let open_id = open.id().clone();
    let refresh_id = refresh.id().clone();
    let quit_id = quit.id().clone();

    let mut tray = None;
    let mut next_poll = std::time::Instant::now();

    let menu_rx = MenuEvent::receiver();
    let _ = TrayIconEvent::receiver();

    event_loop.run(move |_event, _target, control_flow| {
        // Wake regularly so the readout refreshes without any user interaction.
        *control_flow = ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(500));

        // The tray icon must be created after the event loop is running, or macOS
        // never draws it.
        if tray.is_none() {
            tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu.clone()))
                .with_title("AR …")
                .with_tooltip("Agent Router")
                .build()
                .ok();
        }

        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == quit_id {
                *control_flow = ControlFlow::Exit;
                return;
            } else if ev.id == open_id {
                let _ = std::process::Command::new("/usr/bin/open")
                    .arg(format!("http://{HOST}:{}", port()))
                    .spawn();
            } else if ev.id == refresh_id {
                next_poll = std::time::Instant::now();
            }
        }

        if std::time::Instant::now() >= next_poll {
            next_poll = std::time::Instant::now() + POLL;
            let q = poll();
            if let Some(t) = tray.as_ref() {
                t.set_title(Some(q.title()));
            }
            match &q.error {
                Some(err) => {
                    five.set_text("5-hour  —");
                    seven.set_text("7-day  —");
                    account.set_text(format!("⚠︎ {err}"));
                }
                None => {
                    five.set_text(format!(
                        "5-hour   {}   ·   {}",
                        q.five_hour.map(|v| format!("{v:.0}%")).unwrap_or("—".into()),
                        resets_in(&q.five_hour_resets)
                    ));
                    seven.set_text(format!(
                        "7-day    {}   ·   {}",
                        q.seven_day.map(|v| format!("{v:.0}%")).unwrap_or("—".into()),
                        resets_in(&q.seven_day_resets)
                    ));
                    account.set_text(match q.source.as_str() {
                        // Say where the numbers came from: header data only moves when
                        // traffic flows, so a stale reading should not look live.
                        "response-headers" => {
                            format!("{}  ·  from last request", active_account_line())
                        }
                        _ => active_account_line(),
                    });
                }
            }
        }
    });
}

/// The account line in the menu, so a glance says which one is serving.
fn active_account_line() -> String {
    let Fetch::Body(body) = get("/api/accounts") else {
        return "account —".into();
    };
    let Ok(list) = serde_json::from_str::<Value>(&body) else {
        return "account —".into();
    };
    list.as_array()
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("active").and_then(Value::as_i64) == Some(1))
        })
        .map(|a| {
            let name = a
                .get("email")
                .and_then(Value::as_str)
                .or_else(|| a.get("label").and_then(Value::as_str))
                .unwrap_or("account");
            let plan = a.get("subscription_type").and_then(Value::as_str).unwrap_or("");
            if plan.is_empty() { name.to_string() } else { format!("{name} · {plan}") }
        })
        .unwrap_or_else(|| "account —".into())
}
