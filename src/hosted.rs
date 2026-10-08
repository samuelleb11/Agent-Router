//! What a hosted router answers in place of the routes that act on the machine it runs
//! on. See `docs/hosting-on-kubernetes.md`.
//!
//! A router on a developer's machine can set up that machine's Claude Code, register a
//! login item, wrap its editor's extension, widen its own socket, run `claude` to sign
//! in, and read the transcripts, registry and wrapper sockets of the sessions running
//! there. A hosted router is on none of those machines. Its routes stay registered and
//! answer in the shapes the installed phone apps decode — the iPhone app requires every
//! field of `/api/setup` and `/api/setup/shim`, so an error there would be a decode
//! failure on the phone — and every action refuses with a sentence that says why.
//!
//! The live routes are not here: `machines` relays them to the agent on the machine
//! holding each session. What a hosted router has that a local one does not is here too:
//! the agent's own endpoint, and the installer that sets an agent up (`downloads`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::proxy::AppState;
use crate::refusal::JsonBody;

const NO_MACHINE: &str = "this router is hosted: it does not run on a developer's machine, so there is no \
    Claude Code, login item, editor extension or network switch here to manage — set those up on each machine";
const NO_SIGN_IN: &str = "signing in runs Claude Code's own login on the router's machine, which a hosted \
    router does not have — run `claude setup-token` on your machine and paste the token";
const NO_PROFILE: &str = "a profile account is a Claude Code login directory on the router's machine, which \
    a hosted router does not have — run `claude setup-token` on your machine and paste the token";

/// Every path `main::machine_routes` registers, answered for a router with no machine.
pub fn routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/accounts", get(crate::api::accounts_list).post(accounts_add))
        .route("/api/accounts/probe", post(no_profile))
        .route("/api/push/pending", get(crate::machines::api::pending))
        .route("/api/login/start", post(no_sign_in))
        .route("/api/login/status", get(no_login))
        .route("/api/setup", get(setup_status))
        .route("/api/setup/service", post(no_machine))
        .route("/api/setup/config", post(no_machine))
        .route("/api/setup/shim", get(shim_status).post(no_machine))
        .route("/api/setup/shim/auto", post(no_machine))
        .route("/api/network", post(no_machine))
        // The live view is on the developer machines: each route is relayed to the
        // agent of the machine holding the session (see `machines`).
        .route("/api/live/sessions", get(crate::machines::api::sessions))
        .route("/api/live/stream", get(crate::machines::api::by_query))
        .route("/api/live/stats", get(crate::machines::api::by_query))
        .route("/api/live/prompt", get(crate::machines::api::by_query))
        .route("/api/live/project", get(crate::machines::api::project))
        .route("/api/live/send", post(crate::machines::api::by_body))
        .route("/api/live/interrupt", post(crate::machines::api::by_body))
        .route("/api/live/answer", post(crate::machines::api::by_body))
        .route("/api/live/model", post(crate::machines::api::by_body))
        .route("/api/live/effort", post(crate::machines::api::by_body))
        .route("/api/live/mode", post(crate::machines::api::by_body))
        .route("/api/live/aside", post(crate::machines::api::by_body))
        .route("/api/live/archive", post(crate::machines::api::archive))
        .route("/api/machines", get(crate::machines::api::machines))
        .route(crate::tunnel::PATH, get(crate::machines::api::connect))
        // The Library: what the console reads and writes, and what every machine's agent
        // syncs from (`library::api`, docs/library.md).
        .route("/api/library", get(crate::library::api::list))
        .route("/api/library/manifest", get(crate::library::api::manifest))
        .route("/api/library/blobs/{digest}", get(crate::library::api::blob))
        .route("/api/library/sync", post(crate::library::push::sync_now))
        .route("/api/library/report", post(crate::library::push::report))
        .route("/api/library/forges", get(crate::library::sources::list_forges))
        .route(
            "/api/library/forges/{key}",
            put(crate::library::sources::put_forge_api).delete(crate::library::sources::delete_forge_api),
        )
        .route("/api/library/forges/{key}/check", post(crate::library::sources::check_forge))
        .route("/api/library/sources", get(crate::library::sources::list_sources))
        .route(
            "/api/library/sources/{key}",
            put(crate::library::sources::put_source_api).delete(crate::library::sources::delete_source_api),
        )
        .route("/api/library/sources/{key}/import", post(crate::library::sources::import_now))
        .route("/api/library/items/{kind}/{name}", get(crate::library::api::item))
        .route("/api/library/items/{kind}/{name}/archive", post(crate::library::api::archive))
        .route("/api/library/items/{kind}/{name}/versions", post(crate::library::api::publish))
        .route("/api/library/items/{kind}/{name}/versions/{number}/live", post(crate::library::api::make_live))
        .route("/api/library/items/{kind}/{name}/versions/{number}/reject", post(crate::library::api::reject))
        .route("/api/library/items/{kind}/{name}/versions/{number}/files/{*path}", get(crate::library::api::file))
        // The agent's installer, open without the password (see `downloads`).
        .route("/agent/install.sh", get(crate::downloads::install_sh))
        .route("/agent/install.ps1", get(crate::downloads::install_ps1))
        .route("/agent/{platform}/{file}", get(crate::downloads::file))
}

/// The refusal for anything that would act on the router's own machine. A sentence the
/// dashboard and the phones show as it is, under the `error` key they already read.
pub fn refuse(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": why }))).into_response()
}

pub fn refuse_sign_in() -> Response {
    refuse(StatusCode::CONFLICT, NO_SIGN_IN)
}

pub fn refuse_profile() -> Response {
    refuse(StatusCode::CONFLICT, NO_PROFILE)
}

/// `POST /api/accounts`: a pasted `claude setup-token` value, and nothing else. The
/// other sources are a Claude Code login on the router's own machine. A body that cannot
/// be read is refused as every other is (`refusal::JsonBody`).
async fn accounts_add(state: State<Arc<AppState>>, JsonBody(req): JsonBody<crate::api::AddAccount>) -> Response {
    if req.source != "token" {
        return refuse_profile();
    }
    crate::api::accounts_add(state, JsonBody(req)).await.into_response()
}

async fn no_machine() -> Response {
    refuse(StatusCode::CONFLICT, NO_MACHINE)
}

async fn no_sign_in() -> Response {
    refuse_sign_in()
}

async fn no_profile() -> Response {
    refuse_profile()
}

/// `GET /api/setup`: nothing managed, nothing registered, and nothing created on disk to
/// find that out. `supported: false` is what hides "Start at login" on both phones.
async fn setup_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "service": { "registered": false, "supported": false, "platform": "server" },
        "url": state.config.public_url.clone().unwrap_or_default(),
        "global_settings_path": "",
        "managed": [],
        "mode": "server",
    }))
}

/// `GET /api/setup/shim`: no editor here, so nothing to wrap — and no "not wrapped"
/// warning, which is only true of an editor that exists.
async fn shim_status() -> Json<Value> {
    Json(json!({ "available": false, "installed": 0, "targets": [], "auto": false, "auto_log": [] }))
}

/// `GET /api/login/status`: no sign-in is ever running.
async fn no_login() -> Json<Value> {
    Json(json!({}))
}

