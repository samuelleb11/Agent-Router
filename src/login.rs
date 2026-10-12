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
//!
//! That terminal is where the token is printed, so the end of it that a status carries
//! for diagnosis has every token cut out before it is kept (see [`Tail`]).

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
    /// The end of the terminal, kept for diagnosis when something goes wrong. Private, so
    /// that nothing reaches it except through [`Tail::push`], which cuts the tokens out.
    tail: Tail,
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
            // Never the token: it is cut out before the tail keeps anything, and once it
            // is captured there is no tail at all (null).
            "tail": self.tail.shown(),
        })
    }
}

pub type Shared = Arc<Mutex<Flow>>;

/// How much of the terminal's end a status carries.
const TAIL_CHARS: usize = 400;

/// How every token the router knows begins: `sk-ant-oat01-` (what `setup-token` mints),
/// `sk-ant-ort01-` and `sk-ant-api03-` (`accounts::token_kind`), and the `sk-ant-sid…`
/// session key; and all that a replacement token is checked for (`api::accounts_edit`). The
/// tail cuts out anything that starts with it, whatever family follows.
const TOKEN_PREFIX: &str = "sk-ant-";

/// What a token cut from the tail leaves in its place.
const HIDDEN: &str = "[token hidden]";

/// The characters a token is made of, for capturing one and for cutting one out alike.
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The end of the terminal, for a status to show when a sign-in goes wrong.
///
/// `setup-token` prints the token it mints on this terminal, and a status goes to any
/// client allowed to read one: a browser on the LAN holding the password, or a phone. So
/// every token is cut out on the way in, never at the moment of showing, and what this
/// holds is always safe to send. A token can arrive split across two reads, so the end of
/// a read is left for the next one to decide: an `sk-an` that may begin a token is held
/// back, and a token that runs to the end of a read is cut on into the next.
///
/// Once the token is captured there is no tail at all. Nothing after that needs one, and
/// the terminal goes on showing the token until the program exits.
#[derive(Debug, Default)]
struct Tail {
    /// What a status may carry: already cut, and only the last `TAIL_CHARS` characters.
    shown: String,
    /// The end of the last read, when it could be the start of a token.
    held: String,
    /// The last read ended inside a token, so the next one starts with the rest of it.
    inside: bool,
    /// The token has been captured.
    closed: bool,
}

impl Tail {
    /// The terminal's next stretch of text, its escapes already stripped.
    fn push(&mut self, text: &str) {
        if self.closed {
            return;
        }
        let text = std::mem::take(&mut self.held) + text;
        let mut rest = text.as_str();
        if self.inside {
            rest = rest.trim_start_matches(token_char);
            self.inside = rest.is_empty();
        }
        while !self.inside {
            let Some(at) = rest.find(TOKEN_PREFIX) else { break };
            self.shown.push_str(&rest[..at]);
            self.shown.push_str(HIDDEN);
            rest = rest[at + TOKEN_PREFIX.len()..].trim_start_matches(token_char);
            // A token that runs to the end of this read may go on in the next one.
            self.inside = rest.is_empty();
        }
        if !self.inside {
            // `sk-a` at the very end may be the start of a token the next read finishes.
            let held = (1..TOKEN_PREFIX.len())
                .rev()
                .find(|&n| rest.ends_with(&TOKEN_PREFIX[..n]))
                .unwrap_or(0);
            let (now, later) = rest.split_at(rest.len() - held);
            self.shown.push_str(now);
            self.held = later.to_string();
        }
        self.keep_last();
    }

    /// The terminal closed, so what was held back can no longer become a token.
    fn end(&mut self) {
        if !self.closed {
            let held = std::mem::take(&mut self.held);
            self.shown.push_str(&held);
            self.keep_last();
        }
    }

    /// The token is captured: from here on there is no tail.
    fn close(&mut self) {
        *self = Tail { closed: true, ..Tail::default() };
    }

    /// What a status carries: nothing once the token is captured.
    fn shown(&self) -> Option<&str> {
        (!self.closed).then_some(self.shown.as_str())
    }

    fn keep_last(&mut self) {
        let over = self.shown.chars().count().saturating_sub(TAIL_CHARS);
        if over > 0 {
            let cut = self.shown.char_indices().nth(over).map_or(self.shown.len(), |(i, _)| i);
            self.shown.drain(..cut);
        }
    }
}

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

/// How the token `setup-token` mints begins.
const MINTED: &str = "sk-ant-oat01-";

/// The token `setup-token` printed, once something after it says where it ends.
///
/// A read of the terminal can stop anywhere, partway through the token included, and what
/// has been read so far then ends inside it. Taking that end for the token's stored the
/// part read so far, and the account got a credential that did not work. So a run of
/// token characters is taken only once a character that cannot be part of one follows
/// it; a run that reaches the end of what has been read may go on in the next read, and is
/// left for it. Only [`token_at_exit`] lets the end of the text end a token.
fn find_token(text: &str) -> Option<String> {
    token_runs(text)
        .find(|&(run, ended)| ended && whole(run))
        .map(|(run, _)| run.to_string())
}

/// The token, once the program has exited. Nothing more will be read, so a run that
/// reaches the end of its output was ended by the exit, not by a read: it is whole as
/// printed.
fn token_at_exit(text: &str) -> Option<String> {
    token_runs(text).find(|&(run, _)| whole(run)).map(|(run, _)| run.to_string())
}

/// Every run of token characters from [`MINTED`] on, and whether something after it ends it.
fn token_runs(text: &str) -> impl Iterator<Item = (&str, bool)> {
    text.match_indices(MINTED).map(move |(at, _)| {
        let rest = &text[at..];
        match rest.find(|c: char| !token_char(c)) {
            Some(end) => (&rest[..end], true),
            None => (rest, false),
        }
    })
}

/// A truncated capture is worse than none: the shortest real token seen is ~100, so a
/// shorter run is a line still being drawn, not a credential.
fn whole(run: &str) -> bool {
    run.len() >= 80
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

/// What the reader thread carries from one read of the terminal to the next.
struct Terminal {
    /// Where the `$BROWSER` stand-in writes the sign-in link.
    capture: PathBuf,
    /// Everything read so far, escapes and all. The link and the token are looked for in
    /// the whole of it, so neither is missed for arriving in two reads.
    raw: String,
    /// How much of `plain(raw)` the tail has been given.
    given: usize,
    token: Option<String>,
}

impl Terminal {
    fn new(capture: PathBuf) -> Self {
        Self { capture, raw: String::new(), given: 0, token: None }
    }

    /// One read's worth of the terminal's output.
    fn feed(&mut self, bytes: &[u8], f: &mut Flow) {
        self.raw.push_str(&String::from_utf8_lossy(bytes));
        let text = plain(&self.raw);

        if f.url.is_none() {
            let found = std::fs::read_to_string(&self.capture)
                .ok()
                .and_then(|t| find_url(&t))
                .or_else(|| find_url(&text));
            if let Some(url) = found {
                f.url = Some(url);
                f.phase = Some(Phase::AwaitingSignIn);
            }
        }
        if self.token.is_none() {
            self.token = find_token(&text);
            if self.token.is_some() {
                // Nothing after this needs a tail, and the terminal goes on showing the
                // token until the program exits.
                f.tail.close();
            }
        }
        // `plain` only ever adds to what it gave for less input (an escape cut off at the
        // end of one read gives nothing until the next completes it), so what this read
        // added is everything past `given`.
        f.tail.push(text.get(self.given..).unwrap_or_default());
        self.given = text.len();
    }

    /// The terminal closed: the token, if it printed one.
    fn end(mut self, f: &mut Flow) -> Option<String> {
        if self.token.is_none() {
            self.token = token_at_exit(&plain(&self.raw));
            if self.token.is_some() {
                f.tail.close();
            }
        }
        f.tail.end();
        self.token
    }
}

/// Start a login. Returns immediately; progress is polled from `Shared`. The account is
/// added to the store at `db`, and never over one already there: a name whose id another
/// account has is refused here ([`crate::accounts::Conflict::NameTaken`]), before the
/// profile directory, which would be that account's, is made or `setup-token` run in it.
pub fn start(label: String, db: PathBuf, shared: Shared) -> Result<()> {
    let id = crate::accounts::slugify(&label);
    crate::accounts::Store::new(db.clone()).check_free(&id)?;
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

    // From here the flow says Starting, and only its end may change that. A sign-in whose
    // `claude` never started once stood Starting for good: it read as one still running, and
    // `api::start_login` refused every later start until the router restarted. It ends
    // Failed instead, saying why in the words the start itself answers with.
    let (mut child, mut reader) = match run_setup_token(&profile, &shim, &capture) {
        Ok(running) => running,
        Err(e) => {
            let mut f = shared.lock().unwrap();
            f.phase = Some(Phase::Failed);
            f.error = Some(e.to_string());
            let _ = std::fs::remove_file(&shim);
            return Err(e);
        }
    };

    std::thread::spawn(move || {
        let mut term = Terminal::new(capture.clone());
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => term.feed(&buf[..n], &mut shared.lock().unwrap()),
            }
        }

        let _ = child.wait();
        let mut f = shared.lock().unwrap();
        let token = term.end(&mut f);
        let email = token.as_ref().and_then(|_| {
            crate::accounts::probe_profile(Some(&profile)).ok().and_then(|s| s.email)
        });
        settle(&mut f, &db, &id, token, email);
        // The profile directory has done its job; the token is ours now.
        let _ = std::fs::remove_file(&capture);
        let _ = std::fs::remove_file(&shim);
    });

    Ok(())
}

/// What a sign-in runs: Claude Code's CLI, looked for on the router's PATH.
#[cfg(not(test))]
const CLAUDE: &str = "claude";

/// Under test, a program no PATH holds, so a sign-in a test starts fails to start, as one
/// does on a machine without Claude Code. The real one would begin a sign-in to the test
/// machine's own account, and wait there for someone to finish it.
#[cfg(test)]
const CLAUDE: &str = "claude-is-never-run-under-test";

/// `claude setup-token`, started in a pty of its own with `profile` as its config
/// directory: the program, and its terminal to read.
fn run_setup_token(
    profile: &std::path::Path,
    shim: &std::path::Path,
    capture: &std::path::Path,
) -> Result<(Box<dyn portable_pty::Child + Send + Sync>, Box<dyn Read + Send>)> {
    let pty = portable_pty::native_pty_system();
    let pair = pty
        .openpty(portable_pty::PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening a pty")?;

    let mut cmd = portable_pty::CommandBuilder::new(CLAUDE);
    cmd.arg("setup-token");
    cmd.env("CLAUDE_CONFIG_DIR", profile);
    cmd.env("BROWSER", shim);
    cmd.env("CAPTURE_FILE", capture);
    cmd.env("TERM", "xterm-256color");
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }

    let child = pair.slave.spawn_command(cmd).context(
        "starting `claude setup-token` — is the claude CLI on PATH?",
    )?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    Ok((child, reader))
}

/// How a sign-in ends once `setup-token` has exited: the account it added, or why there
/// is none. `email` is who the profile it ran in says it signed in as.
fn settle(f: &mut Flow, db: &std::path::Path, id: &str, token: Option<String>, email: Option<String>) {
    let Some(token) = token else {
        f.phase = Some(Phase::Failed);
        f.error = Some("the sign-in did not complete, so no token was issued".to_string());
        return;
    };
    f.email = email.clone();
    let label = email.clone().unwrap_or_else(|| f.label.clone());
    match finish(db, id, &label, &token, email.as_deref()) {
        Ok(()) => {
            // Only an account that was added is named: a refused one is another's.
            f.account_id = Some(id.to_string());
            f.phase = Some(Phase::Done);
        }
        Err(e) => {
            f.phase = Some(Phase::Failed);
            f.error = Some(e.to_string());
        }
    }
}

/// Store the minted token as an ordinary long-lived account, in the store at `db`.
///
/// The name was free when the sign-in started, but an account can be added under it
/// while the reader is signing in. `add` refuses it then, and stores the token only once
/// the row is in, so this token never lands on that account's.
fn finish(db: &std::path::Path, id: &str, label: &str, token: &str, email: Option<&str>) -> Result<()> {
    let store = crate::accounts::Store::new(db.to_path_buf());
    let status = crate::accounts::AuthStatus {
        email: email.map(str::to_owned),
        ..Default::default()
    };
    store.add(id, label, crate::accounts::Source::Token, crate::accounts::Provider::Anthropic, None, None, &status, || {
        crate::accounts::store_token(id, token)
    })
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

    // The tail a status carries never holds a token. Each test below drives the reader
    // thread's own step, `Terminal::feed`, one call per read, and reads the status as
    // `login_status` sends it: `Flow::to_json`, whole. No pty, no `claude`, and nothing read
    // from or written to disk.

    use serde_json::Value;

    const LINK: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz";

    /// What `setup-token` (2.1.267) draws while it waits for the reader to sign in: the link
    /// as a hyperlink, and the paste prompt.
    const WAITING: &str = "\u{1b}[2mBrowser didn't open? Use the url below to sign in \u{1b}[22m\r\n\r\n\
        \u{1b}]8;;https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz\u{7}\
        \u{1b}[2mhttps://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz\u{1b}[22m\
        \u{1b}]8;;\u{7}\r\n\r\nPaste code here if prompted > ";

    /// What `setup-token` (2.1.267) draws once it has the token, over the frame before: the
    /// token on a line of its own, in the warning colour, then two more lines. All of it
    /// falls inside the last 400 characters a tail keeps.
    fn success(token: &str) -> String {
        format!(
            "\u{1b}[2K\u{1b}[1A\u{1b}[2K\u{1b}[G\
             \u{1b}[32m\u{2713} Long-lived authentication token created successfully!\u{1b}[39m\r\n\r\n\
             Your OAuth token (valid for 1 year):\r\n\r\n\
             \u{1b}[33m{token}\u{1b}[39m\r\n\r\n\
             \u{1b}[2mStore this token securely. You won't be able to see it again.\u{1b}[22m\r\n\r\n\
             \u{1b}[2mUse this token by setting: export CLAUDE_CODE_OAUTH_TOKEN=<token>\u{1b}[22m\r\n"
        )
    }

    /// A fake token's body: the `-` and `_` real ones carry, and characters nothing else in
    /// these terminals uses, so three of them in a row in a status are a piece of a token.
    const FILLER: &str = "QZ9-JX8_";

    /// A token of the given family, in the shape `find_token` captures. Not a credential.
    fn fake(family: &str) -> String {
        format!("{family}{}", FILLER.repeat(12))
    }

    /// The families `find_token` does not capture, so only the cut keeps them out of a tail.
    const UNCAPTURED: [&str; 3] = ["sk-ant-ort01-", "sk-ant-api03-", "sk-ant-sid01-"];

    /// Whether a status shows any of a token: anything token-shaped, or a piece of a body.
    fn shows(status: &Value) -> bool {
        let text = status.to_string();
        let body = FILLER.repeat(2);
        text.contains("sk-ant-") || (0..FILLER.len()).any(|i| text.contains(&body[i..i + 3]))
    }

    /// A terminal with no `$BROWSER` capture file, so the link is read off the screen. The
    /// path is empty, which names no file, so nothing on disk is read.
    fn terminal() -> Terminal {
        Terminal::new(PathBuf::new())
    }

    #[test]
    fn login_tail_never_carries_the_token_that_arrives_in_one_read() {
        // The token `setup-token` mints, whole in one read: captured, and no tail after it.
        let token = fake("sk-ant-oat01-");
        let mut f = Flow::default();
        let mut term = terminal();
        term.feed(WAITING.as_bytes(), &mut f);
        term.feed(success(&token).as_bytes(), &mut f);
        let status = f.to_json();
        assert!(!shows(&status), "{status}");
        assert_eq!(status["url"], LINK);
        assert_eq!(term.end(&mut f).as_deref(), Some(token.as_str()), "captured whole");

        // Any other token, whole in one read, is cut where it stood, and the text either side
        // of it is kept. A token drawn only partway is one `find_token` will not capture.
        let partway = format!("sk-ant-oat01-{}", FILLER.repeat(3));
        for token in UNCAPTURED.map(fake).into_iter().chain([partway]) {
            let mut f = Flow::default();
            let mut term = terminal();
            term.feed(WAITING.as_bytes(), &mut f);
            term.feed(format!("refresh token: {token}\r\nnot the one wanted\r\n").as_bytes(), &mut f);
            let status = f.to_json();
            assert!(!shows(&status), "{status}");
            let tail = status["tail"].as_str().expect("nothing captured, so a tail is kept");
            assert!(tail.contains("Paste code here if prompted > refresh token: "), "{tail}");
            assert!(tail.ends_with("\r\nnot the one wanted\r\n"), "{tail}");
            assert_eq!(term.end(&mut f), None);
        }
    }

    #[test]
    fn login_tail_never_carries_the_token_split_across_two_reads() {
        // Split at every byte of what `setup-token` draws: inside the escapes round the
        // token, inside `sk-ant-`, inside the token, and either side of it.
        let token = fake("sk-ant-oat01-");
        let frame = success(&token);
        for at in 1..frame.len() {
            let (first, second) = frame.as_bytes().split_at(at);
            let mut f = Flow::default();
            let mut term = terminal();
            term.feed(WAITING.as_bytes(), &mut f);
            term.feed(first, &mut f);
            assert!(!shows(&f.to_json()), "first read, split at {at}: {}", f.to_json());
            term.feed(second, &mut f);
            assert!(!shows(&f.to_json()), "second read, split at {at}: {}", f.to_json());
        }

        // A token nothing captures is cut whichever read it ends in, and the tail goes on
        // after it.
        for token in UNCAPTURED.map(fake) {
            let screen = format!("refresh token: {token}\r\nnot the one wanted\r\n");
            for at in 1..screen.len() {
                let (first, second) = screen.as_bytes().split_at(at);
                let mut f = Flow::default();
                let mut term = terminal();
                term.feed(first, &mut f);
                assert!(!shows(&f.to_json()), "first read, split at {at}: {}", f.to_json());
                term.feed(second, &mut f);
                let status = f.to_json();
                assert!(!shows(&status), "second read, split at {at}: {status}");
                let tail = status["tail"].as_str().expect("nothing captured, so a tail is kept");
                assert!(tail.starts_with("refresh token: "), "split at {at}: {tail}");
                assert!(tail.ends_with("\r\nnot the one wanted\r\n"), "split at {at}: {tail}");
            }
        }
    }

    #[test]
    fn login_tail_never_carries_the_token_once_it_is_captured() {
        let token = fake("sk-ant-oat01-");
        let mut f = Flow { phase: Some(Phase::Starting), label: "work".into(), ..Flow::default() };
        let mut term = terminal();
        term.feed(WAITING.as_bytes(), &mut f);
        // Until the token, the tail is there for diagnosis.
        let tail = f.to_json()["tail"].as_str().map(str::to_owned);
        assert!(tail.is_some_and(|t| t.ends_with("Paste code here if prompted > ")));

        term.feed(success(&token).as_bytes(), &mut f);
        let status = f.to_json();
        assert!(!shows(&status), "{status}");
        assert_eq!(status["tail"], Value::Null, "captured, so no tail: {status}");

        // The terminal goes on: the frame is drawn again, the cursor shown, and the program
        // exits. None of it brings a tail back.
        term.feed(success(&token).as_bytes(), &mut f);
        term.feed(b"\x1b[?25h\r\n", &mut f);
        assert_eq!(f.to_json()["tail"], Value::Null);
        assert_eq!(term.end(&mut f).as_deref(), Some(token.as_str()));
        assert_eq!(f.to_json()["tail"], Value::Null);

        // Settled as the reader thread settles it (not through `finish`, which writes the
        // secrets file): the link, the stage, the account and the error are still all said.
        f.account_id = Some("work".into());
        f.phase = Some(Phase::Done);
        let status = f.to_json();
        assert!(!shows(&status), "{status}");
        assert_eq!(status["tail"], Value::Null);
        assert_eq!(status["phase"], "done");
        assert_eq!(status["url"], LINK);
        assert_eq!(status["label"], "work");
        assert_eq!(status["account_id"], "work");

        f.phase = Some(Phase::Failed);
        f.error = Some("the secrets file would not take it".into());
        let status = f.to_json();
        assert!(!shows(&status), "{status}");
        assert_eq!(status["tail"], Value::Null);
        assert_eq!(status["phase"], "failed");
        assert_eq!(status["error"], "the secrets file would not take it");
    }

    #[test]
    fn login_tail_never_carries_the_token_but_keeps_text_that_holds_none() {
        // With nothing token-shaped on the screen, as when a sign-in goes wrong, the tail is
        // what it always was: the last 400 characters, whole. The reads end on what could
        // begin a token (`…ask`, then `-ant`, and a last `s`), held back only until a later
        // read, or the end, shows that it was none.
        let reads = [
            WAITING,
            WAITING,
            "\r\nOAuth error: signed in as sales@ask",
            "-ant",
            "iques.example, not the account wanted.\r\n",
            "Press Enter to retry or Esc to cancel. Closing in 3s",
        ];
        let mut f = Flow { phase: Some(Phase::Starting), label: "work".into(), ..Flow::default() };
        let mut term = terminal();
        let mut screen = String::new();
        for read in reads {
            term.feed(read.as_bytes(), &mut f);
            screen.push_str(read);
        }
        assert_eq!(term.end(&mut f), None);

        let screen = plain(&screen);
        assert!(screen.chars().count() > 400, "long enough to be cut to its end");
        let end: String = screen.chars().skip(screen.chars().count() - 400).collect();
        let status = f.to_json();
        assert_eq!(status["tail"], end.as_str());
        assert_eq!(status["url"], LINK);
        assert_eq!(status["phase"], "awaiting_sign_in");
        assert_eq!(status["label"], "work");
    }

    // The token is taken whole however the terminal's reads cut it: only once something
    // after it ends it, or the program exits. `fake` makes one of 109 characters, as long as
    // a real one; split 80 to 108 characters in, it was stored cut where the read ended.

    #[test]
    fn setup_token_split_across_reads_is_captured_whole_wherever_the_split_falls() {
        let token = fake("sk-ant-oat01-");
        let frame = success(&token);
        let start = frame.find(&token).expect("the frame shows the token");
        // Every boundary within the token, and either edge of it.
        for at in start..=start + token.len() {
            let (first, second) = frame.as_bytes().split_at(at);
            let mut f = Flow::default();
            let mut term = terminal();
            term.feed(WAITING.as_bytes(), &mut f);
            term.feed(first, &mut f);
            assert_eq!(term.token, None, "nothing has ended the token yet: split at {at}");
            assert!(!shows(&f.to_json()), "split at {at}: {}", f.to_json());
            term.feed(second, &mut f);
            assert_eq!(term.token.as_deref(), Some(token.as_str()), "split at {at}");
            assert_eq!(f.to_json()["tail"], Value::Null, "captured, so no tail: split at {at}");
            assert_eq!(term.end(&mut f).as_deref(), Some(token.as_str()), "split at {at}");
        }
    }

    #[test]
    fn setup_token_split_across_reads_and_cut_off_by_the_exit_is_captured_as_printed() {
        // The program prints the token and goes, with nothing after it — not even the escape
        // that ends its colour — so only the exit says where it ends.
        let token = fake("sk-ant-oat01-");
        let screen = format!("\r\nYour OAuth token (valid for 1 year):\r\n\r\n\u{1b}[33m{token}");
        let start = screen.find(&token).expect("the screen shows the token");
        for at in start..=start + token.len() {
            let (first, second) = screen.as_bytes().split_at(at);
            let mut f = Flow::default();
            let mut term = terminal();
            term.feed(WAITING.as_bytes(), &mut f);
            term.feed(first, &mut f);
            term.feed(second, &mut f);
            assert_eq!(term.token, None, "a further read could still add to it: split at {at}");
            assert!(!shows(&f.to_json()), "split at {at}: {}", f.to_json());
            assert_eq!(term.end(&mut f).as_deref(), Some(token.as_str()), "split at {at}");
            assert_eq!(f.to_json()["tail"], Value::Null, "captured, so no tail: split at {at}");
        }

        // A line drawn only partway when the program went is not a credential.
        let mut f = Flow::default();
        let mut term = terminal();
        term.feed(format!("\u{1b}[33m{}", &token[..60]).as_bytes(), &mut f);
        assert_eq!(term.end(&mut f), None);
        assert!(!shows(&f.to_json()), "{}", f.to_json());
    }

    /// The name was free when the sign-in started and was taken while the reader signed
    /// in: the minted token is not stored over that account's, and the sign-in fails
    /// saying whose the name is. Settled as the reader thread settles it, against a store of
    /// its own.
    #[test]
    fn account_name_clash_when_a_sign_in_ends_stores_nothing_over_the_account() {
        use crate::accounts::{AuthStatus, Source, Store};
        let dir = std::env::temp_dir().join(format!("ar-login-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("agent-router.db");
        crate::db::Db::open(&db).unwrap();
        let store = Store::new(db.clone());
        let theirs = fake("sk-ant-oat01-");
        let ours = format!("sk-ant-oat01-ours-{}", FILLER.repeat(12));
        store
            .add("late-clash-work", "Late clash work", Source::Token, crate::accounts::Provider::Anthropic, None, None, &AuthStatus::default(), || {
                crate::accounts::store_token("late-clash-work", &theirs)
            })
            .unwrap();

        let mut f = Flow { phase: Some(Phase::AwaitingSignIn), label: "Late Clash Work".into(), ..Flow::default() };
        settle(&mut f, &db, "late-clash-work", Some(ours.clone()), Some("someone@example.com".into()));
        assert_eq!(f.phase, Some(Phase::Failed));
        let why = f.error.clone().expect("the sign-in says why it failed");
        assert!(why.contains(r#""late-clash-work""#) && why.contains(r#""Late clash work""#), "{why}");
        assert_eq!(f.account_id, None, "nothing was added, so nothing is named");
        let account = store.get("late-clash-work").unwrap().expect("still there");
        assert_eq!((account.label.as_str(), account.source), ("Late clash work", Source::Token));
        assert_eq!(crate::accounts::resolve(&account).unwrap().token, theirs, "its own token");

        // A name still free when the sign-in ends: added, under the address it signed in as.
        let mut f = Flow { phase: Some(Phase::AwaitingSignIn), label: "Late Clash Home".into(), ..Flow::default() };
        settle(&mut f, &db, "late-clash-home", Some(ours.clone()), Some("someone@example.com".into()));
        assert_eq!(f.phase, Some(Phase::Done), "{:?}", f.error);
        assert_eq!(f.account_id.as_deref(), Some("late-clash-home"));
        let account = store.get("late-clash-home").unwrap().expect("added");
        assert_eq!((account.label.as_str(), account.source), ("someone@example.com", Source::Token));
        assert_eq!(crate::accounts::resolve(&account).unwrap().token, ours);
        let _ = std::fs::remove_dir_all(&dir);
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
        let db = state.config.db_path.clone();
        match tokio::task::spawn_blocking(move || super::start(label, db, shared)).await {
            Ok(Ok(())) => Json(state.login.lock().unwrap().to_json()).into_response(),
            // 409 for a name whose id another account has: its token would have gone over
            // that account's (`crate::accounts::Conflict`). Otherwise 500: the router could
            // not start `claude setup-token` — no profile directory, no pty, or no `claude`
            // on its PATH — and the request had nothing to do with it.
            Ok(Err(e)) => crate::api::not_made(e),
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
