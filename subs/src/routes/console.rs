//! RPC console proxy endpoints.

use axum::{
    extract::State,
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};

use reqwest::RequestBuilder;

use crate::state::AppState;
use super::json_error;

/// Apply Spaced RPC credentials to an outbound request (same precedence as `build_rpc_client`).
fn apply_spaced_rpc_auth(
    req: RequestBuilder,
    state: &AppState,
) -> Result<RequestBuilder, Response> {
    if let Some(user) = state.spaced_rpc_user.as_deref() {
        return Ok(req.basic_auth(user, state.spaced_rpc_password.as_deref()));
    }
    if let Some(path) = state.spaced_rpc_cookie.as_ref() {
        let cookie = std::fs::read_to_string(path).map_err(|e| {
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read RPC cookie file: {e}"),
            )
        })?;
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            cookie.trim().as_bytes(),
        );
        return Ok(req.header("Authorization", format!("Basic {encoded}")));
    }
    Ok(req)
}

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct MineRequest {
    #[serde(default = "default_mine_count")]
    #[cfg_attr(not(feature = "test-rig"), allow(dead_code))]
    pub count: u32,
}

fn default_mine_count() -> u32 {
    1
}

#[derive(Debug, Serialize)]
pub struct RpcEndpoints {
    pub spaced: Option<String>,
    pub bitcoin: Option<String>,
    pub certrelay: Option<String>,
    pub wallet: String,
    pub balance_sats: u64,
    pub network: String,
    pub tip_height: u32,
    pub chain_height: u32,
    pub synced: bool,
}

/// GET /rpc/endpoints - Get available RPC endpoints and status
pub async fn get_endpoints(State(state): State<AppState>) -> Json<RpcEndpoints> {
    use spaces_client::rpc::RpcClient;

    let wallet = state.operator.wallet().to_string();
    let (balance_sats, network, tip_height, chain_height, synced) = if let Some(rpc) = state.operator.rpc() {
        let server_info = rpc.get_server_info().await.ok();
        let balance = rpc.wallet_get_balance(&wallet).await.ok();

        let (network, tip_height, chain_height, progress) = server_info
            .map(|info| (info.network.to_string(), info.tip.height, info.chain.blocks, info.progress))
            .unwrap_or_else(|| ("unknown".to_string(), 0, 0, 0.0));

        let balance_sats = balance.map(|b| b.balance.to_sat()).unwrap_or(0);
        (balance_sats, network, tip_height, chain_height, progress >= 1.0)
    } else {
        (0, "offline".to_string(), 0, 0, false)
    };

    Json(RpcEndpoints {
        spaced: state.spaced_rpc_url.clone(),
        bitcoin: state.bitcoin_rpc_url.clone(),
        certrelay: state.certrelay_url.clone(),
        wallet,
        balance_sats,
        network,
        tip_height,
        chain_height,
        synced,
    })
}

/// POST /rpc/spaced - Proxy RPC call to spaced
pub async fn proxy_spaced(
    State(state): State<AppState>,
    Json(request): Json<RpcRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    let rpc_url = state
        .spaced_rpc_url
        .as_ref()
        .ok_or_else(|| json_error(StatusCode::SERVICE_UNAVAILABLE, "Spaced RPC URL not configured"))?;

    proxy_rpc_call(rpc_url, &request, |req| apply_spaced_rpc_auth(req, &state)).await
}

/// POST /rpc/bitcoin - Proxy RPC call to bitcoind (test-rig only)
pub async fn proxy_bitcoin(
    State(state): State<AppState>,
    Json(request): Json<RpcRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    let rpc_url = state
        .bitcoin_rpc_url
        .as_ref()
        .ok_or_else(|| json_error(StatusCode::SERVICE_UNAVAILABLE, "Bitcoin RPC not available (only in test-rig mode)"))?;

    proxy_rpc_call(rpc_url, &request, |req| {
        Ok(req.basic_auth("user", Some("password")))
    })
    .await
}

/// POST /rpc/mine - Mine blocks (test-rig only)
#[cfg(feature = "test-rig")]
pub async fn mine_blocks(
    State(state): State<AppState>,
    Json(request): Json<MineRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    let test_rig = state
        .test_rig
        .as_ref()
        .ok_or_else(|| json_error(StatusCode::SERVICE_UNAVAILABLE, "Mining only available in test-rig mode"))?;

    test_rig
        .mine_blocks(request.count as usize)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to mine: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "blocks_mined": request.count
    })))
}

#[cfg(not(feature = "test-rig"))]
pub async fn mine_blocks(
    State(_state): State<AppState>,
    Json(_request): Json<MineRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    Err(json_error(StatusCode::SERVICE_UNAVAILABLE, "Mining only available in test-rig mode"))
}

async fn proxy_rpc_call<F>(
    rpc_url: &str,
    request: &RpcRequest,
    apply_auth: F,
) -> Result<Json<serde_json::Value>, Response>
where
    F: FnOnce(RequestBuilder) -> Result<RequestBuilder, Response>,
{
    let client = reqwest::Client::new();

    // Build JSON-RPC request
    let rpc_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": request.method,
        "params": request.params,
    });

    let req = client
        .post(rpc_url)
        .header("Content-Type", "application/json")
        .json(&rpc_body);
    let req = apply_auth(req)?;

    let response = req
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("RPC request failed: {}", e)))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| json_error(StatusCode::BAD_GATEWAY, format!("Invalid RPC response: {}", e)))?;

    if !status.is_success() {
        // Return the error from the RPC server
        if let Some(error) = body.get("error") {
            return Ok(Json(serde_json::json!({
                "error": error,
                "result": null
            })));
        }
        return Err(json_error(StatusCode::BAD_GATEWAY, format!("RPC error: {}", status)));
    }

    Ok(Json(body))
}
