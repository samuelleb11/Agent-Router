//! What a build does when the console has not been built: `ui/dist` is what `console` embeds,
//! and nothing in a cargo build makes it. Decision B in docs/web-console-carbon.md: the
//! console is built by `scripts/build-target.sh` and by CI's `build:ui` job, never by
//! `build.rs`, which would put Node on every cargo call; and it is never committed, because
//! its hashed files churn history and collide between sessions working in one checkout.
//!
//! So a build can meet a tree with no `ui/dist`. A debug build serves a placeholder that
//! names the command, so a change to the Rust alone never needs Node, and a blank page never
//! reads as a broken router. A release build stops: shipped, the placeholder is a router
//! with no console. `AGENT_ROUTER_ALLOW_NO_UI=1` lets one through on purpose.
//!
//! `build.rs` includes this file by path and prints what it decides; the crate includes it as
//! `console::policy`, so its tests hold the decision to its contracts and the placeholder names
//! the same command the build's message does. Outside its tests the crate uses only the command,
//! which is why it declares the module with `allow(dead_code)`.

use std::path::Path;

/// The command that builds the console, from the repository's root.
pub const BUILD_COMMAND: &str = "cd ui && pnpm install && pnpm run build";

/// Set to `1`, it lets a release be built without the console, serving the placeholder.
pub const ALLOW_NO_UI: &str = "AGENT_ROUTER_ALLOW_NO_UI";

/// What a build does about the console.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// `ui/dist` is there, and is what the binary embeds.
    Embed,
    /// It is not, and this build serves the placeholder in its place, saying so as a warning.
    Placeholder(String),
    /// It is not, and this build must not finish without it.
    Refuse(String),
}

/// `built` is whether `ui/dist` holds a document; `profile` is cargo's `PROFILE`, `release`
/// or `debug`; `allow` is `AGENT_ROUTER_ALLOW_NO_UI`, when set.
pub fn verdict(built: bool, profile: &str, allow: Option<&str>) -> Verdict {
    if built {
        return Verdict::Embed;
    }
    if profile != "release" {
        return Verdict::Placeholder(format!(
            "the console is not built (no ui/dist), so this router serves a placeholder at / until it is: {BUILD_COMMAND}"
        ));
    }
    if allow == Some("1") {
        return Verdict::Placeholder(format!(
            "{ALLOW_NO_UI}=1: this release has no console and serves a placeholder at / in its place"
        ));
    }
    Verdict::Refuse(format!(
        "the console is not built (no ui/dist), and a release must not ship without it. Build it \
         first: {BUILD_COMMAND}. To build a release that serves a placeholder instead, set {ALLOW_NO_UI}=1."
    ))
}

/// What `build.rs` prints for the console, under `root` (the package's directory).
///
/// It watches `ui/dist` as a directory, which cargo scans whole: a rebuilt console has new
/// hashed names, and only a rerun of the build script recompiles the crate that embeds it.
/// While `ui/dist` is missing, cargo reruns the script on every build, which keeps the
/// warning in front of whoever is building without the console.
pub fn directives(root: &Path, verdict: &Verdict) -> Vec<String> {
    let mut lines = vec![
        format!("cargo:rerun-if-changed={}", root.join("ui").join("dist").display()),
        format!("cargo:rerun-if-env-changed={ALLOW_NO_UI}"),
    ];
    match verdict {
        Verdict::Embed => {}
        Verdict::Placeholder(why) => lines.push(format!("cargo:warning={why}")),
        // Fails the build once the script has finished, with the sentence and nothing else:
        // no "failed to run custom build command" around it.
        Verdict::Refuse(why) => lines.push(format!("cargo::error={why}")),
    }
    lines
}
