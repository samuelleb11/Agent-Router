//! The web console: the React and IBM Carbon app in `ui/`, built into `ui/dist` and compiled
//! into the binary. It answers at `/` (the Overview), `/live` (Sessions), `/machines` and
//! `/settings/…`, behind the same guard as `/api` and `/v1`: `main::app` adds these routes
//! before its one guard layer, in the local and the hosted route sets alike. The design is "Serving and
//! security" in docs/web-console-carbon.md; the rules it keeps are the SV-C contracts in
//! docs/web-console-contracts.md, each named by the test in `tests` that holds it.
//!
//! Every path is named, and none is a fallback or a wildcard at the root. The router's
//! fallback is the proxy, which is how Claude Code reaches `/v1/*`: a wildcard here would catch
//! that and break every agent, and a fallback would answer a mistyped API path with a web page.
//! So the document answers at the app's own addresses, hashed files under `/assets/`, and each
//! file the build puts at its root at its own address. Anything else is the proxy's plain 404.
//!
//! - **The document** is `no-store`. It ships inside the binary and changes with every build,
//!   so a cached copy is a stale copy, and a phone holding one looks exactly like "it's not
//!   working". It carries the content security policy, `POLICY`.
//! - **Hashed files** never change under their names, so they are `immutable`. One this binary
//!   does not hold is a plain 404 and never the document: a tab left open across a deploy has
//!   to see its missing chunk fail, which is what makes the app reload (`vite:preloadError`,
//!   in ui/src/main.tsx) rather than parse a web page as a script.
//! - **Files at the build's root** keep their names from build to build (`/theme-boot.js`, the
//!   fonts, the notices), so a browser revalidates them, by an ETag, rather than keeping them.

use std::borrow::Cow;

use axum::extract::Path;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

#[allow(dead_code)] // build.rs and the tests use the rest
mod policy;
#[cfg(test)]
mod tests;

/// `ui/dist` as it was when this binary was compiled: in every build, debug included
/// (`debug-embed`), so nothing is read from disk at run time and a test serves what a release
/// does. A tree that has not built the console compiles to an empty set (`allow_missing`), and
/// `policy` decides what that build may do.
#[derive(RustEmbed)]
#[folder = "ui/dist"]
#[allow_missing = true]
struct Dist;

/// The document's content security policy (decision D in docs/web-console-carbon.md).
///
/// - Everything is `'self'`. The fonts are self-hosted and the rest is embedded, so the console
///   loads nothing from any other origin, and an injected script or stylesheet has nowhere to
///   come from. No inline script ships: the theme's pre-paint snippet is `/theme-boot.js`.
/// - `style-src 'self'` and `script-src 'self'` hold with no exception. React and Carbon set
///   styles through the CSSOM, which the policy does not govern. Served under this policy in
///   headless Chrome on 2026-10-05, the first build inserted no `<style>` element, had no style
///   attribute refused and ran no inline script on any screen, the Overview's chart drawn,
///   hovered and re-themed included; lazy stylesheets arrive as `<link>`s from this origin.
/// - `img-src data:` is decision D's: Carbon Charts draws its image export through data URLs.
/// - `object-src 'none'`: no plugins. `base-uri 'none'`: no `<base>` can move where the
///   document's paths point. The one dependency that sets one is that same export, which the
///   console's chart does not offer (its toolbar is off), and refused, it changes nothing in
///   the image. `form-action 'none'`: every form is handled in script, so none ever navigates.
/// - `frame-ancestors`: only VS Code's own schemes may frame the console — its Simple Browser
///   and the extension's fallback frame. It approves prompts and can switch a session to
///   bypass permissions, and a web page could otherwise frame `http://127.0.0.1:8787/live`,
///   since the guard's Host and peer checks both pass for it. `X-Frame-Options` is not sent: it
///   cannot express an allow-list, and its `DENY` refuses those very frames.
pub const POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; \
    img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
    frame-ancestors vscode-webview: vscode-file:";

/// The addresses the app shows, each answered with the document so a reload lands where it
/// was. The client's route table (ui/src/app, ui/src/lib/routes.ts) is held to these by a test.
const DOCUMENT: [&str; 5] = ["/", "/live", "/machines", "/settings", "/settings/{*section}"];

/// Top-level names the router answers itself. A file the build puts at its root under one of
/// them is not routed, rather than shadowing the router: `v1` is Claude Code's, `api` and
/// `_router` the API's, `hooks` the forges' (`library::hooks`), and the rest are the
/// console's own addresses.
const RESERVED: [&str; 10] =
    ["api", "v1", "_router", "hooks", "live", "machines", "settings", "assets", "index.html", "favicon.svg"];

const HTML: &str = "text/html; charset=utf-8";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The console's files: what this binary embeds, or in a test a build of the test's own.
#[derive(Clone)]
pub struct Files(Source);

#[derive(Clone)]
enum Source {
    Embedded,
    #[cfg(test)]
    Fixture(std::sync::Arc<std::collections::BTreeMap<String, Vec<u8>>>),
}

/// One file of the build, and a digest of it to revalidate by.
struct File {
    data: Cow<'static, [u8]>,
    sha256: [u8; 32],
}

impl Files {
    /// The build compiled into this binary.
    pub fn embedded() -> Self {
        Files(Source::Embedded)
    }

    /// A build of a test's own, as `vite build` would have written it: name and contents.
    #[cfg(test)]
    pub fn fixture<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let files = files.into_iter().map(|(name, body)| (name.to_string(), body.as_bytes().to_vec()));
        Files(Source::Fixture(std::sync::Arc::new(files.collect())))
    }

    fn get(&self, name: &str) -> Option<File> {
        match &self.0 {
            Source::Embedded => Dist::get(name).map(|f| File { sha256: f.metadata.sha256_hash(), data: f.data }),
            #[cfg(test)]
            Source::Fixture(files) => files.get(name).map(|bytes| {
                use sha2::Digest;
                File { sha256: sha2::Sha256::digest(bytes).into(), data: Cow::Owned(bytes.clone()) }
            }),
        }
    }

    fn names(&self) -> Vec<String> {
        match &self.0 {
            Source::Embedded => Dist::iter().map(Cow::into_owned).collect(),
            #[cfg(test)]
            Source::Fixture(files) => files.keys().cloned().collect(),
        }
    }
}

/// What the build puts at its root, other than the document and `assets/`: a file, served at
/// its own name, or a directory, whose files are served under it (`/fonts/…`).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Root {
    File(String),
    Dir(String),
}

/// The root entries of `names`, minus the router's own names and any name that is not plainly
/// a file's: a route is built from each, and a brace or a wildcard in one would be read as a
/// pattern, which axum refuses with a panic as the router starts.
fn roots(names: &[String]) -> Vec<Root> {
    let plain = |s: &str| {
        !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    let mut roots: Vec<Root> = names
        .iter()
        .map(|name| match name.split_once('/') {
            Some((dir, _)) => Root::Dir(dir.to_string()),
            None => Root::File(name.clone()),
        })
        .filter(|root| {
            let (Root::File(name) | Root::Dir(name)) = root;
            plain(name) && !RESERVED.contains(&name.as_str())
        })
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// The console's routes, added to `router`. `main::app` adds them before its guard layer, so
/// the guard wraps the document and every file exactly as it wraps `/api` and `/v1`.
pub fn routes<S>(mut router: Router<S>, files: Files) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    for path in DOCUMENT {
        let files = files.clone();
        router = router.route(path, get(move || document(files.clone())));
    }
    let hashed = files.clone();
    router = router
        .route("/favicon.svg", get(favicon))
        .route("/assets/{*path}", get(move |Path(path): Path<String>| asset(hashed.clone(), path)));
    for root in roots(&files.names()) {
        let files = files.clone();
        router = match root {
            Root::File(name) => router.route(
                &format!("/{name}"),
                get(move |asked: HeaderMap| root_file(files.clone(), name.clone(), asked)),
            ),
            Root::Dir(dir) => router.route(
                &format!("/{dir}/{{*path}}"),
                get(move |Path(path): Path<String>, asked: HeaderMap| {
                    root_file(files.clone(), format!("{dir}/{path}"), asked)
                }),
            ),
        };
    }
    router
}

async fn document(files: Files) -> Response {
    let Some(file) = files.get("index.html") else {
        return placeholder();
    };
    (
        [
            (header::CONTENT_TYPE, HTML),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, POLICY),
        ],
        file.data,
    )
        .into_response()
}

/// What stands in for the document in a build without the console (see `policy`): a page that
/// says so and names the command, rather than a blank one that reads as a broken router. 503,
/// because a console is not being served; the API and the proxy are, as ever.
fn placeholder() -> Response {
    let page = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Agent Router</title></head>\n\
         <body>\n<h1>The console is not built</h1>\n\
         <p>This router was built without its web console. Build the console, then the router:</p>\n\
         <pre>{}</pre>\n<p>Everything else the router serves works as usual.</p>\n</body>\n</html>\n",
        policy::BUILD_COMMAND
    );
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [
            (header::CONTENT_TYPE, HTML),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, POLICY),
        ],
        page,
    )
        .into_response()
}

async fn asset(files: Files, path: String) -> Response {
    let name = format!("assets/{path}");
    let file = if plain_path(&path) { files.get(&name) } else { None };
    match file {
        Some(file) => ([(header::CONTENT_TYPE, kind(&name)), (header::CACHE_CONTROL, IMMUTABLE)], file.data).into_response(),
        None => missing(),
    }
}

/// A file at the build's root, or under one of its directories: revalidated every time, and
/// answered with no body when the browser already holds it.
async fn root_file(files: Files, name: String, asked: HeaderMap) -> Response {
    let file = if plain_path(&name) { files.get(&name) } else { None };
    let Some(file) = file else {
        return missing();
    };
    let tag = file.sha256[..16].iter().map(|b| format!("{b:02x}")).collect::<String>();
    let etag = format!("\"{tag}\"");
    let held = asked
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').map(|t| t.trim().trim_start_matches("W/")).any(|t| t == etag || t == "*"));
    if held {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag), (header::CACHE_CONTROL, "no-cache".into())])
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, kind(&name).to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::ETAG, etag),
        ],
        file.data,
    )
        .into_response()
}

/// Whether a path a URL asked for could name a file the build wrote: no empty segment, no `.`
/// or `..`, no backslash. Nothing outside the build is reachable either way, since lookups are
/// by exact name; this keeps a traversal from even being looked up.
fn plain_path(path: &str) -> bool {
    !path.contains('\\') && path.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// A file this binary does not hold: a plain 404, never the document.
fn missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "agent-router: no such file\n",
    )
        .into_response()
}

/// The type a file is served as, by its extension. Under `nosniff` a browser runs a script or
/// applies a stylesheet only when it is served as one, so these are exact.
fn kind(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html") => HTML,
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json" | "map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// The router's own icon, which the document links. Inline, so the binary stays
/// self-contained and a build without the console still has it.
async fn favicon() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="7" fill="#4f46e5"/><path d="M8 21V11h3.2l4.8 6.4V11h3.2v10h-3.2l-4.8-6.4V21z" fill="#fff"/></svg>"##,
    )
        .into_response()
}

/// `X-Content-Type-Options: nosniff` on every response the router gives: the console's, the
/// API's, the proxy's, and the guard's refusals, which is why `main::app` layers it outside the
/// guard. Under it a browser takes nothing the router answers for something it is not served as.
pub async fn nosniff(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}
