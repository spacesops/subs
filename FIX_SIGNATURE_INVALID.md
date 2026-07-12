# Fixing "signature invalid" / Unresolvable Subhandles

Symptom: `@space` resolves from the chain anchor alone, but a subhandle like `gxxxxxx@space` fails to resolve (e.g. `receipt required for @space`), and/or publish returns `rejected: signature invalid for <handle>@space`.

The failure means relays have enough data to anchor `@space` at the chain level, but not a **complete, verifiable certificate chain** for subhandles. For commitment index ≥ 1, the **root cert must include a ZK receipt**; without it, `libveritas` rejects the bundle and the subhandle cannot resolve.

Fix it by finishing the pipeline, then **republishing**.

---

## 1. Diagnose where you are stuck

Open the space page for `@space` and check the pipeline stepper, or use the API:

```bash
BASE=http://127.0.0.1:7777
SPACE=%40space   # URL-encoded @space

# If basic auth is enabled, prefix curl with:
#   -u "$SUBS_BASIC_AUTH_USER:$SUBS_BASIC_AUTH_PASSWORD"

curl -s "$BASE/spaces/$SPACE/pipeline" | jq
curl -s "$BASE/spaces/$SPACE/commit/status" | jq
curl -s "$BASE/spaces/$SPACE/handles?filter=unpublished" | jq
curl -s "$BASE/spaces/$SPACE/handles/gxxxxxx" | jq
```

Look for:

| Signal | Meaning |
|--------|---------|
| `current_step: "proving"` | STARK proof not done — **cannot publish valid root cert** |
| `current_step: "broadcast"` | Committed locally but not on-chain |
| `confirmed` but not `finalized` | On-chain but UI still waiting (publish is OK once **Confirmed**) |
| `publish_status: null` | Never successfully published |
| `publish_status: "temp"` | Published before chain caught up — may need republish |
| Space shows **STARK** badge | Receipt exists locally |
| **Untracked On-Chain Commitment** warning | Local DB out of sync with chain — fix data dir / sync first |

Also check the handle’s `commitment_idx` vs on-chain tip: if the handle is committed locally but not yet at chain tip, publish will issue **temp** certs that veritas may not accept the same way.

---

## 2. Complete the pipeline (in order)

The full sequence is documented in `SUBS_PUBLISH.md`:

```
Stage → Local commit → [Proving] → Broadcast → Confirmed → Publish → Resolve
```

### If handles are still staged

**Commit Local** (UI) or:

```bash
curl -X POST "$BASE/spaces/$SPACE/commit" \
  -H "Content-Type: application/json" -d '{"dry_run":false}'
```

### If this is commitment #2 or later (`is_initial: false`)

You **must** prove before broadcast. Root certs embed the receipt from local DB:

```bash
# Prover must be running and SUBS_PROVER_ENDPOINT reachable
curl -X POST "$BASE/spaces/$SPACE/proving/push"
curl -X POST "$BASE/spaces/$SPACE/proving/poll"
```

Repeat poll until proving step is **Complete**. Verify:

```bash
curl -s "$BASE/spaces/$SPACE/proving/next"
# should return empty / no pending request
```

Without a stored step (and fold, for idx ≥ 2) receipt, `issue_cert` for `@space` cannot attach the receipt libveritas requires.

### Broadcast on-chain

```bash
curl -X POST "$BASE/spaces/$SPACE/broadcast" \
  -H "Content-Type: application/json" -d '{"fee_rate": 1.0}'
```

Wait until `commit/status` shows **Confirmed** (on-chain tip root matches your commitment). **Do not publish before this** — early publish produces temp certs that relays often reject (`signature invalid for <handle>@space`).

### Publish certificates

Publish the subhandle (subs includes the root cert in the bundle):

```bash
curl -X POST "$BASE/spaces/$SPACE/publish" \
  -H "Content-Type: application/json" \
  -d '{"handles":["gxxxxxx"]}'
```

Or use **Publish** on the space page. Subs will:

1. Reset stale temp certs if chain tip moved
2. Issue root cert **with receipt** + leaf cert for `gxxxxxx@space`
3. Broadcast to fabric relays

Success looks like:

```json
{ "handles_published": 1, "remaining": 0 }
```

Handle should show `publish_status: "final"` (not `temp`) once its commitment is confirmed on-chain.

---

## 3. Verify resolution

```bash
curl -X POST "$BASE/query" \
  -H "Content-Type: application/json" \
  -d '{"handle":"gxxxxxx@space"}'
```

Or use the Query UI. You want a zone back with script pubkey and records, not a verify error.

CLI equivalent:

```bash
fabric resolve gxxxxxx@space
```

---

## 4. If you already published but resolve still fails

**Republish after fixing upstream steps:**

1. Confirm proving is complete and commitment is **Confirmed** on-chain.
2. Republish the handle (UI: **Re-publish Certificate**, or same `POST /publish` curl).
3. `publish_certs` automatically calls `reset_stale_temp_certs` when the chain tip changed, clearing old temp publishes so they get reissued.

If publish returns `signature invalid for ...`:

- Commitment not yet at chain tip, or
- Wrong operating wallet, or
- Stale temp cert — wait for confirm, then republish.

If publish succeeds but resolve still says `receipt required for @space`:

- Root cert on relays is still old/incomplete — republish after proving receipts exist locally (check for **STARK** badge on space page).
- Relays may need a moment to propagate; retry resolve after a successful publish.

---

## 5. Docker-specific checks

Ensure inside the container:

- `SUBS_PROVER_ENDPOINT` points to a **running** prover (host `127.0.0.1:8888` from inside Docker is the container itself — use host IP or run prover in the same container).
- `SUBS_SPACED_RPC_URL` reaches your spaced node.
- `SUBS_DATA_DIR` is the **same** data directory where commits and receipts were stored (wrong data dir → missing receipts → publish without receipt).

---

## Quick decision tree

```
Is commitment idx >= 1?
  ├─ Yes → Is proving complete? (step/fold receipts in DB)
  │         ├─ No  → Prove first, then broadcast, then publish
  │         └─ Yes → Is commit on-chain (Confirmed)?
  │                   ├─ No  → Broadcast, wait for confirm, then publish
  │                   └─ Yes → Publish (or republish) gxxxxxx
  └─ No (genesis only) → Broadcast if needed, then publish
```

The root cause is almost always: **subhandle publish never completed with a root cert that includes the STARK receipt**, because proving, broadcast, or publish was incomplete or done out of order. Walk the pipeline on `@space` until every step is green, then republish `gxxxxxx`.

---

## Related code

| Concern | Location |
|---------|----------|
| Publish flow | `core/src/app.rs` (`publish_certs`, `issue_certs`, `issue_cert`) |
| Root cert + receipt | `core/src/core.rs` (`issue_cert`, `get_receipt`) |
| Stale temp reset | `core/src/storage.rs` (`reset_stale_temp_certs`) |
| Unpublished selection | `core/src/storage.rs` (`HandleSelector::Unpublished`) |
| Pipeline status | `core/src/app.rs` (`get_pipeline_status`) |
| Resolve/verify | `core/src/app.rs` (`resolve`) + `fabric-resolver` / `libveritas` |
| Full walkthrough | `SUBS_PUBLISH.md`, `VERITAS_RESOLUTION.md` |
