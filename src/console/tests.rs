//! The console as a browser, a launcher and a build meet it. Each test names the contracts it
//! holds: the SV-C ids of "Serving" in docs/web-console-contracts.md.
//!
//! The tests drive the app `serve` builds (`main::app_with`: the same routes, the same guard
//! in front, the same headers) over a store of their own in a temporary directory, serving a
//! console build of their own (`fixture`), except where one says it reads the build this
//! binary embeds. Nothing here listens on a socket, writes outside its temporary directory, asks
//! the Keychain, or names the live router's port: requests go straight into the app, with the
//! peer `serve` would have handed it. (A hosted router's session list reads the archive shelf
//! in the data directory, as it always does; nothing is written there.) Where a contract lives
//! in another file — the client's route table, a launcher, the build script, CI — the test
//! reads that file, so the two cannot drift apart.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, HeaderName, Method, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

use super::{policy, Dist, Files, POLICY};
use crate::proxy::AppState;

/// The password, and the two ways a browser on another machine presents something: Basic with
/// whatever username, `whoever:correct horse battery`, and the same with the wrong password.
const PASSWORD: &str = "correct horse battery";
const BASIC: &str = "Basic d2hvZXZlcjpjb3JyZWN0IGhvcnNlIGJhdHRlcnk=";
const WRONG: &str = "Basic d2hvZXZlcjpub3QgdGhlIHBhc3N3b3Jk";
const CHALLENGE: &str = "Basic realm=\"agent-router\", charset=\"UTF-8\"";

/// This machine, and another on its network (TEST-NET-1, routed nowhere). The Hosts are how a
/// browser on each addresses the router; the port is 8790, not the live router's.
const HERE: [u8; 4] = [127, 0, 0, 1];
const LAN: [u8; 4] = [192, 0, 2, 10];
const LOCAL_HOST: &str = "127.0.0.1:8790";
const LAN_HOST: &str = "192.0.2.1:8790";
/// The one name a hosted router here is given.
const HOSTED_NAME: &str = "router.example.com";

const HTML: &str = "text/html; charset=utf-8";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NOT_SERVED: &str = "agent-router: Host header is not an address we serve\n";
const NO_PASSWORD: &str =
    "agent-router: no password set — set one from the dashboard on the machine running the router\n";

/// The fixture's document. What the real one must say is read from ui/index.html.
const DOC: &str = "<!doctype html><html><head><title>fixture</title><script src=\"/theme-boot.js\"></script>\
<script type=\"module\" src=\"/assets/index-Fx1a2B.js\"></script></head><body><div id=\"root\"></div></body></html>";

/// A console build of the tests' own, shaped as `vite build` writes one: the document, hashed
/// files under assets/, and what public/ puts at the root.
fn fixture() -> Files {
    Files::fixture([
        ("index.html", DOC),
        ("assets/index-Fx1a2B.js", "export {};\n"),
        ("assets/index-Fx1a2B.css", "body{margin:0}\n"),
        ("assets/mark-Fx1a2B.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
        ("theme-boot.js", "document.documentElement.classList.add('ar-light');\n"),
        ("THIRD-PARTY-NOTICES.txt", "The console's third-party notices.\n"),
        ("fonts/IBMPlexSans-Regular.woff2", "wOF2"),
        ("fonts/LICENSE.txt", "SIL Open Font License\n"),
    ])
}

/// Every address the console answers with a file or the document, in the fixture.
const CONSOLE: [&str; 9] = [
    "/",
    "/live",
    "/settings",
    "/settings/accounts",
    "/assets/index-Fx1a2B.js",
    "/theme-boot.js",
    "/fonts/IBMPlexSans-Regular.woff2",
    "/THIRD-PARTY-NOTICES.txt",
    "/favicon.svg",
];

/// A router over a store of its own, built as `serve` builds one, serving `files` as its console.
struct Lab {
    app: Router,
    state: Arc<AppState>,
    dir: PathBuf,
}

impl Lab {
    fn local(name: &str, files: Files) -> Self {
        Self::new(name, files, false)
    }

    /// A hosted router: its password pinned, as a deployment pins it at every start, and the
    /// one name it was given.
    async fn hosted(name: &str, files: Files) -> Self {
        let lab = Self::new(name, files, true);
        lab.state.auth.pin(&lab.state.config.db_path, PASSWORD).await.unwrap();
        lab
    }

    fn new(name: &str, files: Files, server: bool) -> Self {
        // As `serve` does before anything builds a TLS client.
        rustls::crypto::ring::default_provider().install_default().ok();
        let dir = std::env::temp_dir().join(format!("ar-console-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("agent-router.db");
        crate::db::Db::open(&db_path).unwrap();
        // By hand rather than by `Config::load`, which settles the data directory for the
        // rest of the process.
        let config = crate::config::Config {
            server,
            bind: SocketAddr::from((HERE, 0)),
            upstream: "https://upstream.invalid".into(),
            db_path,
            body_cap: 64 * 1024,
            dump_headers: 0,
            allow_hosts: if server { vec![HOSTED_NAME.to_string()] } else { Vec::new() },
            public_url: server.then(|| format!("https://{HOSTED_NAME}")),
            push_keys_dir: None,
            agents_dir: server.then(|| dir.join("agents")),
            drain: Duration::from_secs(1),
        };
        let (capture, _) = tokio::sync::mpsc::channel(1);
        // The switch's other half is dropped, never supervised: nothing here opens a socket.
        let (network, _) = crate::network::switch();
        let state = AppState::new(config, capture, network).unwrap();
        Self { app: crate::app_with(state.clone(), files), state, dir }
    }

    async fn set_password(&self) {
        self.state.auth.set(&self.state.config.db_path, PASSWORD).await.unwrap();
    }

    async fn ask(&self, ask: Ask) -> Answer {
        let mut req = Request::builder().method(ask.method).uri(&ask.path);
        if let Some(host) = &ask.host {
            req = req.header(header::HOST, host);
        }
        for (name, value) in &ask.headers {
            req = req.header(name, value);
        }
        let mut req = req.body(Body::from(ask.body)).unwrap();
        // What `serve` hands the guard about every connection.
        req.extensions_mut().insert(ConnectInfo(ask.peer));
        let res = self.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = axum::body::to_bytes(res.into_body(), 8 << 20).await.unwrap().to_vec();
        Answer { status, headers, body }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One request: what is asked, the Host it names and the machine it comes from.
struct Ask {
    method: Method,
    path: String,
    host: Option<String>,
    peer: SocketAddr,
    headers: Vec<(HeaderName, String)>,
    body: String,
}

/// A GET from a browser on this machine.
fn get(path: &str) -> Ask {
    Ask {
        method: Method::GET,
        path: path.to_string(),
        host: Some(LOCAL_HOST.to_string()),
        peer: SocketAddr::from((HERE, 50_000)),
        headers: Vec::new(),
        body: String::new(),
    }
}

/// A JSON POST from this machine, with no `Origin`, as a phone or Claude Code sends one.
fn post(path: &str, body: &str) -> Ask {
    let mut ask = get(path).header(header::CONTENT_TYPE, "application/json");
    ask.method = Method::POST;
    ask.body = body.to_string();
    ask
}

impl Ask {
    fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    fn host(mut self, host: &str) -> Self {
        self.host = Some(host.to_string());
        self
    }

    /// From a browser on another machine, which addresses the router by its LAN address.
    fn from_lan(mut self) -> Self {
        self.peer = SocketAddr::from((LAN, 50_000));
        self.host = Some(LAN_HOST.to_string());
        self
    }

    fn header(mut self, name: HeaderName, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn header(&self, name: HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("no JSON in {}: {}", self.status, self.text()))
    }

    /// The fixture's document, with the headers every address of the app sends it with.
    fn assert_document(&self, what: &str) {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text());
        assert_eq!(self.text(), DOC, "{what}");
        assert_eq!(self.header(header::CONTENT_TYPE), Some(HTML), "{what}");
        assert_eq!(self.header(header::CACHE_CONTROL), Some("no-store"), "{what}");
        assert_eq!(self.header(header::CONTENT_SECURITY_POLICY), Some(POLICY), "{what}");
        assert!(self.headers.get(header::LOCATION).is_none(), "{what}: answered in place");
    }

    /// The proxy's own answer to a path outside `/v1/`: so no route of the console's took it.
    fn assert_proxy_404(&self, what: &str) {
        assert_eq!(self.status, StatusCode::NOT_FOUND, "{what}: {}", self.text());
        assert_eq!(self.text(), "agent-router: not an API path\n", "{what}");
    }

    /// A browser's challenge: HTTP Basic, which it answers with its own prompt.
    fn assert_challenged(&self, what: &str) {
        assert_eq!(self.status, StatusCode::UNAUTHORIZED, "{what}: {}", self.text());
        assert_eq!(self.header(header::WWW_AUTHENTICATE), Some(CHALLENGE), "{what}");
    }
}

/// The guard's off-host switch is one per process, as the socket it follows is one per router.
/// A test that depends on where it stands holds this for its length, and leaves it off.
static SWITCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Switch {
    _held: tokio::sync::MutexGuard<'static, ()>,
}

impl Switch {
    fn set(&self, on: bool) {
        crate::guard::allow_off_host(on);
    }
}

impl Drop for Switch {
    fn drop(&mut self) {
        crate::guard::allow_off_host(false);
    }
}

/// The network listener on (as Settings' switch, or a hosted router, leaves it) or off.
async fn network(on: bool) -> Switch {
    let switch = Switch { _held: SWITCH.lock().await };
    switch.set(on);
    switch
}

/// A file of this repository: the client's route table, a launcher, the build script, CI.
fn source(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What follows each `marker` in `text`, up to the next single quote.
fn quoted_after<'a>(text: &'a str, marker: &str) -> Vec<&'a str> {
    text.match_indices(marker)
        .filter_map(|(at, _)| text[at + marker.len()..].split_once('\'').map(|(value, _)| value))
        .collect()
}

/// `html` with its comments taken out.
fn uncommented(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = rest[start..].split_once("-->").map_or("", |(_, after)| after);
    }
    out.push_str(rest);
    out
}

/// Each `<name …>` tag in `html`, from its `<` to its `>`.
fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    html.match_indices(&open)
        .map(|(at, _)| &html[at..])
        .filter(|tag| tag[open.len()..].starts_with(|c: char| c.is_whitespace() || c == '>'))
        .map(|tag| &tag[..=tag.find('>').expect("a tag that closes")])
        .collect()
}

/// The value of `name="…"` in a tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!(" {name}=\"");
    let at = tag.find(&marker)? + marker.len();
    tag[at..].split_once('"').map(|(value, _)| value)
}

/// One document's rules, as written or as built: no inline script or style, the theme's script
/// first and classic, every file by absolute path from the app's base, nothing from another
/// origin, no `<base>`. Returns the paths it refers to.
fn check_document(what: &str, html: &str) -> Vec<String> {
    let html = uncommented(html);
    let scripts = tags(&html, "script");
    assert!(scripts.len() >= 2, "{what}: the theme's script and the app's");
    for tag in &scripts {
        assert!(attr(tag, "src").is_some(), "{what}: an inline script: {tag}");
        let after = &html[html.find(tag).unwrap() + tag.len()..];
        assert!(after.starts_with("</script>"), "{what}: a script with a body: {tag}");
    }
    assert_eq!(attr(scripts[0], "src"), Some("/theme-boot.js"), "{what}: the theme's script comes first");
    assert!(attr(scripts[0], "type").is_none(), "{what}: and is classic, so it runs before anything paints");
    assert!(scripts[1..].iter().any(|t| attr(t, "type") == Some("module")), "{what}: then the app");
    assert!(!html.contains("<style") && !html.contains(" style=\""), "{what}: an inline style");
    assert!(tags(&html, "base").is_empty(), "{what}: a <base> would move every path");
    assert!(!html.contains("http://") && !html.contains("https://"), "{what}: another origin");
    let refs: Vec<String> = ["script", "link", "img"]
        .iter()
        .flat_map(|name| tags(&html, name))
        .filter_map(|tag| attr(tag, "src").or_else(|| attr(tag, "href")))
        .map(str::to_string)
        .collect();
    for path in &refs {
        assert!(path.starts_with('/') && !path.starts_with("//"), "{what}: {path} is not a path from the app's base");
    }
    refs
}

/// The addresses the client's router can land on, from its own route table: each literal path
/// its routes declare in ui/src/app, the screens' paths in ui/src/lib/routes.ts, and
/// `/settings/<slug>` for each settings page there.
fn client_addresses() -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    let routes = source("ui/src/lib/routes.ts");
    for slug in quoted_after(&routes, "slug: '") {
        found.insert(format!("/settings/{slug}"));
    }
    for path in quoted_after(&routes, ": '/") {
        found.insert(format!("/{path}"));
    }
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src/app");
    for entry in std::fs::read_dir(&app).unwrap() {
        let file = entry.unwrap().path();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".tsx") || name.contains(".spec.") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for path in quoted_after(&text, "path: '") {
            match path {
                "/" => found.insert("/".to_string()),
                // A parameter is a settings page (above); `*` is the client's own "not found".
                p if p.contains([':', '*']) => continue,
                p => found.insert(format!("/{}", p.trim_start_matches('/'))),
            };
        }
    }
    found.into_iter().collect()
}

/// The lines of one job in a CI file, from its name at the margin to the next one.
fn job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n{name}:"))
        .unwrap_or_else(|| panic!("no job {name} in .gitlab-ci.yml"))
        + 1;
    let rest = &ci[start..];
    let end = rest
        .match_indices('\n')
        .map(|(at, _)| at + 1)
        .find(|&at| rest[at..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '.'))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The paths of every route in a route table written as `.route("<path>", post(…))`: a write and
/// nothing else on the same path.
fn writes_only(table: &str) -> Vec<&str> {
    table
        .match_indices(".route(\"")
        .filter_map(|(at, marker)| {
            let (path, rest) = table[at + marker.len()..].split_once('"')?;
            let method = rest.strip_prefix(", ")?.lines().next()?;
            (method.starts_with("post(") && !method.contains(".get(")).then_some(path)
        })
        .collect()
}

/// SV-C4, SV-C20, SV-C23, SV-C25, SV-C27, SV-C28: `/` and `/live`, and every other address
/// of the app, answer one document under one policy. It is `no-store`. The CSP is enforced,
/// not report-only, with `base-uri 'none'`, `form-action 'none'`, and a frame policy that
/// admits VS Code's own schemes alone. No `X-Frame-Options` is sent.
#[tokio::test]
async fn serves_root_and_live_as_one_document_under_one_policy() {
    let lab = Lab::local("document", fixture());
    for path in ["/", "/live", "/live?session=0f3c", "/settings", "/settings/network"] {
        let answer = lab.ask(get(path)).await;
        answer.assert_document(path);
        assert!(answer.headers.get("content-security-policy-report-only").is_none(), "{path}: enforced");
        assert!(answer.headers.get(header::X_FRAME_OPTIONS).is_none(), "{path}: no X-Frame-Options");
        assert_eq!(answer.header(header::X_CONTENT_TYPE_OPTIONS), Some("nosniff"), "{path}");
    }
    let directives: Vec<&str> = POLICY.split(';').map(str::trim).collect();
    for directive in ["base-uri 'none'", "form-action 'none'", "object-src 'none'", "script-src 'self'", "style-src 'self'"] {
        assert!(directives.contains(&directive), "{directive}");
    }
    assert_eq!(directives.last(), Some(&"frame-ancestors vscode-webview: vscode-file:"));
}

/// SV-C3: every address the app can show answers the document on a reload, the addresses read
/// from the client's own route table rather than listed here, so a screen added there and not
/// to the router fails this.
#[tokio::test]
async fn every_address_the_app_shows_answers_the_document_on_a_reload() {
    let addresses = client_addresses();
    for must in ["/", "/live", "/settings", "/settings/accounts", "/settings/router"] {
        assert!(addresses.iter().any(|a| a == must), "the client's table was not read: {addresses:?}");
    }
    let lab = Lab::local("reload", fixture());
    for path in &addresses {
        lab.ask(get(path)).await.assert_document(path);
    }
}

/// SV-C5, SV-C21: a hashed file answers as itself, `immutable`, typed by its extension. One this
/// binary does not hold, such as another build's chunk, is a plain 404 and never the document,
/// and no path out of assets/ reaches anything.
#[tokio::test]
async fn a_hashed_asset_is_immutable_and_an_unknown_one_is_a_plain_404() {
    let lab = Lab::local("assets", fixture());
    for (path, kind) in [
        ("/assets/index-Fx1a2B.js", "text/javascript"),
        ("/assets/index-Fx1a2B.css", "text/css"),
        ("/assets/mark-Fx1a2B.svg", "image/svg+xml"),
    ] {
        let answer = lab.ask(get(path)).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert_eq!(answer.header(header::CONTENT_TYPE), Some(kind), "{path}");
        assert_eq!(answer.header(header::CACHE_CONTROL), Some(IMMUTABLE), "{path}");
    }
    for path in [
        "/assets/index-Gone99.js",
        "/assets/index.html",
        "/assets/../index.html",
        "/assets/%2e%2e/index.html",
        "/assets/",
        "/assets",
    ] {
        let answer = lab.ask(get(path)).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
        assert_ne!(answer.text(), DOC, "{path}: never the document");
        assert!(answer.header(header::CONTENT_TYPE).is_some_and(|t| t.starts_with("text/plain")), "{path}");
    }
}

/// SV-C2: the console never takes the fallback and registers no wildcard at the root. Claude
/// Code's POST /v1/messages still reaches the proxy, which refuses a call with no credential in
/// its own words, not a page and not a 405. A path the console does not name is the proxy's
/// 404: a root file the build lacks, `/index.html`, an unknown API path. A build with files
/// under the router's own names takes none of them over, and one whose name reads as a route
/// pattern is left unrouted rather than stopping the router.
#[tokio::test]
async fn the_console_takes_no_fallback_and_no_wildcard_and_v1_reaches_the_proxy() {
    let hostile = Files::fixture([
        ("index.html", DOC),
        ("v1/messages", "not Claude Code's"),
        ("api/auth", "not the API's"),
        ("_router/health", "not health"),
        ("live", "not the document"),
        ("settings/accounts", "not the document"),
        ("favicon.svg", "<svg/>"),
        ("{section}.js", "a pattern"),
        ("*.js", "a wildcard"),
        ("theme-boot.js", "/* boot */"),
    ]);
    for (name, files) in [("fallback", fixture()), ("fallback-hostile", hostile)] {
        let lab = Lab::local(name, files);
        let refused = (StatusCode::UNAUTHORIZED, "agent-router: no credential presented\n".to_string());
        let answer = lab.ask(post("/v1/messages", "{}")).await;
        assert_eq!((answer.status, answer.text()), refused, "{name}: POST /v1/messages");
        let answer = lab.ask(get("/v1/models")).await;
        assert_eq!((answer.status, answer.text()), refused, "{name}: GET /v1/models");
        for path in ["/nope.js", "/index.html", "/live/extra", "/api/nope", "/fonts", "/theme-boot.js/x"] {
            lab.ask(get(path)).await.assert_proxy_404(&format!("{name}: {path}"));
        }
        lab.ask(get("/live")).await.assert_document(name);
        lab.ask(get("/settings/accounts")).await.assert_document(name);
        let health = lab.ask(get("/_router/health")).await.json();
        assert_eq!(health["version"], env!("CARGO_PKG_VERSION"), "{name}");
        assert_eq!(lab.ask(get("/api/auth")).await.json()["mode"], "local", "{name}");
        assert!(lab.ask(get("/favicon.svg")).await.text().starts_with("<svg xmlns="), "{name}");
    }
}

/// Files at the build's root keep their names from build to build, so they are revalidated:
/// `no-cache` and an ETag, then a 304 with no body for a copy the browser holds. The hashed files
/// are kept instead. A file the build does not have under one of its directories is a plain 404.
#[tokio::test]
async fn files_at_the_root_are_revalidated_by_their_etag() {
    let lab = Lab::local("revalidate", fixture());
    for path in ["/theme-boot.js", "/THIRD-PARTY-NOTICES.txt", "/fonts/LICENSE.txt"] {
        let first = lab.ask(get(path)).await;
        assert_eq!(first.status, StatusCode::OK, "{path}");
        assert_eq!(first.header(header::CACHE_CONTROL), Some("no-cache"), "{path}");
        let etag = first.header(header::ETAG).expect("an ETag").to_string();
        let again = lab.ask(get(path).header(header::IF_NONE_MATCH, &etag)).await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED, "{path}");
        assert!(again.body.is_empty(), "{path}");
        let weak = lab.ask(get(path).header(header::IF_NONE_MATCH, &format!("W/{etag}"))).await;
        assert_eq!(weak.status, StatusCode::NOT_MODIFIED, "{path}: a weak match is a match");
        let other = lab.ask(get(path).header(header::IF_NONE_MATCH, "\"another build\"")).await;
        assert_eq!(other.status, StatusCode::OK, "{path}");
    }
    let gone = lab.ask(get("/fonts/IBMPlexSans-Gone.woff2")).await;
    assert_eq!((gone.status, gone.text().as_str()), (StatusCode::NOT_FOUND, "agent-router: no such file\n"));
}

/// SV-C22: every response carries `nosniff`: the console's, the API's, the proxy's and the
/// guard's refusals alike. Every file is served as the type its extension names. The document
/// is text/html; charset=utf-8, scripts text/javascript, styles text/css, fonts font/woff2, and
/// icons image/svg+xml.
#[tokio::test]
async fn every_response_is_nosniff_and_every_file_is_served_as_its_kind() {
    let lab = Lab::local("nosniff", fixture());
    for (path, kind) in [
        ("/", HTML),
        ("/live", HTML),
        ("/assets/index-Fx1a2B.js", "text/javascript"),
        ("/theme-boot.js", "text/javascript"),
        ("/assets/index-Fx1a2B.css", "text/css"),
        ("/fonts/IBMPlexSans-Regular.woff2", "font/woff2"),
        ("/assets/mark-Fx1a2B.svg", "image/svg+xml"),
        ("/favicon.svg", "image/svg+xml"),
        ("/THIRD-PARTY-NOTICES.txt", "text/plain; charset=utf-8"),
    ] {
        let answer = lab.ask(get(path)).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert_eq!(answer.header(header::CONTENT_TYPE), Some(kind), "{path}");
        assert_eq!(answer.header(header::X_CONTENT_TYPE_OPTIONS), Some("nosniff"), "{path}");
    }
    for (what, ask) in [
        ("health", get("/_router/health")),
        ("the API", get("/api/auth")),
        ("the proxy's 404", get("/nope")),
        ("the proxy's refusal", post("/v1/messages", "{}")),
        ("a missing asset", get("/assets/index-Gone99.js")),
        ("the guard, on a name", get("/").host("evil.example.com")),
        ("the guard, off the machine", get("/").from_lan()),
    ] {
        let answer = lab.ask(ask).await;
        assert_eq!(answer.header(header::X_CONTENT_TYPE_OPTIONS), Some("nosniff"), "{what}: {}", answer.status);
    }
}

/// SV-C10: a local router never asks a browser on its own machine for the password, even with
/// one set. Over loopback, by address or by name, it is let straight in to the console.
#[tokio::test]
async fn a_browser_on_this_machine_is_never_asked_for_the_password() {
    let lab = Lab::local("loopback", fixture());
    lab.set_password().await;
    for (host, peer) in [
        ("127.0.0.1:8790", SocketAddr::from((HERE, 50_000))),
        ("localhost:8790", SocketAddr::from((HERE, 50_000))),
        ("[::1]:8790", "[::1]:50000".parse().unwrap()),
    ] {
        for path in ["/", "/live", "/settings/password"] {
            let mut ask = get(path).host(host);
            ask.peer = peer;
            lab.ask(ask).await.assert_document(host);
        }
        let mut ask = get("/assets/index-Fx1a2B.js").host(host);
        ask.peer = peer;
        assert_eq!(lab.ask(ask).await.status, StatusCode::OK, "{host}");
    }
}

/// SV-C11, SV-C12, SV-C18: a browser on another machine, with the network listener on. Before
/// any password exists it is refused outright with 403 and the sentence, and no prompt. With one
/// set it is challenged with HTTP Basic and let in with any username and the password. There is
/// no login page of the console's own. `/_router/health` alone answers it without the password,
/// with pid, version and build; nothing else the console loads does.
#[tokio::test]
async fn a_browser_on_another_machine_meets_basic_and_only_health_is_open() {
    let _on = network(true).await;
    let lab = Lab::local("lan", fixture());
    for path in ["/", "/live", "/assets/index-Fx1a2B.js", "/theme-boot.js"] {
        let answer = lab.ask(get(path).from_lan()).await;
        assert_eq!((answer.status, answer.text().as_str()), (StatusCode::FORBIDDEN, NO_PASSWORD), "{path}");
        assert!(answer.headers.get(header::WWW_AUTHENTICATE).is_none(), "{path}: no prompt");
    }
    lab.set_password().await;
    for path in CONSOLE.iter().chain(&["/api/auth", "/api/summary"]) {
        lab.ask(get(path).from_lan()).await.assert_challenged(path);
    }
    let health = lab.ask(get("/_router/health").from_lan()).await;
    assert_eq!(health.status, StatusCode::OK);
    let health = health.json();
    assert_eq!(health["pid"], std::process::id());
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(health["build"], env!("AGENT_ROUTER_BUILD"));
    lab.ask(get("/").from_lan().header(header::AUTHORIZATION, BASIC)).await.assert_document("any user, the password");
    lab.ask(get("/live").from_lan().header(header::AUTHORIZATION, BASIC)).await.assert_document("/live");
    lab.ask(get("/").from_lan().header(header::AUTHORIZATION, WRONG)).await.assert_challenged("the wrong password");
    for path in ["/login", "/logout", "/signin"] {
        lab.ask(get(path).from_lan().header(header::AUTHORIZATION, BASIC)).await.assert_proxy_404(path);
    }
}

/// SV-C8: off this machine the console sits behind the same guard as the API, because the
/// network listener serves the very app the loopback one does (`serve_here` hands
/// `network::supervise` a clone of it). A caller on another machine without the password is
/// refused the console exactly as it is refused the API, wherever the switch stands. Off, it
/// is not on this machine; on with no password, nobody gets in; on with one, it is asked for it.
#[tokio::test]
async fn a_caller_off_this_machine_without_the_password_is_refused_the_console_as_the_api() {
    assert!(
        source("src/main.rs").contains("network::supervise(app.clone(), port, pending)"),
        "the network listener serves the loopback listener's own app"
    );
    let switch = network(false).await;
    let lab = Lab::local("off-machine", fixture());
    let compare = |what: &'static str| {
        let lab = &lab;
        async move {
            let api = lab.ask(get("/api/summary").from_lan()).await;
            for path in CONSOLE {
                let console = lab.ask(get(path).from_lan()).await;
                assert_eq!((console.status, console.text()), (api.status, api.text()), "{what}: {path}");
                assert_eq!(
                    console.header(header::WWW_AUTHENTICATE),
                    api.header(header::WWW_AUTHENTICATE),
                    "{what}: {path}"
                );
            }
            api
        }
    };
    let off = compare("network off").await;
    assert_eq!((off.status, off.text().as_str()), (StatusCode::FORBIDDEN, "agent-router: caller is not on this machine\n"));
    switch.set(true);
    let no_password = compare("no password").await;
    assert_eq!((no_password.status, no_password.text().as_str()), (StatusCode::FORBIDDEN, NO_PASSWORD));
    lab.set_password().await;
    compare("a password set").await.assert_challenged("the API");
}

/// SV-C13, SV-C14, SV-C19: the console is served only to a Host the guard accepts. That is
/// localhost or a loopback address, and a bare IP literal once the network listener is on. Any
/// other name is refused, parsed rather than matched, so `127.0.0.1.evil.com` and
/// `localhost.evil.com` go the way of any rebinding name. Where it is served it is answered in
/// place, with no redirect to another origin: not from one loopback spelling to another, and
/// not to a LAN address.
#[tokio::test]
async fn only_a_host_the_guard_accepts_is_served_the_console_and_none_is_redirected() {
    let switch = network(false).await;
    let lab = Lab::local("hosts", fixture());
    let paths = ["/", "/live", "/assets/index-Fx1a2B.js", "/theme-boot.js"];
    for host in ["127.0.0.1:8790", "localhost:8790", "LOCALHOST", "[::1]:8790", "127.5.5.5"] {
        for path in paths {
            let answer = lab.ask(get(path).host(host)).await;
            assert_eq!(answer.status, StatusCode::OK, "{host}{path}");
            assert!(answer.headers.get(header::LOCATION).is_none(), "{host}{path}: no redirect");
        }
    }
    let refused = |answer: Answer, what: &str| {
        assert_eq!((answer.status, answer.text().as_str()), (StatusCode::FORBIDDEN, NOT_SERVED), "{what}");
    };
    for host in [
        "evil.example.com",
        "evil.example.com:8790",
        "127.0.0.1.evil.com",
        "localhost.evil.com",
        "router.lan:8790",
        LAN_HOST,
    ] {
        for path in paths {
            refused(lab.ask(get(path).host(host)).await, &format!("{host}{path}"));
        }
    }
    let mut nameless = get("/");
    nameless.host = None;
    let answer = lab.ask(nameless).await;
    assert_eq!((answer.status, answer.text().as_str()), (StatusCode::FORBIDDEN, "agent-router: no Host header\n"));
    // With the network listener on, the router's LAN address is a bare IP and is served; a name
    // still is not, because DNS rebinding needs one.
    switch.set(true);
    for path in paths {
        let answer = lab.ask(get(path).host(LAN_HOST)).await;
        assert_eq!(answer.status, StatusCode::OK, "{LAN_HOST}{path}");
        assert!(answer.headers.get(header::LOCATION).is_none(), "{LAN_HOST}{path}: no redirect");
        refused(lab.ask(get(path).host("router.lan:8790")).await, path);
        refused(lab.ask(get(path).host("192.0.2.1.evil.com")).await, path);
    }
}

/// SV-C40, SV-C41, SV-C42, SV-C14: a hosted router asks every caller for the password, loopback
/// included, and leaves only `/_router/health` open. It serves the console to the names it was
/// given, matched exactly, whatever the case, the port and one trailing dot, and never as a
/// suffix; to IP literals; and to localhost. Any other name gets a 403 that says what it does
/// answer to. On its public name the document carries the same policy and frame policy as
/// anywhere, since neither is optional there.
#[tokio::test]
async fn a_hosted_router_serves_the_console_to_its_names_and_its_password_only() {
    let _on = network(true).await;
    let lab = Lab::hosted("hosted", fixture()).await;
    for path in CONSOLE {
        lab.ask(get(path)).await.assert_challenged(&format!("loopback, {path}"));
    }
    let health = lab.ask(get("/_router/health")).await;
    assert_eq!((health.status, health.json()["ok"].clone()), (StatusCode::OK, Value::Bool(true)));
    for host in ["router.example.com", "Router.Example.COM", "router.example.com:443", "router.example.com.", "10.1.2.3", "localhost:8790"] {
        for path in ["/", "/live", "/settings/accounts"] {
            let answer = lab.ask(get(path).host(host).header(header::AUTHORIZATION, BASIC)).await;
            answer.assert_document(&format!("{host}{path}"));
        }
        let asset = lab.ask(get("/assets/index-Fx1a2B.js").host(host).header(header::AUTHORIZATION, BASIC)).await;
        assert_eq!(asset.status, StatusCode::OK, "{host}");
    }
    let says = format!("agent-router: Host header is not an address we serve (this router answers to {HOSTED_NAME})\n");
    for host in ["router.example.com.evil.com", "evil-router.example.com", "example.com", "evil.example.com"] {
        for path in ["/", "/assets/index-Fx1a2B.js"] {
            let answer = lab.ask(get(path).host(host).header(header::AUTHORIZATION, BASIC)).await;
            assert_eq!((answer.status, answer.text()), (StatusCode::FORBIDDEN, says.clone()), "{host}{path}");
        }
    }
}

/// The agent's installer is open on a hosted router, and only its own addresses are: the two
/// scripts, each carrying the router's public address, and each platform's files as the image
/// holds them. Anything else under `/agent/` is asked for the password, and a local router
/// serves none of it.
#[tokio::test]
async fn a_hosted_router_hands_out_its_agent_installer_without_the_password() {
    let _on = network(true).await;
    let lab = Lab::hosted("installer", fixture()).await;
    let sh = lab.ask(get("/agent/install.sh").host(HOSTED_NAME)).await;
    assert_eq!(sh.status, StatusCode::OK, "{}", sh.text());
    assert_eq!(sh.header(header::CONTENT_TYPE), Some("text/plain; charset=utf-8"));
    assert!(sh.text().contains(&format!("\nSERVER='https://{HOSTED_NAME}'\n")), "{}", sh.text());
    let ps = lab.ask(get("/agent/install.ps1").host(HOSTED_NAME)).await;
    assert_eq!(ps.status, StatusCode::OK, "{}", ps.text());
    assert!(ps.text().contains(&format!("\n$Server = 'https://{HOSTED_NAME}'\n")), "{}", ps.text());

    let none = lab.ask(get("/agent/darwin-arm64/agent-router").host(HOSTED_NAME)).await;
    assert_eq!(none.status, StatusCode::NOT_FOUND, "{}", none.text());
    assert!(none.text().contains("built without the agent's packages"), "{}", none.text());
    let dir = lab.state.config.agents_dir.clone().unwrap().join("win32-x64");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("agent-router.exe"), b"MZ, the program").unwrap();
    let exe = lab.ask(get("/agent/win32-x64/agent-router.exe").host(HOSTED_NAME)).await;
    assert_eq!((exe.status, exe.body.as_slice()), (StatusCode::OK, &b"MZ, the program"[..]));
    assert_eq!(exe.header(header::CONTENT_TYPE), Some("application/octet-stream"));

    for path in ["/agent/", "/agent/win32-x64/claude-wrapper.exe", "/agent/waiting", "/api/machines"] {
        lab.ask(get(path).host(HOSTED_NAME)).await.assert_challenged(path);
    }
    let local = Lab::local("installer-local", fixture());
    local.set_password().await;
    local.ask(get("/agent/install.sh").from_lan()).await.assert_challenged("local, from another machine");
    local.ask(get("/agent/install.sh")).await.assert_proxy_404("local, on its own machine");
}

/// SV-C1: every console route sits behind the one guard, in the hosted route set and the local
/// one alike: the document at each address, the hashed files, the root files and the icon. On a
/// hosted router a Host it was not given is refused all of them, password or not; on a local
/// one a caller on another machine without the password is. A route added after the guard layer
/// would answer both.
#[tokio::test]
async fn every_console_route_sits_behind_the_guard_in_both_route_sets() {
    let _on = network(true).await;
    let hosted = Lab::hosted("guarded-hosted", fixture()).await;
    for path in CONSOLE {
        let answer = hosted.ask(get(path).host("evil.example.com").header(header::AUTHORIZATION, BASIC)).await;
        assert_eq!(answer.status, StatusCode::FORBIDDEN, "hosted {path}");
        assert!(answer.text().starts_with("agent-router: Host header is not an address we serve"), "hosted {path}");
    }
    let local = Lab::local("guarded-local", fixture());
    local.set_password().await;
    for path in CONSOLE {
        local.ask(get(path).from_lan()).await.assert_challenged(&format!("local {path}"));
    }
}

/// SV-C15, SV-C16: a write the console makes is same-origin and passes; the same write from
/// another site, or with `Origin: null`, is refused with 403 before any handler runs. Reads are
/// never origin-checked, so nothing read with GET, HEAD or OPTIONS may change state. No route
/// that only writes answers those methods: each `post` route in main.rs and hosted.rs refuses
/// them, in its own route set. The console's own routes answer GET and HEAD alone.
#[tokio::test]
async fn writes_are_same_origin_and_nothing_read_reaches_a_writer() {
    let lab = Lab::local("origin", fixture());
    let write = || post("/api/auth", r#"{"password":"short"}"#);
    let mine = lab.ask(write().header(header::ORIGIN, "http://127.0.0.1:8790")).await;
    assert_eq!(mine.status, StatusCode::BAD_REQUEST, "past the guard, to the handler's own refusal");
    assert_eq!(mine.json()["error"], "use at least 8 characters");
    for origin in ["https://evil.example.com", "http://127.0.0.1:9999", "null"] {
        let answer = lab.ask(write().header(header::ORIGIN, origin)).await;
        assert_eq!(
            (answer.status, answer.text().as_str()),
            (StatusCode::FORBIDDEN, "agent-router: cross-site request refused\n"),
            "{origin}"
        );
    }
    let read = lab.ask(get("/api/auth").header(header::ORIGIN, "https://evil.example.com")).await;
    assert_eq!(read.status, StatusCode::OK, "a read is not origin-checked");
    assert!(!lab.state.auth.configured().await, "and nothing above changed anything");

    let hosted = Lab::hosted("origin-hosted", fixture()).await;
    for (lab, table) in [(&lab, source("src/main.rs")), (&hosted, source("src/hosted.rs"))] {
        let writers = writes_only(&table);
        assert!(writers.len() >= 10, "the route table was not read: {writers:?}");
        for path in writers {
            for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
                let answer = lab.ask(get(path).method(method.clone()).header(header::AUTHORIZATION, BASIC)).await;
                assert_eq!(answer.status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            }
        }
    }
    for path in CONSOLE {
        let head = lab.ask(get(path).method(Method::HEAD)).await;
        assert_eq!(head.status, StatusCode::OK, "HEAD {path}");
        for method in [Method::POST, Method::PUT, Method::DELETE] {
            let answer = lab.ask(get(path).method(method.clone())).await;
            assert_eq!(answer.status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        }
    }
}

/// SV-C17: the extension's fallback frame keeps `allow-same-origin`, so the console inside it
/// writes with the router's own origin. Without it the frame would send `Origin: null`, which
/// the guard refuses for every write.
#[tokio::test]
async fn the_extensions_fallback_frame_keeps_its_origin() {
    let dashboard = source("vscode/src/dashboard.ts");
    let frames = tags(&dashboard, "iframe");
    assert_eq!(frames.len(), 1, "the fallback frame");
    let sandbox: Vec<&str> = attr(frames[0], "sandbox").expect("a sandboxed frame").split_whitespace().collect();
    for allowed in ["allow-same-origin", "allow-scripts", "allow-forms"] {
        assert!(sandbox.contains(&allowed), "{allowed} in {sandbox:?}");
    }
    let lab = Lab::local("null-origin", fixture());
    let answer = lab.ask(post("/api/auth", r#"{"password":"short"}"#).header(header::ORIGIN, "null")).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
}

/// SV-C6, SV-C24, SV-C26: the document as written (ui/index.html) and, where this tree built the
/// console before compiling, as built and embedded. No inline script ships: the theme's pre-paint
/// snippet is `/theme-boot.js`, a same-origin classic script loaded ahead of the app. Every file
/// is referred to by absolute path from the app's base, so a reload on /settings/<section> or
/// /live?session= still finds it. Nothing comes from another origin. The policy's sources are
/// this origin alone: data: only for images, and the two VS Code schemes only as frame ancestors.
#[test]
fn the_document_loads_no_inline_script_and_nothing_from_another_origin() {
    check_document("ui/index.html", &source("ui/index.html"));
    assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/public/theme-boot.js").is_file());
    for directive in POLICY.split(';').map(str::trim) {
        let (name, sources) = directive.split_once(' ').unwrap();
        for source in sources.split_whitespace() {
            let allowed = matches!(source, "'self'" | "'none'")
                || (name == "img-src" && source == "data:")
                || (name == "frame-ancestors" && matches!(source, "vscode-webview:" | "vscode-file:"));
            assert!(allowed, "{name} admits {source}");
        }
    }
    if let Some(built) = Dist::get("index.html") {
        let html = String::from_utf8(built.data.into_owned()).unwrap();
        for path in check_document("ui/dist/index.html", &html) {
            if path != "/favicon.svg" {
                assert!(Dist::get(path.trim_start_matches('/')).is_some(), "the build refers to {path}, which it lacks");
            }
        }
    }
}

/// SV-C7: `/favicon.svg` keeps answering as image/svg+xml, and is the router's own whatever the
/// build holds. The document links it rather than Northstar's `/favicon.ico`, which has no route.
#[tokio::test]
async fn the_icon_is_the_routers_own_and_the_document_links_it() {
    let lab = Lab::local("icon", fixture());
    let icon = lab.ask(get("/favicon.svg")).await;
    assert_eq!(icon.status, StatusCode::OK);
    assert_eq!(icon.header(header::CONTENT_TYPE), Some("image/svg+xml"));
    assert!(icon.text().starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    lab.ask(get("/favicon.ico")).await.assert_proxy_404("/favicon.ico");
    let document = uncommented(&source("ui/index.html"));
    let icons: Vec<&str> = tags(&document, "link").into_iter().filter(|t| attr(t, "rel") == Some("icon")).collect();
    assert_eq!(icons.len(), 1, "{icons:?}");
    assert_eq!(attr(icons[0], "href"), Some("/favicon.svg"));
    assert!(!document.contains("favicon.ico"));
}

/// SV-C29: the console is compiled in. Every file of `ui/dist` this binary serves is a slice of
/// the binary, borrowed and never read from disk at run time, because the embed is on in every
/// build (`debug-embed`). A tree that has not built the console still compiles (`allow_missing`),
/// to an empty set that serves the placeholder.
#[tokio::test]
async fn the_console_is_compiled_in_and_nothing_is_read_from_disk() {
    let manifest = source("Cargo.toml");
    let line = manifest.lines().find(|l| l.starts_with("rust-embed ")).expect("rust-embed");
    assert!(line.contains("\"debug-embed\""), "{line}");
    let names: Vec<String> = Dist::iter().map(Cow::into_owned).collect();
    for name in &names {
        assert!(matches!(Dist::get(name).unwrap().data, Cow::Borrowed(_)), "{name} was read at run time");
    }
    let lab = Lab::local("embedded", Files::embedded());
    let answer = lab.ask(get("/")).await;
    match Dist::get("index.html") {
        Some(document) => {
            assert_eq!(answer.status, StatusCode::OK);
            assert_eq!(answer.body, document.data.as_ref());
        }
        None => {
            assert!(names.is_empty(), "{names:?}");
            assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(answer.text().contains(policy::BUILD_COMMAND));
        }
    }
}

/// SV-C30: a debug build without the console serves a placeholder that names the command that
/// builds it, under the document's own headers, and the build prints that as a cargo warning.
/// Nothing else is lost: the API and the proxy answer as ever.
#[tokio::test]
async fn without_the_console_a_debug_build_serves_a_placeholder_and_warns() {
    let root = Path::new("/repo");
    let verdict = policy::verdict(false, "debug", None);
    let warning = match &verdict {
        policy::Verdict::Placeholder(warning) => warning.clone(),
        other => panic!("{other:?}"),
    };
    assert!(warning.contains(policy::BUILD_COMMAND), "{warning}");
    assert!(policy::directives(root, &verdict).contains(&format!("cargo:warning={warning}")));
    let build = source("build.rs");
    assert!(build.contains("#[path = \"src/console/policy.rs\"]"), "build.rs decides with this very policy");
    assert!(build.contains("console_policy::directives(root, &verdict)"), "and prints what it says");

    let lab = Lab::local("placeholder", Files::fixture(Vec::<(&str, &str)>::new()));
    for path in ["/", "/live", "/settings/accounts"] {
        let answer = lab.ask(get(path)).await;
        assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert!(answer.text().contains(policy::BUILD_COMMAND), "{path}: {}", answer.text());
        assert_eq!(answer.header(header::CONTENT_TYPE), Some(HTML), "{path}");
        assert_eq!(answer.header(header::CACHE_CONTROL), Some("no-store"), "{path}");
        assert_eq!(answer.header(header::CONTENT_SECURITY_POLICY), Some(POLICY), "{path}");
    }
    assert_eq!(lab.ask(get("/assets/index-Fx1a2B.js")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(lab.ask(get("/api/auth")).await.status, StatusCode::OK);
    assert_eq!(lab.ask(post("/v1/messages", "{}")).await.status, StatusCode::UNAUTHORIZED);
}

/// SV-C31: a release build without the console fails, naming the command, unless
/// `AGENT_ROUTER_ALLOW_NO_UI=1` says to build one that serves the placeholder; with the console
/// built, any build embeds it. Whatever it decides, the build watches `ui/dist`
/// (`rerun-if-changed`) and the variable.
#[test]
fn a_release_without_the_console_fails_unless_told_otherwise() {
    let root = Path::new("/repo");
    let refusal = match policy::verdict(false, "release", None) {
        policy::Verdict::Refuse(why) => why,
        other => panic!("{other:?}"),
    };
    assert!(refusal.contains(policy::BUILD_COMMAND) && refusal.contains("AGENT_ROUTER_ALLOW_NO_UI=1"), "{refusal}");
    for refused in [None, Some("0"), Some(""), Some("yes")] {
        assert!(matches!(policy::verdict(false, "release", refused), policy::Verdict::Refuse(_)), "{refused:?}");
    }
    assert!(matches!(policy::verdict(false, "release", Some("1")), policy::Verdict::Placeholder(_)));
    for profile in ["release", "debug"] {
        assert_eq!(policy::verdict(true, profile, None), policy::Verdict::Embed);
    }
    let watched = [
        format!("cargo:rerun-if-changed={}", root.join("ui").join("dist").display()),
        "cargo:rerun-if-env-changed=AGENT_ROUTER_ALLOW_NO_UI".to_string(),
    ];
    for verdict in [
        policy::verdict(true, "release", None),
        policy::verdict(false, "debug", None),
        policy::verdict(false, "release", Some("1")),
        policy::verdict(false, "release", None),
    ] {
        let lines = policy::directives(root, &verdict);
        for watch in &watched {
            assert!(lines.contains(watch), "{verdict:?}: {lines:?}");
        }
    }
    let lines = policy::directives(root, &policy::verdict(false, "release", None));
    assert!(lines.contains(&format!("cargo::error={refusal}")), "{lines:?}");
}

/// SV-C32: the console is built once, before any cargo step, and never by cargo.
/// build-target.sh runs pnpm before its first release build, and skips it when ui/dist is
/// already there. CI's build:ui job hands ui/dist to every binary job as an artifact. Nothing in
/// build.rs runs anything but git. Git ignores ui/dist, so it is never committed.
#[test]
fn the_console_is_built_once_before_cargo_and_never_by_it() {
    let script = source("scripts/build-target.sh");
    let lines: Vec<&str> = script.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    let pnpm = lines.iter().position(|l| l.contains("pnpm")).expect("build-target.sh builds the console");
    let release = lines.iter().position(|l| l.contains("--release")).expect("and the router");
    assert!(pnpm < release, "the console first");
    assert!(script.contains("[ ! -f ui/dist/index.html ]"), "and only when it is not there already");

    let ci = source(".gitlab-ci.yml");
    let ui = job(&ci, "build:ui");
    assert!(ui.contains("pnpm run build"), "{ui}");
    assert!(ui.contains("artifacts:") && ui.contains("ui/dist"), "{ui}");
    assert!(job(&ci, ".vsix-build").contains("needs: [\"build:ui\"]"), "every binary job takes it");
    for binary in ["build:macos", "build:linux", "build:windows"] {
        assert!(job(&ci, binary).contains("extends: .vsix-build"), "{binary}");
    }

    let build = (source("build.rs") + &source("src/console/policy.rs")).replace('"', "'");
    let spawned = quoted_after(&build, "Command::new('");
    assert_eq!(spawned, ["git", "git"], "build.rs runs git, and nothing that builds the console");
    assert!(source("ui/.gitignore").lines().any(|l| l.trim() == "dist"), "ui/dist is never committed");
}

/// SV-C33: a change to the console alone reaches a router already serving only through a newer
/// CARGO_PKG_VERSION. Health reports the crate version. `--ensure`, which the extension runs in
/// every window it opens, takes the port over only from an older version, by this comparison
/// and nothing else. The same version is left serving, its console changed or not, so a
/// console-only change has to bump the version to ship.
#[tokio::test]
async fn a_console_change_reaches_a_running_router_only_with_a_newer_version() {
    let ours = env!("CARGO_PKG_VERSION");
    let lab = Lab::local("version", fixture());
    assert_eq!(lab.ask(get("/_router/health")).await.json()["version"], ours);
    assert!(source("src/main.rs").contains("Some((pid, theirs)) if !lifecycle::is_newer(ours, &theirs) =>"));
    assert!(!crate::lifecycle::is_newer(ours, ours), "the same version is left serving");
    let parts: Vec<u64> = ours.split('.').map(|p| p.parse().unwrap()).collect();
    let bumped = format!("{}.{}.{}", parts[0], parts[1], parts[2] + 1);
    assert!(crate::lifecycle::is_newer(&bumped, ours), "a bumped one takes over");
}

/// SV-C34: a tab left open across a deploy reloads, once, rather than ask the new binary for a
/// chunk it lacks. The app reloads on `vite:preloadError` (ui/src/main.tsx), and the event fires
/// because a chunk this binary does not hold answers a plain 404, never the document.
#[tokio::test]
async fn a_tab_open_across_a_deploy_reloads_rather_than_take_a_page_for_a_chunk() {
    let main = source("ui/src/main.tsx");
    let (_, handler) = main
        .split_once("window.addEventListener('vite:preloadError'")
        .expect("the app listens for vite:preloadError");
    let handler = &handler[..handler.find("});").expect("one handler")];
    assert!(handler.contains("window.location.reload()"), "{handler}");
    assert!(handler.contains("if (reloaded) return;") && handler.contains("reloaded = true;"), "once: {handler}");
    let lab = Lab::local("deploy", fixture());
    let gone = lab.ask(get("/assets/Overview-FromTheLastBuild.js")).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_ne!(gone.text(), DOC);
}

/// SV-C35, SV-C36: every launcher opens a bare `/` or `/live`, and both keep answering. The
/// menubar's "Open dashboard" opens the router's base URL with no path. The extension's commands
/// and status bar open `/` and `/live`. The iPhone opens the router's base URL. `/` answers the
/// Overview's document and `/live`, with `?session=<id>` too, Sessions', in place, so a reload
/// lands back on the session it showed.
#[tokio::test]
async fn every_launcher_opens_root_or_live_and_both_answer_the_console() {
    assert!(source("menubar/src/main.rs").contains(".arg(format!(\"http://{HOST}:{}\", port()))"));
    let extension = source("vscode/src/extension.ts");
    assert!(extension.contains("dashboard.open('/', 'Agent Router')"));
    assert!(extension.contains("dashboard.open('/live', 'Live sessions')"));
    assert!(source("vscode/src/status.ts").contains("this.item.command = 'agentRouter.openDashboard';"));
    let iphone = source("ios/AgentRouter/Sources/Views/RouterSettingsView.swift");
    assert!(iphone.contains("if let url = state.router.baseURL") && iphone.contains("Link(destination: url)"));
    let lab = Lab::local("launchers", fixture());
    for path in ["/", "/live", "/live?session=7c2d0e1a-b3f4-4c5d-9e8f-0a1b2c3d4e5f"] {
        lab.ask(get(path)).await.assert_document(path);
    }
}

/// SV-C37: an address in the old dashboard's form, `/#a=…&p=…`, opens the console at `/`
/// unscoped. The fragment never reaches the router, which answers `/` with the document as at
/// any other time, and no part of the console reads `location.hash`, so nothing reads or
/// rewrites the old form.
#[tokio::test]
async fn an_old_dashboard_address_opens_the_console_unscoped() {
    let lab = Lab::local("old-address", fixture());
    lab.ask(get("/")).await.assert_document("/ (the fragment stays in the browser)");
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src")];
    let mut read = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                stack.push(path);
            } else if (name.ends_with(".ts") || name.ends_with(".tsx")) && !name.contains(".spec.") {
                read += 1;
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(!text.contains("location.hash"), "{} reads the address's fragment", path.display());
            }
        }
    }
    assert!(read > 20, "the console's sources were not read");
}

/// SV-C38: the console promises no address outside the app but its own. Only `/`, `/live` and
/// `/settings…` answer the document, so `/next/`, `/dashboard` and the like are the proxy's 404.
/// A push carries no address at all: its payload names the session, which the phones route by.
#[tokio::test]
async fn the_console_answers_at_its_own_addresses_and_a_push_names_none() {
    let lab = Lab::local("addresses", fixture());
    for path in ["/next/", "/next/live", "/dashboard", "/sessions", "/overview", "/index.html", "/live/0f3c", "/app"] {
        lab.ask(get(path)).await.assert_proxy_404(path);
    }
    let notify = source("src/notify.rs");
    assert!(notify.contains("\"session\": session,"), "a push names its session");
    for key in ["\"url\":", "\"link\":", "\"href\":"] {
        assert!(!notify.contains(key), "a push carries {key}");
    }
}

/// SV-C39: the dev server listens on loopback only, on a port it holds strictly, and forwards
/// /api to a scratch router on a port of its own, never the live router's 8787, which it refuses
/// outright. The fixture harness listens on loopback only as well.
#[test]
fn the_dev_server_stays_on_loopback_and_off_the_live_router() {
    let vite = source("ui/vite.config.ts");
    for must in [
        "host: '127.0.0.1'",
        "strictPort: true",
        "const LIVE_ROUTER_PORT = '8787';",
        "if (devPort === LIVE_ROUTER_PORT) {",
        "const target = `http://127.0.0.1:${devPort}`;",
        "'/api': proxied,",
    ] {
        assert!(vite.contains(must), "ui/vite.config.ts: {must}");
    }
    assert!(source("ui/vite.harness.config.ts").contains("host: '127.0.0.1'"));
}

/// SV-C43: a hosted router keeps the machine routes registered, answering in the shapes the
/// installed phones decode: every field of /api/setup and of /api/setup/shim. It refuses each
/// machine action with 409 and `{ok: false, error}`, a sentence the console shows as written.
#[tokio::test]
async fn a_hosted_router_answers_the_machine_routes_in_the_phones_shapes() {
    let lab = Lab::hosted("machine-routes", fixture()).await;
    let read = |path: &'static str| {
        let lab = &lab;
        async move { lab.ask(get(path).header(header::AUTHORIZATION, BASIC)).await.json() }
    };
    let setup = read("/api/setup").await;
    for field in ["service", "url", "global_settings_path", "managed", "mode"] {
        assert!(setup.get(field).is_some(), "/api/setup.{field}: {setup}");
    }
    for field in ["registered", "supported", "platform"] {
        assert!(setup["service"].get(field).is_some(), "/api/setup.service.{field}: {setup}");
    }
    let shim = read("/api/setup/shim").await;
    for field in ["available", "installed", "targets", "auto", "auto_log"] {
        assert!(shim.get(field).is_some(), "/api/setup/shim.{field}: {shim}");
    }
    for (path, body) in [("/api/setup/service", r#"{"enable":true}"#), ("/api/network", r#"{"enable":true}"#)] {
        let answer = lab.ask(post(path, body).header(header::AUTHORIZATION, BASIC)).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{path}");
        let refusal = answer.json();
        assert_eq!(refusal["ok"], false, "{path}");
        assert!(refusal["error"].as_str().is_some_and(|e| e.starts_with("this router is hosted")), "{path}: {refusal}");
    }
}

/// SV-C44: a hosted router with no machine agent connected lists no session, says it is hosted
/// (`/api/auth`'s mode), and reports no editor to wrap. That is what the console reads to say why
/// the list is empty, never "No sessions in the last week", and to warn of no unwrapped editor.
#[tokio::test]
async fn a_hosted_router_with_no_agent_lists_no_session_and_no_editor() {
    let lab = Lab::hosted("no-agent", fixture()).await;
    let read = |path: &'static str| {
        let lab = &lab;
        async move { lab.ask(get(path).header(header::AUTHORIZATION, BASIC)).await.json() }
    };
    assert_eq!(read("/api/live/sessions").await, Value::Array(Vec::new()));
    assert_eq!(read("/api/auth").await["mode"], "server");
    let shim = read("/api/setup/shim").await;
    assert_eq!((shim["available"].clone(), shim["installed"].clone()), (Value::Bool(false), Value::from(0)));
}

/// SV-C9: the cut is clean. `/` and `/live` serve the console. The two pages that were compiled
/// in with `include_str!` are gone from the tree and from the code that served them. There was
/// no `/next/` period, so nothing answers there.
#[tokio::test]
async fn the_cut_is_clean() {
    let lab = Lab::local("cut", fixture());
    lab.ask(get("/")).await.assert_document("/");
    lab.ask(get("/live")).await.assert_document("/live");
    for path in ["/next/", "/next/live", "/next/settings/network"] {
        lab.ask(get(path)).await.assert_proxy_404(path);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for gone in ["assets/index.html", "assets/live.html", "scripts/check-dashboard-ui.js", "scripts/check-live-ui.js"] {
        assert!(!root.join(gone).exists(), "{gone} is still here");
    }
    for file in ["src/api.rs", "src/live.rs", "src/main.rs"] {
        assert!(!source(file).contains("include_str!(\"../assets/"), "{file} still compiles an old page in");
    }
}
