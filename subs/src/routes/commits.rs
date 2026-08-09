//! Commit endpoints.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    response::Response,
};
use serde::{Deserialize, Serialize};
use subs_core::{PipelineStatus, SpaceCommitResult};

use crate::state::AppState;
use super::json_error;

/// Recommended fee rates from mempool.space
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedFees {
    pub fastest_fee: u64,
    pub half_hour_fee: u64,
    pub hour_fee: u64,
    pub economy_fee: u64,
    pub minimum_fee: u64,
}

/// Fetch recommended fees from mempool.space API
async fn fetch_recommended_fees() -> Option<RecommendedFees> {
    let url = "https://mempool.space/api/v1/fees/recommended";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    client
        .get(url)
        .send()
        .await
        .ok()?
        .json::<RecommendedFees>()
        .await
        .ok()
}

/// GET /fees - Get recommended fee rates
pub async fn get_fees() -> Result<Json<RecommendedFees>, Response> {
    match fetch_recommended_fees().await {
        Some(fees) => Ok(Json(fees)),
        None => Err(json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "could not fetch fee rates from mempool.space",
        )),
    }
}

#[derive(Deserialize)]
pub struct CommitBody {
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /spaces/{space}/commit - Commit staged handles locally
pub async fn commit_local(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Json(body): Json<CommitBody>,
) -> Result<Json<SpaceCommitResult>, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    if body.dry_run {
        // For dry run, check if commit is possible
        if let Some(reason) = state
            .operator
            .can_commit_local(&space)
            .await
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        {
            return Err(json_error(StatusCode::BAD_REQUEST, format!("cannot commit: {}", reason)));
        }
        // Return empty result for dry run
        return Ok(Json(SpaceCommitResult {
            space: space.clone(),
            prev_root: None,
            root: String::new(),
            handles_committed: 0,
            is_initial: false,
        }));
    }

    state
        .operator
        .commit_local(&space)
        .await
        .map(Json)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub struct BroadcastBody {
    #[serde(default)]
    pub fee_rate: Option<f64>,
}

#[derive(Serialize)]
pub struct BroadcastResponse {
    pub txid: String,
}

/// POST /spaces/:space/broadcast - Broadcast commit on-chain
pub async fn broadcast(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Json(body): Json<BroadcastBody>,
) -> Result<Json<BroadcastResponse>, Response> {
    use bitcoin::FeeRate;

    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let fee_rate = body.fee_rate.map(|r| FeeRate::from_sat_per_vb_unchecked(r as u64));

    let txid = state
        .operator
        .commit(&space, fee_rate)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(BroadcastResponse {
        txid: txid.to_string(),
    }))
}

#[derive(Serialize)]
pub struct CommitStatusResponse {
    pub status: String,
    pub txid: Option<String>,
    pub block_height: Option<u32>,
    pub confirmations: Option<u32>,
}

/// GET /spaces/{space}/commit/status - Get on-chain commit status
pub async fn get_commit_status(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<CommitStatusResponse>, Response> {
    use subs_core::app::CommitStatus;

    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let status = state
        .operator
        .get_commit_status(&space)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = match status {
        CommitStatus::None => CommitStatusResponse {
            status: "none".to_string(),
            txid: None,
            block_height: None,
            confirmations: None,
        },
        CommitStatus::Pending { txid, .. } => CommitStatusResponse {
            status: "pending".to_string(),
            txid: Some(txid.to_string()),
            block_height: None,
            confirmations: None,
        },
        CommitStatus::Confirmed {
            txid,
            block_height,
            confirmations,
        } => CommitStatusResponse {
            status: "confirmed".to_string(),
            txid: Some(txid.to_string()),
            block_height: Some(block_height),
            confirmations: Some(confirmations),
        },
        CommitStatus::Finalized { block_height } => CommitStatusResponse {
            status: "finalized".to_string(),
            txid: None,
            block_height: Some(block_height),
            confirmations: None,
        },
    };

    Ok(Json(response))
}

/// Read live progress for one job, best-effort.
async fn fetch_job_progress(
    state: &AppState,
    prover_endpoint: &str,
    job_id: &str,
) -> Option<super::proving::JobProgress> {
    #[derive(serde::Deserialize)]
    struct Resp {
        #[serde(default)]
        progress: Option<super::proving::JobProgress>,
    }

    let client = reqwest::Client::builder()
        // Short: this runs inside a UI poll, so a slow prover should degrade
        // to "no progress bar", not hold up the whole pipeline response.
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;

    let url = format!("{}/jobs/{}", prover_endpoint.trim_end_matches('/'), job_id);
    let mut req = client.get(&url);
    if let Some(t) = state.config.prover_auth_token().ok().flatten() {
        req = req.bearer_auth(t);
    }

    req.send().await.ok()?.json::<Resp>().await.ok()?.progress
}

/// Sanity cap on certificates per publish batch.
///
/// Not the target: publish_certs sizes each batch from the last message's
/// measured bytes, because a relay rejects anything over its 512 KB
/// max_message_size outright and the real constraint is bytes rather than
/// count. This only bounds the pathological case where a message is somehow
/// tiny — it must stay well *above* the byte-derived size, or it silently
/// caps growth and the measurement does nothing.
///
/// Measured against a 20k-handle backlog: batches converge to ~500
/// certificates for ~445 KB, so bytes bind well below this.
pub(crate) const PUBLISH_BATCH_SIZE: usize = 1000;

#[derive(Serialize)]
pub struct PublishResponse {
    pub handles_published: usize,
    pub remaining: usize,
}

#[derive(Deserialize, Default)]
pub struct PublishBody {
    /// Publish only these specific handles (empty = all unpublished)
    #[serde(default)]
    pub handles: Vec<String>,
}

/// POST /spaces/:space/publish - Publish certificates in batches
pub async fn publish_certs(
    State(state): State<AppState>,
    Path(space): Path<String>,
    body: Option<Json<PublishBody>>,
) -> Result<Json<PublishResponse>, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let handles = body.map(|b| b.0.handles).unwrap_or_default();

    let (count, remaining) = state
        .operator
        .publish_certs(
            &space,
            PUBLISH_BATCH_SIZE,
            &handles,
            state.publish_require_finalized,
        )
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if state.publish_require_finalized
                && (msg.contains("confirmations")
                    || msg.contains("not broadcast")
                    || msg.contains("not confirmed")
                    || msg.contains("not committed")
                    || msg.contains("RPC required"))
            {
                json_error(StatusCode::CONFLICT, msg)
            } else {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        })?;

    Ok(Json(PublishResponse {
        handles_published: count,
        remaining,
    }))
}

/// POST /spaces/:space/rollback-local - Rollback the last unbroadcast local commitment
pub async fn rollback_local(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let space_label: spaces_protocol::slabel::SLabel = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    // Grab any in-flight proving job info before rollback so we can clean up
    let proving_request = state
        .operator
        .get_next_proving_request(&space_label)
        .await
        .ok()
        .flatten();

    state
        .operator
        .rollback_local(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Clean up any prover job keys for the rolled-back commitment
    if let Some(req) = proving_request {
        let cid = req.commitment_id();
        let step_key = format!("job:{}:{}:step", space, cid);
        let fold_key = format!("job:{}:{}:fold", space, cid);
        let _ = state.config.delete(&step_key);
        let _ = state.config.delete(&fold_key);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ParkBody {
    #[serde(default)]
    pub handles: Vec<String>,
    #[serde(default)]
    pub parked: bool,
    /// Bulk mode: park/unpark all staged handles matching search/filter
    pub search: Option<String>,
    pub filter: Option<String>,
}

/// POST /spaces/:space/park - Park or unpark staged handles
pub async fn park_handles(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Json(body): Json<ParkBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let count = state
        .operator
        .set_parked(&space, &body.handles, body.parked, body.search, body.filter)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(serde_json::json!({ "updated": count })))
}

#[derive(Deserialize)]
pub struct RemoveBody {
    #[serde(default)]
    pub handles: Vec<String>,
    pub search: Option<String>,
    pub filter: Option<String>,
}

/// POST /spaces/:space/remove - Remove staged handles
pub async fn remove_handles(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Json(body): Json<RemoveBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let space = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    let count = state
        .operator
        .remove_staged(&space, &body.handles, body.search, body.filter)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(serde_json::json!({ "removed": count })))
}

/// GET /spaces/:space/pipeline - Get commitment pipeline status for stepper UI
/// Extended pipeline status with prover config info for the UI
#[derive(Serialize)]
pub struct PipelineResponse {
    #[serde(flatten)]
    pub status: PipelineStatus,
    /// Whether a prover endpoint is configured in settings
    pub prover_configured: bool,
    /// Whether a proving job is currently in flight on the prover
    pub proving_job_active: bool,
    /// When true, publish is blocked until commitments reach 150 confirmations.
    pub publish_require_finalized: bool,
    /// Whether publishing is currently allowed for unpublished handles.
    pub publish_allowed: bool,
    /// Reason publish is blocked, when `publish_require_finalized` is enabled.
    pub publish_blocked_reason: Option<String>,
    /// Which proof of the commitment is next (1-based), when proving.
    pub proof_index: Option<usize>,
    /// How many proofs this commitment needs in total: the first commitment
    /// after genesis needs only a step, later ones need step + fold.
    pub proof_total: Option<usize>,
    /// Prover-side id of the in-flight job, so it can be correlated with the
    /// prover's own logs and with the runpod proxy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proving_job_id: Option<String>,
    /// Live progress of the in-flight proof, fetched from the prover.
    /// Best-effort: absent if the prover is unreachable or predates progress
    /// reporting, which must not fail the pipeline view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proving_progress: Option<super::proving::JobProgress>,
}

pub async fn get_pipeline_status(
    State(state): State<AppState>,
    Path(space): Path<String>,
) -> Result<Json<PipelineResponse>, Response> {
    let space_label: spaces_protocol::slabel::SLabel = space
        .parse()
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, format!("invalid space: {}", e)))?;

    // Ensure space is loaded
    state
        .operator
        .load_or_create_space(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let status = state
        .operator
        .get_pipeline_status(&space_label)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let prover_configured = state.config.prover_endpoint().unwrap_or(None).is_some();

    // Check if there's an active proving job by looking for a job key in config.
    // The job key uses the commitment's SQLite row id from the proving request,
    // matching the format used by push_to_prover and the background loop.
    // Also report which proof of this commitment is next. count_pending_proofs
    // charges a fold only from idx >= 2, so the first commitment after genesis
    // is a single step proof and later ones are step + fold.
    let mut proof_index = None;
    let mut proof_total = None;
    let mut active_job_id: Option<String> = None;

    let proving_job_active = if let Some(idx) = status.commitment_idx {
        if let Ok(Some(req)) = state.operator.get_next_proving_request(&space_label).await {
            let cid = req.commitment_id();
            let is_fold = matches!(&req, subs_types::ProvingRequest::Fold { .. });
            let kind = if is_fold { "fold" } else { "step" };

            proof_total = Some(if idx >= 2 { 2 } else { 1 });
            proof_index = Some(if is_fold { 2 } else { 1 });

            let job_key = format!("job:{}:{}:{}", space, cid, kind);
            active_job_id = state.config.get(&job_key).unwrap_or(None);
            active_job_id.is_some()
        } else {
            false
        }
    } else {
        false
    };

    let publish_require_finalized = state.publish_require_finalized;
    let (publish_allowed, publish_blocked_reason) = if !publish_require_finalized || status.unpublished == 0 {
        (true, None)
    } else {
        match state
            .operator
            .publish_gate(&space_label, publish_require_finalized, PUBLISH_BATCH_SIZE)
            .await
        {
            Ok((allowed, reason)) => (allowed, reason),
            Err(e) => (false, Some(e.to_string())),
        }
    };

    // Ask the prover how far along it is. Only the prover knows, and the value
    // is stale the moment it is cached, so it is fetched per request rather
    // than stored. Failures are swallowed: a missing progress bar is a much
    // better outcome than a broken pipeline view.
    let proving_progress = match (active_job_id.as_deref(), state.config.prover_endpoint().ok().flatten()) {
        (Some(job_id), Some(endpoint)) => fetch_job_progress(&state, &endpoint, job_id).await,
        _ => None,
    };

    Ok(Json(PipelineResponse {
        status,
        prover_configured,
        proving_job_active,
        publish_require_finalized,
        publish_allowed,
        publish_blocked_reason,
        proof_index,
        proof_total,
        proving_job_id: active_job_id,
        proving_progress,
    }))
}
