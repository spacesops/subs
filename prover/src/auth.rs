//! HTTP Basic authentication middleware for the subs-prover server.
//!
//! Authentication is only enforced when `ServerState::basic_auth` is set (i.e. both
//! `SUBS_PROVER_BASIC_AUTH_USER` and `SUBS_PROVER_BASIC_AUTH_PASSWORD` are provided).
//! The health-check endpoint and CORS preflight requests are always allowed through
//! anonymously.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::Engine;

use crate::server::ServerState;

/// Split a request path into non-empty segments (leading/trailing slashes ignored).
fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Whether a request may bypass authentication.
///
/// Only the liveness probe (`GET /health`) is anonymous; every other prover
/// endpoint requires credentials when auth is enabled.
fn is_anonymous(method: &Method, path: &str) -> bool {
    matches!(path_segments(path).as_slice(), ["health"] if *method == Method::GET)
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
            r#"Basic realm="subs-prover", charset="UTF-8""#,
        )
        .body(Body::from("Unauthorized"))
        .expect("static unauthorized response is valid")
}

/// Axum middleware enforcing HTTP Basic auth across the prover server.
pub async fn require_basic_auth(
    State(state): State<Arc<ServerState>>,
    req: Request,
    next: Next,
) -> Response {
    // Auth disabled unless credentials are configured.
    let Some((user, pass)) = state.basic_auth().as_ref() else {
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
