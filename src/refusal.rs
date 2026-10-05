//! How the router answers a change it will not make: an error status, and the reason in
//! words where every client already looks for them. Never a 200 that carries an error.
//!
//! A 200 says the change was made, and every client reads it that way. The dashboard's
//! `post()` throws only on an error status, so a refusal answered as a 200 read as success:
//! a password too short to keep read "set", a push key that does not sign read "saved", a
//! setup switch that could not be thrown read "done". The phones showed one only where a
//! screen happened to look inside the 200 for `error`, and Android's setup switches never did.
//!
//! The words stay where they were, under `error`, beside the `ok: false` a hosted router's
//! refusals (`hosted::refuse`) have always carried — so a client reads one shape from either
//! kind of router. The wrapper switches refuse with a `reason`, as they always have; see
//! [`refuse_with_reason`].
//!
//! The status says whose move it is:
//!
//! * **400** — the request is wrong whatever the router's state: something it needs is
//!   missing, or is not what it says it is. Sending something else is the fix.
//! * **403** — this caller may not do it. `guard` says so before any handler runs, so none
//!   of the refusals here needs it.
//! * **409** — the request is fine, but the router's state stands in the way: no password
//!   yet, a sign-in already running, a password the deployment pins, a settings file that
//!   is not JSON. Changing that state is the fix; sending the same request again is not.
//! * **422** — complete and well-formed, but what it carries does not work: a key that does
//!   not sign. Also what axum answers for a body of the wrong shape, kept as it is.
//! * **500** — the router tried and failed, and nothing the caller sent was wrong: the
//!   store, the secrets file or launchctl would not take it. Not a refusal, and not a 200
//!   either, for the same reason.
//!
//! `GET /api/quota` is left as it was: a failed read there is still a 200 with `error`,
//! because the menu-bar app and the VS Code status bar read it that way.

use axum::extract::{FromRequest, OptionalFromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::json;

/// A refusal: the status that says whose move it is, and the reason in words.
pub fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, Json(json!({ "ok": false, "error": why.into() }))).into_response()
}

/// The wrapper switches' refusal: `{ok: false, reason}` as they have always sent it, and
/// the same words under `error` beside it. Every current client reads either key, but a
/// dashboard tab opened before the router was upgraded keeps the `post()` it loaded with,
/// which looks only at `error` once the status says no — without it, that tab would show
/// the bare status text where the reason belongs.
pub fn refuse_with_reason(status: StatusCode, why: impl Into<String>) -> Response {
    let why = why.into();
    (status, Json(json!({ "ok": false, "reason": why, "error": why }))).into_response()
}

/// The router failed at something it should have managed. A 500, in the same shape, so a
/// client that shows a refusal's words shows these too.
pub fn failed(why: impl std::fmt::Display) -> Response {
    refuse(StatusCode::INTERNAL_SERVER_ERROR, why.to_string())
}

/// A JSON request body, or a refusal in the shape above.
///
/// axum's own `Json` refuses a body it cannot read — not JSON, the wrong shape, no JSON
/// content type — with the right status but a plain-text sentence that no client reads:
/// the dashboard showed the status text instead, and the phones the raw body. Every handler
/// that refuses in this module's shape takes its body through this, so that refusal carries
/// its words where the others do. The statuses are axum's, unchanged: 400 for a body that
/// is not JSON, 422 for JSON of the wrong shape, 415 without a JSON content type, 413 for a
/// body over the limit.
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match <Json<T> as FromRequest<S>>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(refuse(rejection.status(), rejection.body_text())),
        }
    }
}

/// A body that may be left out, by axum's rule: no content type at all is no body, and a
/// JSON one is read and refused like any other when it cannot be.
impl<T, S> OptionalFromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        match <Json<T> as OptionalFromRequest<S>>::from_request(req, state).await {
            Ok(body) => Ok(body.map(|Json(value)| Self(value))),
            Err(rejection) => Err(refuse(rejection.status(), rejection.body_text())),
        }
    }
}
