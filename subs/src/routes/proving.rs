//! Proving endpoints.
//!
//! Binary format for efficiency:
//! - GET /proving/next returns borsh-serialized Option<ProvingRequest>
//! - POST /proving/fulfill accepts: commitment_id (8 bytes) + request_type (1 byte) + receipt

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use subs_core::CompressInput;

use crate::state::AppState;

/// One figure the prover wants displayed, already formatted by it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stat {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub accent: bool,
}

/// Live proving progress, forwarded verbatim from the prover.
///
/// The prover decides what is displayed — heading, bar, figures and their
/// order. subs relays; it computes and formats nothing, because only the prover
/// knows what its phases are or what a value means.
///
/// **Every field is optional**, deliberately. A prover that is booting a GPU
/// has no segment count and no cycles, and requiring them would leave it
/// choosing between sending zeros it would have to invent and saying nothing at
/// all. An unparseable progress body is dropped entirely, so a required field
/// is not a small cost.
/// Absent fields are skipped on the way out too, not re-emitted as nulls:
/// subs forwards this into the pipeline response, and a prover reporting only
/// "booting" should not turn into a wall of nulls for whoever reads that API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobProgress {
    /// What is happening now, in the prover's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Position in a prover-defined sequence, for an "N of M" indicator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_total: Option<u8>,
    /// 0.0–1.0, meaningful when the bar is determinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// "determinate" | "indeterminate" | "none". Absent means determinate when
    /// `fraction` is set, indeterminate otherwise — so ordinary cases omit it.
    /// "none" draws no bar, for a status that is not progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar: Option<String>,
    /// Figures to display, in the order given.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<Stat>,
    /// Lines to surface — pod boot output, queue notices. Replaced wholesale
    /// each poll; the prover owns retention and formatting.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<String>,
}
use super::json_error;

/// Request type for fulfill payload
const REQUEST_TYPE_STEP: u8 = 0;
const REQUEST_TYPE_FOLD: u8 = 1;

/// GET /spaces/:space/proving/next - Get next proving request (binary borsh format)
pub async fn get_next(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Response, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let request = state
        .operator
        .get_next_proving_request(&space)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Serialize as borsh
    let bytes = borsh::to_vec(&request)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {}", e)))?;

    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    ).into_response())
}

#[derive(Serialize)]
pub struct FulfillResponse {
    pub success: bool,
    pub message: Option<String>,
}

/// POST /spaces/:space/proving/fulfill - Submit proof receipt (binary format)
///
/// Binary payload format:
/// - 8 bytes: commitment_id (i64 little-endian)
/// - 1 byte: request_type (0 = Step, 1 = Fold)
/// - remaining: receipt bytes
pub async fn fulfill(
    State(state): State<AppState>,
    Path(space): Path<String>,
    body: Bytes,
) -> Result<Json<FulfillResponse>, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    // Parse binary payload
    if body.len() < 9 {
        return Err(json_error(StatusCode::BAD_REQUEST, "payload too short: need commitment_id (8) + type (1) + receipt"));
    }

    let commitment_id = i64::from_le_bytes(body[0..8].try_into().unwrap());
    let request_type = body[8];
    let receipt_bytes = &body[9..];

    if receipt_bytes.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "empty receipt"));
    }

    let is_fold = match request_type {
        REQUEST_TYPE_STEP => false,
        REQUEST_TYPE_FOLD => true,
        _ => return Err(json_error(StatusCode::BAD_REQUEST, format!("invalid request_type: {}", request_type))),
    };

    state
        .operator
        .fulfill_request_by_id(&space, commitment_id, is_fold, receipt_bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(FulfillResponse { success: true, message: None }))
}

#[derive(Serialize)]
pub struct CompressInputResponse {
    pub input: Option<CompressInputJson>,
}

#[derive(Serialize)]
pub struct CompressInputJson {
    /// Base64-encoded receipt
    pub receipt: String,
    pub commitment: subs_types::Commitment,
}

/// GET /spaces/:space/compress - Get SNARK compression input
pub async fn get_compress_input(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<CompressInputResponse>, Response> {
    use base64::Engine;

    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let input = state
        .operator
        .get_compress_input(&space)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = match input {
        Some(CompressInput { receipt, commitment }) => CompressInputResponse {
            input: Some(CompressInputJson {
                receipt: base64::engine::general_purpose::STANDARD.encode(&receipt),
                commitment,
            }),
        },
        None => CompressInputResponse { input: None },
    };

    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct SaveSnarkBody {
    /// Base64-encoded SNARK receipt
    pub receipt: String,
}

#[derive(Serialize)]
pub struct SaveSnarkResponse {
    pub success: bool,
}

/// POST /spaces/:space/snark - Save compressed SNARK
pub async fn save_snark(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Json(body): Json<SaveSnarkBody>,
) -> Result<Json<SaveSnarkResponse>, Response> {
    use base64::Engine;

    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let receipt_bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.receipt)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid base64: {}", e)))?;

    state
        .operator
        .save_snark(&space, &receipt_bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(SaveSnarkResponse { success: true }))
}

// --- Push-based proving with external prover server ---

#[derive(Serialize)]
pub struct PushResponse {
    pub success: bool,
    pub job_id: Option<String>,
    pub message: Option<String>,
}

/// POST /spaces/:space/proving/push - Push proving request to configured prover server
///
/// Returns immediately with a job_id. Use /proving/poll to check status.
pub async fn push_to_prover(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<PushResponse>, Response> {
    let space_label = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    // Check if prover endpoint is configured
    let prover_endpoint = state
        .config
        .prover_endpoint()
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "prover_endpoint not configured. Set it via POST /config",
            )
        })?;
    let prover_auth_token = state.config.prover_auth_token().ok().flatten();

    // Get the next proving request
    let request = state
        .operator
        .get_next_proving_request(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let Some(request) = request else {
        return Ok(Json(PushResponse {
            success: false,
            job_id: None,
            message: Some("no pending proving request".to_string()),
        }));
    };

    // Serialize the request
    let request_bytes = borsh::to_vec(&request)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {}", e)))?;

    // Submit to prover server
    let client = reqwest::Client::new();
    let prove_url = format!("{}/prove", prover_endpoint.trim_end_matches('/'));

    let mut req = client
        .post(&prove_url)
        .header("Content-Type", "application/octet-stream")
        .body(request_bytes);
    if let Some(t) = &prover_auth_token {
        req = req.bearer_auth(t);
    }
    let response = req
        .send()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("prover request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(json_error(
            StatusCode::BAD_GATEWAY,
            format!("prover returned {}: {}", status, body),
        ));
    }

    #[derive(Deserialize)]
    struct ProverSubmitResponse {
        job_id: String,
    }

    let submit_response: ProverSubmitResponse = response
        .json()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("invalid prover response: {}", e)))?;

    // Store the job info (space:commitment_id:is_fold -> job_id)
    let commitment_id = match &request {
        subs_core::ProvingRequest::Step { commitment_id, .. } => commitment_id,
        subs_core::ProvingRequest::Fold { commitment_id, .. } => commitment_id,
    };
    let is_fold = matches!(&request, subs_core::ProvingRequest::Fold { .. });

    let job_key = format!("job:{}:{}:{}", space, commitment_id, if is_fold { "fold" } else { "step" });
    state
        .config
        .set(&job_key, &submit_response.job_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(PushResponse {
        success: true,
        job_id: Some(submit_response.job_id),
        message: Some("proving request submitted to prover".to_string()),
    }))
}

#[derive(Serialize)]
pub struct PollResponse {
    pub success: bool,
    pub status: Option<String>,
    pub complete: bool,
    pub message: Option<String>,
}

/// POST /spaces/:space/proving/cancel - Stop the in-flight proving job.
///
/// Clears the local job key regardless of what the prover says, so the UI stops
/// waiting on a job it has abandoned. A queued job never runs; a running one
/// finishes on the prover and has its receipt discarded — see the prover's
/// cancel_job for why it cannot be interrupted mid-proof.
pub async fn cancel_proving(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let space_label: spaces_protocol::slabel::SLabel = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let prover_endpoint = state
        .config
        .prover_endpoint()
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "prover_endpoint not configured"))?;

    let request = state
        .operator
        .get_next_proving_request(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "no proving request in flight"))?;

    let commitment_id = request.commitment_id();
    let is_fold = matches!(&request, subs_core::ProvingRequest::Fold { .. });
    let job_key = format!(
        "job:{}:{}:{}",
        space,
        commitment_id,
        if is_fold { "fold" } else { "step" }
    );

    let job_id = state
        .config
        .get(&job_key)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "no job in flight for this commitment"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let url = format!(
        "{}/jobs/{}/cancel",
        prover_endpoint.trim_end_matches('/'),
        job_id
    );
    let mut req = client.post(&url);
    if let Some(t) = state.config.prover_auth_token().ok().flatten() {
        req = req.bearer_auth(t);
    }

    let prover_said = match req.send().await {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            if status.is_success() {
                serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::json!({}))
            } else {
                // A job the prover has already lost or finished is still worth
                // clearing locally, so this is reported rather than fatal.
                serde_json::json!({ "prover_status": status.as_u16(), "prover_body": body })
            }
        }
        Err(e) => serde_json::json!({ "prover_error": e.to_string() }),
    };

    // Drop the key either way: whatever the prover does with the work, subs is
    // no longer waiting on it, and leaving the key would keep the UI showing a
    // job that will never be collected.
    let _ = state.config.delete(&job_key);

    Ok(Json(serde_json::json!({
        "success": true,
        "job_id": job_id,
        "prover": prover_said,
    })))
}

/// POST /spaces/:space/proving/poll - Poll prover for job completion and save receipt
pub async fn poll_prover(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<PollResponse>, Response> {
    let space_label = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    // Check if prover endpoint is configured
    let prover_endpoint = state
        .config
        .prover_endpoint()
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "prover_endpoint not configured",
            )
        })?;
    let prover_auth_token = state.config.prover_auth_token().ok().flatten();

    // Get the next proving request to know what we're looking for
    let request = state
        .operator
        .get_next_proving_request(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let Some(request) = request else {
        return Ok(Json(PollResponse {
            success: true,
            status: None,
            complete: true,
            message: Some("no pending proving request".to_string()),
        }));
    };

    let commitment_id = match &request {
        subs_core::ProvingRequest::Step { commitment_id, .. } => *commitment_id,
        subs_core::ProvingRequest::Fold { commitment_id, .. } => *commitment_id,
    };
    let is_fold = matches!(&request, subs_core::ProvingRequest::Fold { .. });

    // Look up the job_id
    let job_key = format!("job:{}:{}:{}", space, commitment_id, if is_fold { "fold" } else { "step" });
    let job_id = state
        .config
        .get(&job_key)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "no job found - use /proving/push first",
            )
        })?;

    // Check job status
    let client = reqwest::Client::new();
    let status_url = format!("{}/jobs/{}", prover_endpoint.trim_end_matches('/'), job_id);

    let mut req = client.get(&status_url);
    if let Some(t) = &prover_auth_token {
        req = req.bearer_auth(t);
    }
    let response = req
        .send()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("prover request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(json_error(
            StatusCode::BAD_GATEWAY,
            format!("prover returned {}: {}", status, body),
        ));
    }

    /// Only what this poll acts on. Progress is deliberately absent: the
    /// pipeline view fetches it separately, and parsing it here would make a
    /// custom prover's differently-shaped `progress` fail the whole status
    /// response — turning a healthy poll into a gateway error over a field
    /// nothing reads.
    #[derive(Deserialize)]
    struct JobStatusResponse {
        status: String,
        error: Option<String>,
    }

    let job_status: JobStatusResponse = response
        .json()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("invalid prover response: {}", e)))?;

    match job_status.status.as_str() {
        "complete" => {
            // Download the receipt
            let receipt_url = format!("{}/jobs/{}/receipt", prover_endpoint.trim_end_matches('/'), job_id);
            let mut req = client.get(&receipt_url);
            if let Some(t) = &prover_auth_token {
                req = req.bearer_auth(t);
            }
            let receipt_response = req
                .send()
                .await
                .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("receipt download failed: {}", e)))?;

            if !receipt_response.status().is_success() {
                return Err(json_error(
                    StatusCode::BAD_GATEWAY,
                    format!("receipt download failed: {}", receipt_response.status()),
                ));
            }

            let receipt_bytes = receipt_response
                .bytes()
                .await
                .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("receipt read failed: {}", e)))?;

            // Save the receipt
            state
                .operator
                .fulfill_request_by_id(&space_label, commitment_id, is_fold, &receipt_bytes)
                .await
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

            // Clean up job key
            let _ = state.config.delete(&job_key);

            // The stored estimate described the proof that just finished; the
            // next one is a different shape. Cleared so it isn't read as a
            // forecast for work it says nothing about.
            let _ = state.operator.clear_estimate(&space_label, commitment_id).await;

            Ok(Json(PollResponse {
                success: true,
                status: Some("complete".to_string()),
                complete: true,
                message: Some("proof complete and saved".to_string()),
            }))
        }
        "failed" => {
            // Clean up job key
            let _ = state.config.delete(&job_key);

            Ok(Json(PollResponse {
                success: false,
                status: Some("failed".to_string()),
                complete: true,
                message: job_status.error,
            }))
        }
        status => {
            Ok(Json(PollResponse {
                success: true,
                status: Some(status.to_string()),
                complete: false,
                message: None,
            }))
        }
    }
}

/// GET /spaces/:space/proving/estimate - Get proving time estimate from configured prover
pub async fn get_estimate(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Response, Response> {
    let space_label = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let prover_endpoint = state
        .config
        .prover_endpoint()
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "prover_endpoint not configured"))?;
    let prover_auth_token = state.config.prover_auth_token().ok().flatten();

    let request = state
        .operator
        .get_next_proving_request(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let Some(request) = request else {
        return Err(json_error(StatusCode::NOT_FOUND, "no pending proving request"));
    };

    let request_bytes = borsh::to_vec(&request)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize: {}", e)))?;

    let client = reqwest::Client::new();
    let url = format!("{}/estimate", prover_endpoint.trim_end_matches('/'));

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/octet-stream")
        .body(request_bytes);
    if let Some(t) = &prover_auth_token {
        req = req.bearer_auth(t);
    }
    let response = req
        .send()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("prover request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(json_error(StatusCode::BAD_GATEWAY, format!("prover returned {}: {}", status, body)));
    }

    // Forward the JSON response from the prover as-is (arbitrary key/values)
    let estimate: serde_json::Value = response
        .json()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("invalid prover response: {}", e)))?;

    Ok((StatusCode::OK, Json(estimate)).into_response())
}

#[cfg(test)]
mod tests {
    use super::JobProgress;

    /// Stats keep the prover's order.
    ///
    /// They used to be a flattened `serde_json::Map`, which is a BTreeMap
    /// without the `preserve_order` feature — so a prover's fields were
    /// silently re-sorted alphabetically before display.
    #[test]
    fn stats_keep_the_provers_order() {
        let json = r#"{
            "label": "Proving segments",
            "phase": 1, "phase_total": 2,
            "fraction": 0.62,
            "stats": [
                {"label": "remaining", "value": "~4m 12s", "accent": true},
                {"label": "gpu", "value": "NVIDIA A100 80GB PCIe"},
                {"label": "hourly rate", "value": "$1.19"}
            ]
        }"#;

        let p: JobProgress = serde_json::from_str(json).expect("deserialize");

        let order: Vec<&str> = p.stats.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(order, ["remaining", "gpu", "hourly rate"]);
        assert!(p.stats[0].accent);
        assert!(!p.stats[1].accent, "accent defaults to false");
        assert_eq!(p.fraction, Some(0.62));
    }

    /// A prover with no numbers to report must still deserialize.
    ///
    /// This is the case that forced the redesign: a proxy booting a GPU has no
    /// segments and no cycles. When those fields were required it had to invent
    /// zeros, because an unparseable body is dropped whole and shows nothing.
    #[test]
    fn a_status_without_any_numbers_is_accepted() {
        let json = r#"{
            "label": "Booting GPU server",
            "bar": "indeterminate",
            "log": ["10:32:01 pulling image", "10:32:44 cuda ready"]
        }"#;

        let p: JobProgress = serde_json::from_str(json).expect("deserialize");

        assert_eq!(p.label.as_deref(), Some("Booting GPU server"));
        assert_eq!(p.bar.as_deref(), Some("indeterminate"));
        assert_eq!(p.log.len(), 2);
        assert!(p.stats.is_empty());
        assert_eq!(p.fraction, None);
    }

    /// An empty body is valid and says nothing, rather than failing to parse.
    #[test]
    fn an_empty_body_deserializes() {
        let p: JobProgress = serde_json::from_str("{}").expect("deserialize");
        assert!(p.label.is_none() && p.stats.is_empty() && p.log.is_empty());

        // Absent fields must not be re-emitted as nulls: subs forwards this
        // verbatim, and a wall of nulls is noise for anyone reading the API.
        let out = serde_json::to_string(&p).expect("serialize");
        assert!(!out.contains("null"), "empty progress should stay empty: {out}");
    }
}
