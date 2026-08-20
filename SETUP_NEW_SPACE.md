# Setting Up a New Space — Recommended Sequence

How to stage, commit, prove, broadcast, publish, and resolve handles in a new space using subs.

---

## The correct per-commit cycle

Every commitment (genesis or later) follows the same skeleton:

```
Stage → Commit Local → [Prove if idx ≥ 1] → Broadcast → Confirmed → Publish → Resolve
```

**Publish always comes after broadcast confirms on-chain**, not before.

---

## First commitment (genesis, `idx == 0`)

Recommended for the **first handle(s)** in a new space:

1. **Operate** the space
2. **Stage** one or more handles (all unstaged, unparked handles go into one commit)
3. **Commit Local** — no proving required
4. **Broadcast** on-chain
5. **Wait until Confirmed** (on-chain tip root matches your commitment)
6. **Publish** certificates
7. **Resolve** via Query UI / `fabric resolve`

You need **broadcast + confirm** between commit and publish — not `stage → commit → publish` alone.

You can put **multiple handles in the first commit** — there is no minimum batch size of 2. `commit_local` takes **all staged, unparked handles at once**.

**Example (first commitment, no prover needed):**

```bash
BASE=http://127.0.0.1:7777
SPACE=%40swifty

curl -X POST "$BASE/spaces/$SPACE/operate"
# Stage handle(s) via UI or POST /requests

curl -X POST "$BASE/spaces/$SPACE/commit" \
  -H "Content-Type: application/json" -d '{"dry_run":false}'

curl -X POST "$BASE/spaces/$SPACE/broadcast" \
  -H "Content-Type: application/json" -d '{"fee_rate": 1.0}'

# Wait until GET .../commit/status shows Confirmed

curl -X POST "$BASE/spaces/$SPACE/publish"

curl -X POST "$BASE/query" \
  -H "Content-Type: application/json" -d '{"handle":"test@swifty"}'
```

---

## Second commitment (`idx == 1`)

For the next handle(s):

1. **Stage** the new handle(s)
2. **Commit Local** — creates a non-initial commit
3. **Prove** (STARK step receipt) — **required before broadcast**
4. **Broadcast**
5. **Wait until Confirmed**
6. **Publish**
7. **Resolve**

Order: **prove → broadcast → wait for confirmation → publish**, not prove → publish → wait.

**Example (proving required):**

```bash
curl -X POST "$BASE/spaces/$SPACE/commit" \
  -H "Content-Type: application/json" -d '{"dry_run":false}'

curl -X POST "$BASE/spaces/$SPACE/proving/push"
curl -X POST "$BASE/spaces/$SPACE/proving/poll"
# Repeat poll until proving step is Complete

curl -X POST "$BASE/spaces/$SPACE/broadcast" \
  -H "Content-Type: application/json" -d '{"fee_rate": 1.0}'

# Wait until Confirmed

curl -X POST "$BASE/spaces/$SPACE/publish" \
  -H "Content-Type: application/json" -d '{"handles":["newhandle"]}'
```

---

## Third+ commitments (`idx >= 2`)

Same as the second commitment, but proving may include a **fold** step in addition to the step proof. Commitment index 2+ requires an **aggregate (fold) receipt**, not just the step receipt.

---

## Batching

There is **no rule that batches must be ≥ 2**. Batching is an operational choice:

| Layer | Behavior |
|-------|----------|
| **Commit** | All staged, unparked handles in one local commit |
| **Publish** | Up to **100 handles** per publish request |

Practical guidance:

- **Genesis commit**: batch as many handles as you want in the first commit to avoid extra on-chain transactions and skip proving entirely for that commit.
- **Later commits**: batch when it makes sense (e.g. weekly registry sync), knowing each batch needs **prove → broadcast → confirm → publish**.
- **Single-handle commits** are fine — just slower and more expensive on-chain.

---

## Gate before the next local commit

With RPC connected, `can_commit_local` blocks a new commit until the **previous** one is fully settled:

1. Previous non-genesis commit has **proving complete**
2. Previous commit is **broadcast**
3. Previous commit has **≥ 150 confirmations** (UI “Finalized” step)

Steady-state rhythm:

```
stage batch → commit → prove → broadcast → wait (150 confs) → publish
                                                      ↓
                                            stage next batch
```

**Publish does not require 150 confirmations** — only **Confirmed** is enough. The 150-block wait is for starting the **next** local commit.

---

## Parked handles

Handles marked **parked** are excluded from the next `commit_local`. Unpark them before committing if you want them included.

---

## Quick reference

| Commitment | Proving | Before publish | Before next commit |
|------------|---------|----------------|------------------|
| Genesis (`idx == 0`) | Skipped | Broadcast + Confirmed | Broadcast + 150 confs |
| Second (`idx == 1`) | Step receipt | Prove + Broadcast + Confirmed | Prove + Broadcast + 150 confs |
| Third+ (`idx >= 2`) | Step + fold receipts | Prove + Broadcast + Confirmed | Prove + Broadcast + 150 confs |

---

## Common mistakes

| Wrong | Right |
|-------|-------|
| `stage → commit → publish` | `stage → commit → broadcast → confirm → publish` |
| `prove → publish → wait` | `prove → broadcast → confirm → publish` |
| Wait for batch ≥ 2 before committing | Commit any number of staged handles; batching is optional |
| Publish before on-chain confirm | Wait for Confirmed; early publish causes `signature invalid` errors |

---

## Recommended pattern for a new space

1. **Genesis**: stage N handles → one commit → broadcast → confirm → publish all N.
2. **Ongoing**: accumulate staged handles → when ready, one commit cycle (prove if not genesis) → broadcast → confirm → publish (batched up to 100).

---

## Related docs

| Topic | File |
|-------|------|
| Full publish walkthrough | `SUBS_PUBLISH.md` |
| Query / resolve mechanics | `VERITAS_RESOLUTION.md` |
| Fixing publish/resolve failures | `FIX_SIGNATURE_INVALID.md` |
| Pipeline implementation | `core/src/app.rs` (`get_pipeline_status`, `can_commit_local`) |
| Commit logic | `core/src/core.rs` (`commit`, `prepare_zk_input`) |
