# Building a registry for subs

A **registry** is the service that collects handle requests. `subs` stays
private — it holds wallet keys — and reaches out to the registry to collect
work.

```
┌──────────────────┐            ┌─────────┐
│     Registry     │ ◀───────── │  subs   │
└──────────────────┘   polls    └─────────┘
     (yours)                    (private)
```

**subs always initiates.** The registry never calls subs and never needs a
public address for it. Your registry is an HTTP server that subs polls.

## Scope

This document covers **only** the four endpoints subs calls. Everything else is
yours to decide and subs neither sees nor cares about: how requests get in, how
you authenticate or bill the people making them, how you store or display them.

[`examples/registry-server`](examples/registry-server) is a working
implementation you can run and read.

---

## The cycle

Each pass, per space, subs:

1. `GET /pending` — collect handles awaiting registration
2. stages them locally, validating and de-duplicating
3. `POST /ack` — report what it took
4. publishes certificates to the relay network

Steps 1–3 involve your registry; step 4 is internal to subs. `POST /committed`
fires separately and is **not** automatic.

---

## Authentication

All four endpoints take a single shared bearer token:

```
Authorization: Bearer <token>
```

Configure it in subs under **Settings → Registry Server → Auth Token**. Reject
anything without a valid token with **`401`**; subs surfaces `401`/`403`
distinctly, so an operator sees "registry rejected the auth token" rather than a
generic upstream error.

**`/health` is authenticated too.** subs' **Test** button probes it, so a
successful test proves both reachability and that the token works. If you need
an open liveness probe, expose it on a path subs doesn't use.

---

## Endpoints

### `GET /health`

Return `200` with any body.

### `GET /pending`

Return handles waiting to be staged. subs asks about **one space per request**:

```
GET /pending?space=@example
GET /pending?space=%233438-1-0
```

**Return only handles in that space.** A handle for a space this operator cannot
act on can never be staged, so it is never acked — serve it and it comes back
every cycle, forever.

The value is a canonical space label, percent-encoded; numeric spaces look like
`#3438-1-0`, so `#` arrives as `%23`.

Scope includes spaces **delegated to the operator's wallet but not yet
started** — staging the first handle adopts such a space automatically. Scope is
recomputed every cycle, so a new delegation takes effect without restarting
subs.

```json
{
  "handles": [
    { "handle": "alice@example", "script_pubkey": "5120aabb…" },
    { "handle": "bob@example",   "script_pubkey": "5120ccdd…" }
  ]
}
```

| Field | Type | Notes |
|---|---|---|
| `handle` | string | `name@space` format. Must parse — see [Handle validation](#handle-validation). |
| `script_pubkey` | string | Hex-encoded script pubkey of the owner's taproot address. |

Return `{"handles": []}` when there's nothing pending — not `404`.

**Any non-2xx aborts that space's sync**, including its ack, so nothing is
staged for it that pass. subs retries next cycle. The request times out after 10
seconds.

Returning already-staged handles is harmless — subs de-duplicates — but
filtering them keeps payloads small.

**Pagination** is optional: return the full set for the space, or cap it and let
the next cycle collect the rest. subs stages an entire response in one pass.
Capping per space starves nothing, since each request covers one space.

### `POST /ack`

Called once subs has decided each handle's fate.

```json
{
  "handles": [
    { "handle": "alice@example", "outcome": "staged" },
    { "handle": "bob@example",   "outcome": "already_committed_different_spk" }
  ]
}
```

Move these out of your pending set. Return `2xx`; the body is ignored.

**Every outcome is terminal** — a handle appears here only once settled, and
will not be offered again.

| Outcome | Meaning | What to tell the user |
|---|---|---|
| `staged` | Accepted; awaiting commitment | In progress |
| `already_staged_same_spk` | Already pending under the same owner | In progress — a duplicate request |
| `already_committed_same_spk` | Already registered to this owner | Already theirs |
| `already_staged_different_spk` | Another owner has it pending | **Cannot be fulfilled** |
| `already_committed_different_spk` | Another owner already holds it | **Cannot be fulfilled** |
| `invalid` | Not a parseable `name@space` handle | **Cannot be fulfilled** |

The last three can never succeed, however many times they are retried. If you
take payment, they are your refund signal.

**This must be idempotent.** If the ack fails, subs logs it and continues —
staging already happened on its side — so the next cycle re-pulls, re-stages (a
no-op) and re-acks. The flow self-heals, but only if re-acking succeeds rather
than errors.

#### Retryable failures

Some handles are neither staged nor settled: subs could not reach a decision,
typically because the space could not be loaded or the wallet could not operate
it. These are **deliberately absent** from the ack body. They stay in
`/pending`, and a later cycle picks them up once the operator's configuration is
fixed. Acking them would mark them done and lose them.

### `POST /committed`

```json
{ "root": "<commitment root hex>", "handles": ["alice@example", "bob@example"] }
```

Mark these committed and record the root. Return `2xx`.

**This is not automatic.** It fires only when someone calls
`POST /registry/notify` on subs with a `space` and `root`. If you need committed
state and aren't calling that, track it by watching the chain or by querying the
relay network for the handle's certificate.

---

## Delivery semantics

**At-least-once, never exactly-once.** A handle may be delivered more than once
— an ack that fails after subs staged the handle is the ordinary case. Design so
a repeat delivery is a no-op.

**subs is the source of truth for what is registered**, not your registry. An
ack of `staged` means subs accepted the handle into staging; it does not mean
the handle is committed on-chain. Only the commit notification means that.

---

## Handle validation

`handle` must parse as a spaces name in `name@space` form. One that doesn't is
acked `invalid` and settled immediately, so it won't linger in your queue — but
validate at intake anyway. A malformed handle that reaches `/pending` has
already cost a round trip and, if you charged for it, a refund.

---

## Configuring subs

In subs' **Settings → Registry Server**:

1. Set **Endpoint** to your base URL (e.g. `https://registry.example.com`)
2. Set **Auth Token** to the token your registry expects
3. Click **Test** — it probes `/health` with the token, so it fails on a bad
   token, not just an unreachable host
4. Optionally enable **Automatic Sync**

With automatic sync **off**, the cycle runs only on **Sync Now** (or
`POST /registry/sync`).

With it **on**, subs sleeps 2 seconds *after* each cycle finishes rather than
running on a fixed schedule, so cycles never overlap and a slow one simply
delays the next. It defaults to off because publishing broadcasts to the relay
network.

---

## Checklist

- [ ] `/health`, `/pending`, `/ack`, `/committed` all require the bearer token
- [ ] Missing or wrong token returns `401`
- [ ] `GET /pending` returns the documented shape, `{"handles": []}` when empty
- [ ] `GET /pending` filters on `?space=`
- [ ] `GET /pending` includes spaces delegated to the operator but not yet started
- [ ] `POST /ack` reads a per-handle `outcome` and is idempotent
- [ ] The `*_different_spk` and `invalid` outcomes are surfaced to the user, and refunded if paid
- [ ] Handles absent from an ack stay pending — they are retryable, not settled
- [ ] Handles are validated as `name@space` **at intake**
- [ ] Repeat delivery of the same handle is a no-op
- [ ] `POST /committed` implemented, if you need committed state