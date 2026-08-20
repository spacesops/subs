//! Certificate endpoints.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    Json,
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde::Serialize;

use crate::state::AppState;
use super::json_error;

#[derive(Serialize)]
pub struct IssueCertResponse {
    /// Base64-encoded root certificate
    pub root_cert: String,
    /// Base64-encoded handle certificate (null for space-only certs)
    pub handle_cert: Option<String>,
}

/// GET /certs/:handle - Issue certificate for handle
///
/// Handle can be:
/// - `@space` - issues root certificate only
/// - `alice@space` - issues root + handle certificate
pub async fn issue_cert(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<IssueCertResponse>, Response> {
    use spaces_protocol::sname::SName;

    let handle: SName = handle
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid handle: {}", e)))?;

    let (root_cert, handle_cert) = state
        .operator
        .issue_cert(&handle)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let root_bytes = borsh::to_vec(&root_cert)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to serialize root cert: {}", e)))?;

    let handle_bytes = match handle_cert {
        Some(cert) => Some(
            borsh::to_vec(&cert)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to serialize handle cert: {}", e)))?
        ),
        None => None,
    };

    Ok(Json(IssueCertResponse {
        root_cert: base64::engine::general_purpose::STANDARD.encode(&root_bytes),
        handle_cert: handle_bytes.map(|b| base64::engine::general_purpose::STANDARD.encode(&b)),
    }))
}

/// GET /certs/:handle/message - The exact bytes publishing this handle would
/// broadcast.
///
/// A debugging aid: this is what `submit_certs` builds and hands to the relay,
/// so it can be inspected or replayed without going through a publish.
///
/// Nothing is recorded. Publishing tracks message sizes to size the next batch,
/// and marks handles published — an export that did either would perturb the
/// state it exists to inspect.
///
/// The bytes are only meaningful against the chain tip they were built at:
/// build_message fetches a fresh chain proof, so this is a snapshot rather than
/// a durable artifact. It needs an RPC but no fabric, since nothing is sent.
pub async fn export_message(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Response, Response> {
    use spaces_protocol::sname::SName;

    let handle: SName = handle
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid handle: {}", e)))?;

    let filename = format!("{}.message.bin", handle.to_string().replace(['@', '/'], "_"));

    let certs = state
        .operator
        .issue_certs(vec![handle])
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let message = state
        .operator
        .build_message(certs)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let bytes = message.to_bytes();

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        bytes,
    )
        .into_response())
}

