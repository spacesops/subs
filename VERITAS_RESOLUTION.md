# Veritas Handle Resolution

How the subs Query UI resolves a handle: the method, process, and trust model.

The Query UI uses the same **fabric relay resolution** path as the `fabric resolve` CLI — not local subs database state.

---

## UI → API

1. You enter a handle (e.g. `alice@space`) on `/ui/query` and click **Resolve**.
2. The browser POSTs to subs:

```javascript
const r = await fetch('/query', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ handle })
});
```

3. The API handler splits on commas (so `alice@space, bob@space` works) and calls `Operator::resolve`:

```rust
/// POST /query - Resolve one or more comma-separated handles via the fabric network
pub async fn resolve_handle(...) {
    let handles: Vec<&str> = body.handle.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    ...
    state.operator.resolve(&handles).await
}
```

The per-handle page (`handle.html`) uses the same `POST /query` endpoint for its "Query Fabric" action.

**Key files:** `subs/templates/query.html`, `subs/src/routes/query.rs`, `core/src/app.rs`

---

## Resolution Method: `fabric-resolver` + `libveritas`

Subs does **not** look up handles from its local SQLite. It queries the **fabric relay network** and cryptographically verifies the response.

### Step 1 — Pin Bitcoin Root Anchors (Trust Set)

Before querying relays, subs fetches **root anchors** from **spaced** via RPC and pins them as the trusted anchor set:

```rust
pub async fn resolve(&self, handles: &[&str]) -> anyhow::Result<Vec<ResolvedZone>> {
    let fabric = self.require_fabric()?;

    // Refresh anchors before querying so we have the latest chain state
    let anchors = self.require_rpc()?.get_root_anchors().await?;
    let sets = AnchorSets::from_anchors(anchors);
    _ = fabric.trust_from_set(sets.latest().unwrap())?;

    let rb = fabric.resolve_all(handles).await
```

This builds a `Veritas` verifier anchored to the latest Bitcoin block roots from your spaced node. All certificate messages returned by relays are checked against that chain state.

### Step 2 — Bootstrap Relay Pool

The `Fabric` client uses default relay seeds (unless overridden via `with_fabric_seeds`, e.g. in test-rig):

- `https://relay-cosmos.spacesprotocol.org`
- `https://relay-atlas.spacesprotocol.org`

It discovers/bootstrap peers from those seeds before querying.

### Step 3 — Nested Name Decomposition (`resolve_all`)

For handles like `hello.alice@space`, resolution is **iterative**:

```rust
pub async fn resolve_all(&self, handles: &[&str]) -> Result<ResolvedBatch> {
    let lookup = libveritas::names::Lookup::new(snames);
    ...
    let mut batch: Vec<SName> = lookup.start();   // first level, e.g. alice@space
    while !batch.is_empty() {
        let (verified, relay_url) = self.resolve_flat(&refs).await?;
        batch = lookup.advance(&verified.zones);    // follow aliases to deeper levels
        all_zones.extend(verified.zones);
    }
    lookup.expand_zones(&mut all_zones);            // expand canonical → full handle names
```

Deep names are broken into batches of 2-label lookups (`subname@space`), resolved level by level using alias mappings from prior zones.

### Step 4 — Query Relays (`GET /query`)

For each batch, `resolve_flat` groups handles by space and sends a relay query:

```rust
// Build GET query params
let q_param = q_parts.join(",");   // e.g. "@space,gxxxxxx@space"
...
.get(format!("{url}/query"))
.query(&[("q", &q_param)]);
```

Relays return a binary **`.spacemsg`** bundle containing:

- A **chain proof** (Bitcoin anchor + spaces/nums Merkle proofs)
- **Certificate bundles** per space (root cert + leaf certs)

Subs tries up to 4 relays (preferring those with freshest hints), falling back on failure.

### Step 5 — Cryptographic Verification (`libveritas`)

Each relay response is verified by `Veritas::verify_with_options`:

```rust
match self.veritas.lock().unwrap().verify_with_options(ctx, msg, options) {
    Ok(res) => { ... return Ok((res, url.clone())); }
    Err(e) => { last_err = Error::Verify(e); }
```

Verification includes:

1. **Anchor check** — message anchor matches trusted root anchors from spaced
2. **Chain proof check** — spaces/nums proofs tie certificates to the Bitcoin anchor
3. **Per-space bundle verification**:
   - **Root cert** for `@space`: may require a **ZK receipt** if commitment index > 0 (non-genesis)
   - **Leaf cert** for `handle@space`: inclusion proof (final) or exclusion proof + Schnorr signature (temp)
4. **Name expansion** — aliases resolved to full dotted names

If verification fails, the relay is marked failed and the next relay is tried. If all fail, subs returns an error to the UI.

### Step 6 — Badge Assignment

For each verified zone, subs assigns a UI badge based on sovereignty + trust:

```rust
let badge = fabric.badge_for(zone.sovereignty, &rb.roots);
ResolvedZone {
    badge: match badge {
        Badge::Orange => "orange",       // sovereign + trusted roots → "Verified"
        Badge::Unverified => "unverified",
        Badge::None => "none",
    }.to_string(),
    zone,
}
```

| Badge | Meaning in UI |
|-------|----------------|
| **orange** ("Verified") | Handle is **sovereign** and verified against your **pinned trusted** root anchors |
| **unverified** | Resolved against **observed** (newer) roots that differ from pinned trust |
| **none** | Dependent/pending sovereignty, or no applicable trust state |

---

## What the UI Displays

`renderZones()` shows for each returned zone:

- Handle, sovereignty badge, verification badge, anchor block height
- Script pubkey
- Commitment details (state root, prev root, block height, receipt hash) if present
- Delegate info if present
- Export links: `.spacemsg` binary and `anchors.json`

Export endpoints rebuild the same verified message bundle for offline verification (`export_message` uses the same fabric + anchor flow).

---

## Important Distinctions

| Source | What it resolves |
|--------|------------------|
| **Query UI / `POST /query`** | Published certificates on **fabric relays**, verified against **spaced root anchors** |
| **`GET /spaces/:space/handles/:name`** | **Local subs DB** only (staged/committed status) — anonymous, no fabric |
| **`fabric resolve` CLI** | Same fabric path as Query UI (subs wraps the same `fabric-resolver` library) |

---

## When Resolution Succeeds

Query UI resolution succeeds only if:

1. Certificates were **published to relays** (Publish step completed without relay rejection)
2. The **root cert includes required ZK receipt** (for non-genesis commitments)
3. **spaced RPC** is available (for root anchors)
4. At least one **relay** returns a verifiable message

A common failure mode: the root space (`@space`) resolves from chain anchor alone, but a subhandle (e.g. `gxxxxxx@space`) fails because the published cert chain on relays is incomplete or unverifiable — often due to an incomplete proving → broadcast → finalize → publish pipeline.
