//! HTTP Basic authentication middleware for the subsd UI and API.
//!
//! Authentication is only enforced when `AppState::basic_auth` is set (i.e. both
//! `SUBS_BASIC_AUTH_USER` and `SUBS_BASIC_AUTH_PASSWORD` are provided). Health-check
//! endpoints and CORS preflight requests are always allowed through anonymously.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::Engine;

use crate::state::AppState;

/// Split a request path into non-empty segments (leading/trailing slashes ignored).
fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Whether a request may bypass authentication.
///
/// This covers the liveness probe plus a set of public endpoints used by the
/// handle reservation/claim flow. Matching is method-aware and understands the
/// parameterized routes (`/certs/:handle`, `/spaces/:space/handles/:handle`).
fn is_anonymous(method: &Method, path: &str) -> bool {
    let get = *method == Method::GET;
    let post = *method == Method::POST;

    match path_segments(path).as_slice() {
        // Liveness probe.
        ["health"] => get,
        // Read-only status used by the public UI.
        ["status"] => get,
        // Public handle submission / reservation / claim flow.
        ["requests"] => post,
        ["reserve"] => post,
        ["claim"] => post,
        // Per-handle certificate lookup: GET /certs/{handle}
        ["certs", _handle] => get,
        // Per-handle status lookup: GET /spaces/{space}/handles/{subname}
        ["spaces", _space, "handles", _subname] => get,
        _ => false,
    }
}

/// Constant-time byte comparison to avoid leaking credential length/content via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn credentials_match(req: &Request, user: &str, pass: &str) -> bool {
    let Some(value) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    let Some(encoded) = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))
    else {
        return false;
    };

    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };

    let expected = format!("{user}:{pass}");
    constant_time_eq(&decoded, expected.as_bytes())
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            header::WWW_AUTHENTICATE,
            r#"Basic realm="subs", charset="UTF-8""#,
        )
        .body(Body::from("Unauthorized"))
        .expect("static unauthorized response is valid")
}

/// Axum middleware enforcing HTTP Basic auth across the UI and API.
pub async fn require_basic_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Auth disabled unless credentials are configured.
    let Some((user, pass)) = state.basic_auth.as_ref() else {
        return next.run(req).await;
    };

    // Always allow CORS preflight and the public/anonymous endpoints through.
    if req.method() == Method::OPTIONS || is_anonymous(req.method(), req.uri().path()) {
        return next.run(req).await;
    }

    if credentials_match(&req, user, pass) {
        next.run(req).await
    } else {
        unauthorized()
    }
}
