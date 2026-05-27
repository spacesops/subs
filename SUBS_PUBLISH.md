# Publishing a handle with subs

End-to-end sequence to **create → stage → commit → broadcast → publish → resolve** a handle named **`test@swifty`**.

This document matches the behavior of the subs operator UI and REST API as implemented in `subs-core` and `subs`.

---

## Names and URLs

| Concept | Example | Notes |
|--------|---------|--------|
| **Space label** (`SLabel`) | `@swifty` | May also be a `bc1q…` sovereignty label on mainnet. Use the exact label from `spaced`. |
| **Full handle** (`SName`) | `test@swifty` | Subname `test` in space `@swifty`. |
| **Space API path** | `/spaces/%40swifty/...` | URL-encode `@` as `%40`. |
| **Local data** | `$SUBS_DATA_DIR/@swifty/` | SQLite + SpaceDB under the space label. |

Throughout this doc, **`SPACE=@swifty`** and **`HANDLE=test@swifty`**.

---

## Prerequisites

Before staging `test@swifty`:

1. **`spaced`** is running and reachable (`SUBS_SPACED_RPC_URL`, credentials if required).
2. **`subs`** is running with the wallet that **operates** the space:
   ```bash
   subs --rpc-url http://127.0.0.1:7225 --wallet my-wallet --data-dir ./data --port 7777
   ```
3. **`subs-prover`** is running and configured in subs (Settings or `SUBS_PROVER_ENDPOINT`):
   ```bash
   subs-prover --server --server-port 8888
   ```
4. **Wallet delegation**: the wallet must be allowed to operate `@swifty` on-chain (`wallet_can_operate`). For a new space, the sovereignty owner delegates operation to your wallet first.
5. **Fabric / certrelay**: publish and resolve use the fabric network (`Fabric::new()` default seeds). No extra config is required for normal operation; subs builds a chain proof from `spaced` and broadcasts certificates to relays.

Open the operator UI: **http://127.0.0.1:7777**

---

## Pipeline overview

Each handle goes through these phases (see the stepper on the space page):

```
Stage → Local commit → [Proving] → Broadcast → Confirmed → Finalized → Publish → Resolve
```

| Step | What happens | On-chain? |
|------|----------------|-----------|
| **Stage** | Handle + `script_pubkey` stored locally; not in commitment tree | No |
| **Local commit** | Merkle root computed; commitment row in local DB | No |
| **Proving** | STARK proof for step/fold (required for **2nd+** commitments only) | No |
| **Broadcast** | Commit tx sent via `spaced` wallet | Yes |
| **Confirmed** | Commit tx mined; tip root matches local commitment | Yes |
| **Finalized** | ≥ **150** confirmations on that commit (UI step) | Yes |
| **Publish** | Certificates issued + broadcast to fabric relays | Relays |
| **Resolve** | Query fabric for verified zone | Relays |

**Important:** Do **not** publish until the commitment is **broadcast and visible on-chain**. Publishing while still only locally committed produces **temporary** certificates signed with the space sovereignty key; relays often reject those with `signature invalid for test@swifty`. Wait until **Broadcast** completes and the chain tip includes your root (pipeline **Confirmed** or later).

---

## Step 1 — Operate the space

Load/create local state for `@swifty` and verify the wallet can operate it.

**UI:** Dashboard → select **@swifty** (or add via Operate).

**API:**
```bash
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/operate"
```

**Success:** `{ "success": true, "space": "@swifty" }`  
**Failure:** `403` if the wallet is not delegated to operate this space.

---

## Step 2 — Stage `test@swifty`

Register the handle in the local staging area with a `script_pubkey` (hex-encoded script bytes).

### Option A — Generate keypair (dev / testing)

**API:**
```bash
# Returns HandleRequest + WIF private key
curl -s -X POST "http://127.0.0.1:7777/requests/generate" \
  -H "Content-Type: application/json" \
  -d '{"handle":"test@swifty"}'
```

Take `request` from the response and stage it:

```bash
curl -s -X POST "http://127.0.0.1:7777/requests" \
  -H "Content-Type: application/json" \
  -d '{"requests":[{"handle":"test@swifty","script_pubkey":"<hex from generate>"}]}'
```

### Option B — Known script pubkey

```bash
curl -s -X POST "http://127.0.0.1:7777/requests" \
  -H "Content-Type: application/json" \
  -d '{
    "requests": [{
      "handle": "test@swifty",
      "script_pubkey": "5120..."
    }]
  }'
```

**UI:** Use handle generation / staging flows on the space page (or import from registry sync).

**Verify:**
```bash
curl -s "http://127.0.0.1:7777/spaces/%40swifty/handles?filter=staged"
```

Handle should show **staged**, no `commitment_root`, `publish_status` null.

---

## Step 3 — Local commit

Merge staged handles into the space commitment tree (local only).

**UI:** **Commit Local** on the space pipeline.

**API:**
```bash
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/commit" \
  -H "Content-Type: application/json" \
  -d '{"dry_run":false}'
```

**Success:** `{ "handles_committed": 1, "is_initial": true|false, ... }`

- **`is_initial: true`** (first commitment, `idx == 0`): no proving required before broadcast.
- **`is_initial: false`**: you **must** complete proving (step 4) before broadcast.

**Verify:** Handle shows **committed** with a `commitment_root` and `commitment_idx`.

---

## Step 4 — Proving (non-initial commits only)

Skip this step for the **first** commitment in a space.

For the second and later commits, subs creates a STARK proving request that must be fulfilled before on-chain broadcast.

**UI:** **Prove** (requires prover URL in Settings) → polls until complete.

**API (typical flow):**
```bash
# Submit job to configured prover
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/proving/push"

# Poll until done (UI does this automatically)
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/proving/poll"
```

Alternative: fetch binary request from `GET .../proving/next`, prove offline, `POST .../proving/fulfill`.

**Verify:** Pipeline step **Proving** = complete; `GET .../proving/next` returns empty.

---

## Step 5 — Broadcast on-chain

Submit the commitment root to `spaced` / Bitcoin.

**UI:** **Broadcast On-Chain** (fee modal).

**API:**
```bash
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/broadcast" \
  -H "Content-Type: application/json" \
  -d '{"fee_rate": 1.0}'
```

**Success:** `{ "txid": "..." }`

**Verify:**
```bash
curl -s "http://127.0.0.1:7777/spaces/%40swifty/pipeline"
curl -s "http://127.0.0.1:7777/spaces/%40swifty/commit/status"
```

Pipeline moves to **Confirmed** (mined) then **Finalized** (≥ 150 confirmations). The UI treats **Finalized** as “ready to publish certificates,” but the critical requirement is that the **on-chain tip root matches** your commitment (Confirmed), not necessarily all 150 blocks—though waiting for **Finalized** is recommended.

---

## Step 6 — Publish certificates

Issue certificates for unpublished handles and broadcast them to the fabric relay network.

**UI:** **Publish** bar on the space page (batches up to 100 handles per request).

**API:**
```bash
# All unpublished handles in the space
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/publish"

# Single handle
curl -s -X POST "http://127.0.0.1:7777/spaces/%40swifty/publish" \
  -H "Content-Type: application/json" \
  -d '{"handles":["test"]}'
```

**What subs does internally:**

1. Select unpublished handles (respecting on-chain confirmed commitment index).
2. **`issue_certs`**: for each handle, build root cert + leaf cert (`test@swifty`).
   - If handle is in the tree at **on-chain tip** → **final** leaf cert (inclusion proof, no Schnorr on leaf).
   - If not yet on-chain at tip → **temp** leaf cert (exclusion proof + Schnorr signature from operating wallet).
3. **`build_message`**: `build_chain_proof` RPC against `spaced`.
4. **`fabric.broadcast`**: send message bytes to relays.

**Success:** `{ "handles_published": 1, "remaining": 0 }`

**Verify:** Handle `publish_status` becomes `temp` or `final` in DB/UI.

| `publish_status` | Meaning |
|------------------|---------|
| `null` | Not published |
| `temp` | Temp cert on relays; may need republish after chain advances |
| `final` | Final cert; handle commitment is confirmed on-chain |

**Common failure:**
```text
Could not broadcast message: relay error (400): rejected: signature invalid for test@swifty
```
Usually caused by publishing **before broadcast confirms**, wrong operating wallet, or stale temp cert. Fix: wait for on-chain commit, then publish again.

---

## Step 7 — Resolve `test@swifty`

Query the fabric network for the verified zone (after relays have accepted the publish).

**UI:** **Query** page → enter `test@swifty` → **Resolve**.

**API:**
```bash
curl -s -X POST "http://127.0.0.1:7777/query" \
  -H "Content-Type: application/json" \
  -d '{"handle":"test@swifty"}'
```

**Success:** JSON array of `ResolvedZone` with `badge` (`orange` / `unverified` / `none`) and zone fields (`script_pubkey`, records, etc.).

Export binary proof bundle:
```bash
curl -s "http://127.0.0.1:7777/query/message?handle=test%40swifty" -o query.spacemsg
```

---

## Quick reference — minimal curl sequence

Assume first commitment (`is_initial: true`), prover not needed, space already delegated:

```bash
BASE=http://127.0.0.1:7777
SPACE=%40swifty

curl -X POST "$BASE/spaces/$SPACE/operate"

curl -X POST "$BASE/requests/generate" -H "Content-Type: application/json" \
  -d '{"handle":"test@swifty"}' | tee /tmp/gen.json

# Edit: extract script_pubkey from .request and POST /requests

curl -X POST "$BASE/spaces/$SPACE/commit" -H "Content-Type: application/json" \
  -d '{"dry_run":false}'

# Wait until broadcast is appropriate (no pending proving)

curl -X POST "$BASE/spaces/$SPACE/broadcast" -H "Content-Type: application/json" \
  -d '{"fee_rate": 1.0}'

# Wait for commit tx to confirm on-chain

curl -X POST "$BASE/spaces/$SPACE/publish" -H "Content-Type: application/json" \
  -d '{"handles":["test"]}'

curl -X POST "$BASE/query" -H "Content-Type: application/json" \
  -d '{"handle":"test@swifty"}'
```

---

## Optional — registry-server

Registry is **not** required for the publish/resolve flow above. It is a separate queue for pulling handle registrations into staging:

- `POST /registry/sync` — pull pending handles from registry into staging
- `POST /registry/notify` — notify registry after a commitment is finalized

Use registry when external parties register handles; otherwise stage via `/requests` directly.

---

## State diagram (handle `test`)

```text
[ staged ]  --commit local-->  [ committed locally ]
                                      |
                               broadcast tx
                                      v
                            [ on-chain at tip ]
                                      |
                                  publish
                                      v
                         [ temp or final on relays ]
                                      |
                                   resolve
                                      v
                            [ zone visible in /query ]
```

---

## Related code

| Step | Primary implementation |
|------|------------------------|
| Stage | `Operator::add_requests` → `LocalSpace::add_request` |
| Local commit | `POST /spaces/:space/commit` → `Operator::commit_local` |
| Proving | `POST /spaces/:space/proving/*` |
| Broadcast | `POST /spaces/:space/broadcast` → `Operator::commit` |
| Publish | `POST /spaces/:space/publish` → `Operator::publish_certs` → `submit_certs` |
| Resolve | `POST /query` → `Operator::resolve` |

Pipeline step logic (150 confirmations, publish readiness): `Operator::get_pipeline_status` in `core/src/app.rs`.
