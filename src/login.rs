//! Driving `claude setup-token` on the user's behalf.
//!
//! `setup-token` mints a **long-lived (1-year)** credential, which is exactly what an
//! account here wants: nothing to refresh, and no contention with a running Claude
//! Code. It is a terminal program, but it only needs a real pty — given one, with a
//! window size set, it runs perfectly well under another process. (An earlier attempt
//! through `script` produced no output at all, which is what made this look
//! impossible; it was the pty that was inadequate, not the approach.)
//!
//! So the router spawns it in a pty, points `$BROWSER` at a helper that captures the
//! sign-in URL instead of opening anything, hands that URL to the dashboard, and
//! waits for the token to appear on the terminal. The user clicks one link.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[allow(unused_imports)]
use anyhow::{anyhow, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Spawned; waiting for the sign-in URL to appear.
    Starting,
    /// The URL is ready and the user needs to visit it.
    AwaitingSignIn,
    /// Signed in; the token has been captured and the account created.
    Done,
    Failed,
}

#[derive(Debug, Default)]
pub struct Flow {
    pub phase: Option<Phase>,
    pub url: Option<String>,
    pub label: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub error: Option<String>,
    /// Terminal output, kept for diagnosis when something goes wrong.
    pub log: String,
}

impl Flow {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "phase": self.phase.unwrap_or(Phase::Failed),
            "url": self.url,
            "label": self.label,
            "account_id": self.account_id,
            "email": self.email,
            "error": self.error,
            // Never the token: only what is safe to show if this is ever screenshotted.
            "tail": self.log.chars().rev().take(400).collect::<String>()
                .chars().rev().collect::<String>(),
        })
    }
}

pub type Shared = Arc<Mutex<Flow>>;

/// Strip ANSI escapes so the output can be pattern-matched.
fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\u{7}' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn find_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c.is_control() || c == '"' || c == '\'')
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ')']).to_string();
    url.contains("oauth").then_some(url)
}

fn find_token(text: &str) -> Option<String> {
    let start = text.find("sk-ant-oat01-")?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    // A truncated capture is worse than none: the shortest real token seen is ~100.
    (end >= 80).then(|| rest[..end].to_string())
}

/// A `$BROWSER` stand-in: records the URL rather than opening anything, so the
/// dashboard can present it instead of a browser appearing out of nowhere.
fn write_browser_shim(dir: &std::path::Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = dir.join("browser-shim.sh");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o700)
            .open(&path)?;
        writeln!(f, "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"$CAPTURE_FILE\"")?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Err(anyhow!("browser capture is not implemented on this platform"))
    }
}

/// Start a login. Returns immediately; progress is polled from `Shared`.
pub fn start(label: String, shared: Shared) -> Result<()> {
    let id = crate::accounts::slugify(&label);
    let profile = crate::accounts::profiles_root()?.join(&id);
    std::fs::create_dir_all(&profile)?;
    let capture = profile.join("browser-url.txt");
    let _ = std::fs::remove_file(&capture);
    let shim = write_browser_shim(&profile)?;

    {
        let mut f = shared.lock().unwrap();
        *f = Flow {
            phase: Some(Phase::Starting),
            label: label.clone(),
            ..Default::default()
        };
    }

    let pty = portable_pty::native_pty_system();
    let pair = pty
        .openpty(portable_pty::PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening a pty")?;

    let mut cmd = portable_pty::CommandBuilder::new("claude");
    cmd.arg("setup-token");
    cmd.env("CLAUDE_CONFIG_DIR", &profile);
    cmd.env("BROWSER", &shim);
    cmd.env("CAPTURE_FILE", &capture);
    cmd.env("TERM", "xterm-256color");
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }

    let mut child = pair.slave.spawn_command(cmd).context(
        "starting `claude setup-token` — is the claude CLI on PATH?",
    )?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;

    std::thread::spawn(move || {
        let mut raw = String::new();
        let mut buf = [0u8; 4096];
        let mut token: Option<String> = None;

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => break,
            }
            let text = plain(&raw);
            let mut f = shared.lock().unwrap();
            f.log = text.clone();

            if f.url.is_none() {
                let found = std::fs::read_to_string(&capture)
                    .ok()
                    .and_then(|t| find_url(&t))
                    .or_else(|| find_url(&text));
                if let Some(url) = found {
                    f.url = Some(url);
                    f.phase = Some(Phase::AwaitingSignIn);
                }
            }
            if token.is_none() {
                if let Some(t) = find_token(&text) {
                    token = Some(t);
                }
            }
        }

        let _ = child.wait();
        let mut f = shared.lock().unwrap();
        match token {
            Some(token) => {
                let email = crate::accounts::probe_profile(Some(&profile))
                    .ok()
                    .and_then(|s| s.email);
                f.email = email.clone();
                f.account_id = Some(id.clone());
                match finish(&id, email.as_deref().unwrap_or(&f.label), &token, email.as_deref()) {
                    Ok(()) => f.phase = Some(Phase::Done),
                    Err(e) => {
                        f.phase = Some(Phase::Failed);
                        f.error = Some(e.to_string());
                    }
                }
            }
            None => {
                f.phase = Some(Phase::Failed);
                f.error = Some(
                    "the sign-in did not complete, so no token was issued".to_string(),
                );
            }
        }
        // The profile directory has done its job; the token is ours now.
        let _ = std::fs::remove_file(&capture);
        let _ = std::fs::remove_file(&shim);
    });

    Ok(())
}

/// Store the minted token as an ordinary long-lived account.
fn finish(id: &str, label: &str, token: &str, email: Option<&str>) -> Result<()> {
    crate::accounts::store_token(id, token)?;
    let db = crate::config::db_path()?;
    let store = crate::accounts::Store::new(db);
    let status = crate::accounts::AuthStatus {
        email: email.map(str::to_owned),
        ..Default::default()
    };
    store.add(id, label, crate::accounts::Source::Token, None, &status)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_the_authorize_url_out_of_terminal_noise() {
        let raw = "\u{1b}[2m·\u{1b}[22m Opening browser to sign in…\r\n\
                   https://claude.com/cai/oauth/authorize?code=true&client_id=abc&x=1\r\n";
        let url = find_url(&plain(raw)).expect("finds a url");
        assert!(url.starts_with("https://claude.com/cai/oauth/authorize?"));
        assert!(!url.contains('\r') && !url.contains(' '));
    }

    #[test]
    fn ignores_urls_that_are_not_the_sign_in_link() {
        assert_eq!(find_url("see https://docs.claude.com/help for more"), None);
    }

    #[test]
    fn accepts_a_full_token_and_rejects_a_truncated_one() {
        let full = format!("sk-ant-oat01-{}", "A".repeat(95));
        assert_eq!(find_token(&format!("token: {full}\n")).as_deref(), Some(full.as_str()));
        // A short match is a partially-drawn line, not a credential.
        assert_eq!(find_token("sk-ant-oat01-ABC\n"), None);
    }

    #[test]
    fn strips_ansi_without_eating_content() {
        assert_eq!(plain("\u{1b}[31mred\u{1b}[0m text"), "red text");
        assert_eq!(plain("\u{1b}]0;title\u{7}body"), "body");
    }
}

// ---------------------------------------------------------------------------
// Web API
// ---------------------------------------------------------------------------

pub mod api {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde::Deserialize;
    use serde_json::Value;

    use super::*;
    use crate::proxy::AppState;
    use crate::refusal::{failed, refuse, JsonBody};

    #[derive(Deserialize)]
    pub struct StartReq {
        pub label: String,
    }

    /// Refusals answer with an error status (see `refusal`), and the words under `error`
    /// as they always were.
    pub async fn start_login(
        State(state): State<Arc<AppState>>,
        JsonBody(req): JsonBody<StartReq>,
    ) -> Response {
        let label = req.label.trim().to_string();
        if label.is_empty() {
            // 400: the account needs a name, and the request has none.
            return refuse(StatusCode::BAD_REQUEST, "give the account a name first");
        }
        // One at a time: `setup-token` binds a fixed local callback port, so two
        // flows at once would collide. 409: the request is fine, and once the running
        // sign-in finishes or fails, the same one goes through.
        {
            let flow = state.login.lock().unwrap();
            if flow.phase == Some(Phase::Starting) || flow.phase == Some(Phase::AwaitingSignIn) {
                return refuse(StatusCode::CONFLICT, "a sign-in is already in progress");
            }
        }
        let shared = state.login.clone();
        match tokio::task::spawn_blocking(move || super::start(label, shared)).await {
            Ok(Ok(())) => Json(state.login.lock().unwrap().to_json()).into_response(),
            // 500: the router could not start `claude setup-token` — no profile directory,
            // no pty, or no `claude` on its PATH — and the request had nothing to do with it.
            Ok(Err(e)) => failed(e),
            Err(e) => failed(e),
        }
    }

    pub async fn login_status(State(state): State<Arc<AppState>>) -> Json<Value> {
        let done = {
            let flow = state.login.lock().unwrap();
            flow.phase == Some(Phase::Done)
        };
        if done {
            // A new account changes what can be signed with.
            state.invalidate().await;
        }
        let json = state.login.lock().unwrap().to_json();
        Json(json)
    }
}
