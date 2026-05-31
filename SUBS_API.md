# subs API Reference

HTTP API reference for the `subs` server (`subs/src/routes/*`).

- Base URL: `http://127.0.0.1:7777`
- Content type:
  - Most endpoints: `application/json`
  - Proving endpoints (`/proving/next`, `/proving/fulfill`, `/prover/*`): binary payloads
- Path params:
  - `:space` should be URL encoded when needed (example: `@mad` -> `%40mad`)

---

## Status & Spaces

### GET `/status`
Get status for all loaded spaces.

### GET `/spaces`
List currently loaded/operated spaces.

Response:
```json
{ "spaces": ["@mad", "@other"] }
```

### GET `/spaces/:space`
Get status of a specific space. Loads/creates the local space first.

### POST `/spaces/:space/operate`
Check wallet delegation and load/create the space for operation.

Response:
```json
{ "success": true, "space": "@mad" }
```

### GET `/spaces/:space/handles`
List handles with pagination and optional filtering.

Query params:
- `page` (default `1`)
- `per_page` (default `20`)
- `search` (optional)
- `filter` (optional; values used by UI include `all`, `staged`, `committed`, `parked`, `published`, `unpublished`)

### GET `/spaces/:space/handles/:handle`
Get a single handle record.

---

## Handle Requests

### POST `/requests`
Stage one or more handle requests.

Request:
```json
{
  "requests": [
    {
      "handle": "alice@mad",
      "script_pubkey": "5120...",
      "dev_private_key": null
    }
  ]
}
```

### POST `/requests/generate`
Generate a handle request and optional WIF key.

If `script_pubkey` is omitted, server generates a keypair and returns `private_key` (WIF).

Request:
```json
{ "handle": "alice@mad" }
```

Response:
```json
{
  "request": {
    "handle": "alice@mad",
    "script_pubkey": "5120...",
    "dev_private_key": "L..."
  },
  "private_key": "L..."
}
```

### POST `/requests/bulk-generate`
Generate and stage many handles in one call.

Request:
```json
{
  "space": "@mad",
  "count": 100,
  "prefix": "h"
}
```

Response:
```json
{ "staged": 100 }
```

---

## Fees, Commits, Pipeline, Publish

### GET `/fees`
Fetch recommended fee rates from mempool.space.

Response:
```json
{
  "fastestFee": 8,
  "halfHourFee": 5,
  "hourFee": 3,
  "economyFee": 2,
  "minimumFee": 1
}
```

### POST `/spaces/:space/commit`
Commit staged handles locally.

Request:
```json
{ "dry_run": false }
```

Notes:
- `dry_run=true` validates commit readiness and returns 400 if blocked.
- non-initial commits require proving before on-chain broadcast.

### POST `/spaces/:space/rollback-local`
Rollback the last unbroadcast local commitment.

Response:
```json
{ "ok": true }
```

### POST `/spaces/:space/park`
Park/unpark staged handles by explicit list or bulk search/filter.

Request:
```json
{
  "handles": ["alice", "bob"],
  "parked": true,
  "search": null,
  "filter": null
}
```

Response:
```json
{ "updated": 2 }
```

### POST `/spaces/:space/remove`
Remove staged handles by explicit list or bulk search/filter.

Request:
```json
{
  "handles": ["alice"],
  "search": null,
  "filter": null
}
```

Response:
```json
{ "removed": 1 }
```

### POST `/spaces/:space/broadcast`
Broadcast latest local commitment on-chain.

Request:
```json
{ "fee_rate": 2.0 }
```

Response:
```json
{ "txid": "..." }
```

### GET `/spaces/:space/commit/status`
Get on-chain commit status.

Response shape:
```json
{
  "status": "none|pending|confirmed|finalized",
  "txid": null,
  "block_height": null,
  "confirmations": null
}
```

### GET `/spaces/:space/pipeline`
Get UI stepper/pipeline state.

Response includes:
- flattened `PipelineStatus` (steps, counts, current step, message)
- `prover_configured`
- `proving_job_active`

### POST `/spaces/:space/publish`
Publish certificates in batches (max 100 per request).

Request (optional body):
```json
{ "handles": ["alice", "bob"] }
```

If omitted/empty, server publishes from its unpublished selector.

Response:
```json
{
  "handles_published": 2,
  "remaining": 0
}
```

---

## Proving Endpoints

These endpoints are designed for binary borsh payloads.

### GET `/spaces/:space/proving/next`
Get next proving request as borsh-serialized `Option<ProvingRequest>`.

Response content-type: `application/octet-stream`

### POST `/spaces/:space/proving/fulfill`
Submit a proof receipt in compact binary format.

Payload format:
- 8 bytes: `commitment_id` (`i64`, little-endian)
- 1 byte: `request_type` (`0` step, `1` fold)
- remaining bytes: borsh-serialized receipt

Response:
```json
{ "success": true, "message": null }
```

### POST `/spaces/:space/proving/push`
Push next proving request to configured external prover.

Response:
```json
{
  "success": true,
  "job_id": "uuid",
  "message": "proving request submitted to prover"
}
```

### POST `/spaces/:space/proving/poll`
Poll external prover for completion and persist receipt when ready.

Response:
```json
{
  "success": true,
  "status": "pending|processing|complete|failed|null",
  "complete": false,
  "message": null
}
```

### GET `/spaces/:space/proving/estimate`
Forward estimate call to configured prover and return estimate JSON as-is.

### GET `/spaces/:space/compress`
Get SNARK compression input.

Response:
```json
{
  "input": {
    "receipt": "<base64>",
    "commitment": { "...": "..." }
  }
}
```

### POST `/spaces/:space/snark`
Save compressed SNARK receipt.

Request:
```json
{ "receipt": "<base64>" }
```

Response:
```json
{ "success": true }
```

---

## Query & Certificates

### POST `/query`
Resolve one or more handles via fabric.

Request:
```json
{ "handle": "alice@mad, bob@mad" }
```

Response: array of resolved zones (`badge` + `zone`).

### GET `/query/message?handle=...`
Export binary `.spacemsg` payload for a handle.

### GET `/query/anchors`
Export root anchors as pretty JSON attachment.

### GET `/certs/:handle`
Issue certificate(s) for:
- `@space` -> root cert only
- `name@space` -> root + handle cert

Response:
```json
{
  "root_cert": "<base64 borsh Certificate>",
  "handle_cert": "<base64 borsh Certificate or null>"
}
```

---

## RPC Console Proxy

### GET `/rpc/endpoints`
Return endpoint availability + wallet and chain summary.

### POST `/rpc/spaced`
Proxy JSON-RPC call to configured spaced RPC endpoint.

Request:
```json
{
  "method": "walletlistspaces",
  "params": ["main"]
}
```

### POST `/rpc/bitcoin`
Proxy JSON-RPC call to bitcoind endpoint (test-rig mode only).

### POST `/rpc/mine`
Mine blocks in test-rig mode.

Request:
```json
{ "count": 1 }
```

---

## Runtime Configuration API

### GET `/config`
Read current persisted endpoints:
- `prover_endpoint`
- `registry_endpoint`

### POST `/config`
Set/clear endpoint values.

Request:
```json
{
  "prover_endpoint": "http://127.0.0.1:8888",
  "registry_endpoint": "http://127.0.0.1:8081"
}
```

Notes:
- pass empty string to clear a value
- omitted fields are unchanged

### POST `/config/test/prover`
Health-check prover endpoint (`GET /health`).

### POST `/config/test/registry`
Health-check registry endpoint (`GET /health`, fallback to `/`).

Request for both:
```json
{ "endpoint": "http://127.0.0.1:8888" }
```

Response:
```json
{ "success": true, "error": null }
```

---

## Registry Integration

### GET `/registry/status`
Check whether registry endpoint is configured.

### POST `/registry/sync`
Pull pending handles from configured registry (`/pending`), stage them, then acknowledge via `/ack`.

Response:
```json
{
  "success": true,
  "pulled": 10,
  "staged": 8,
  "errors": []
}
```

### POST `/registry/notify`
Notify registry webhook of committed handles for a given root.

Request:
```json
{
  "space": "@mad",
  "root": "ab935f..."
}
```

Response:
```json
{
  "success": true,
  "notified": 4,
  "message": null
}
```

---

## Error Handling

- Most failures use JSON error responses from `json_error(...)` with HTTP 4xx/5xx.
- Common statuses:
  - `400`: invalid input, missing config, invalid payload
  - `403`: space not delegated to wallet
  - `404`: not found (handle, proving request, etc.)
  - `500`: internal/storage/runtime errors
  - `502`: upstream prover/RPC/registry errors
  - `503`: unavailable dependency (e.g., fee source, rpc endpoint missing)

