# libveritas & Spaces protocol crate upgrade (reference)

Upgrade applied to align subs with current crates.io releases for offline verification and Spaces protocol types. Use this note when rebasing, debugging resolve/badge behavior, or bumping these deps again.

## Version pins (`Cargo.toml` workspace)

| Crate | Previous | Updated |
|--------|----------|---------|
| `libveritas` | `=0.1.2` | **`=0.3.0`** (`elf` feature unchanged) |
| `libveritas_testutil` | `=0.1.2` | **`=0.3.0`** |
| `libveritas_zk` | `=0.1.1` | unchanged |
| `spaces_protocol` | `0.1` | **`=0.2.1`** (`std` feature) |
| `spaces_nums` | `0.1` | **`=0.2.1`** |
| `spaces_client` | `0.1` | **`=0.2.1`** |
| `spaces_wallet` | `0.1` | **`=0.2.1`** |
| `fabric-resolver` (`fabric`) | `=0.1.2` | **git rev `5efa1cb3…`** (see note below) |

`spacedb` and `relay` (git) were not changed in this pass.

### libveritas 0.3.0 and fabric

Published `fabric-resolver` **≥0.2.4** requires `libveritas` **≥0.3.1**; **0.2.6+** requires **0.3.3**. To pin **`libveritas = 0.3.0`**, `fabric` is taken from certrelay commit `5efa1cb3371b87dafcba5b69820aa58b83e1edb4` (`libveritas = "0.3"` in that workspace). To use crates.io fabric again, bump libveritas to at least the version that fabric release requires.

## Build

```bash
cargo build --release
```

Release build succeeds after the code adjustments below.

## Interface changes addressed in subs

### `libveritas::Zone`

- New field: **`anchor_hash`** (`[u8; 32]` / protocol hash).
- Temp zones built for wallet signing in `core/src/app.rs` set `anchor_hash: [0u8; 32]` (same pattern as libveritas tests).

### `fabric-resolver` (`Fabric`)

| Old (0.1.x) | New (0.2.x) |
|-------------|-------------|
| `resolve_all(...)` → batch with `.zones` and `.roots` | `resolve_all(...)` → **`Vec<Zone>`** directly |
| `fabric.badge_for(zone.sovereignty, &rb.roots)` | **`fabric.badge(&zone)`** (uses `zone.anchor_hash`) |

Updated in: `core/src/app.rs` (`Operator::resolve`).

### Deprecations (warnings only)

`spaces_protocol::bitcoin::FeeRate::from_sat_per_vb_unchecked` is deprecated in 0.2.x. Prefer `from_sat_per_vb_u32` in:

- `subs/src/routes/commits.rs`
- `core/tests/full_e2e.rs`
- `core/tests/test_vectors.rs`

Not required for a successful build; optional cleanup.

## Test changes

### `core/src/core.rs` — `test_batch_to_zk_input_format`

The test previously assumed a **96-byte** ZK batch layout (space + subspace + raw script pubkey hashes). `Batch::to_zk_input()` actually emits **64 bytes per entry**:

1. SHA256(subspace `SLabel` bytes)
2. SHA256(`HandleOut { name, spk }.to_vec()`)

This matches `BatchReader` and `test_batch_reader_roundtrip`. The test was updated to assert `[0..32]` and `[32..64]` accordingly.

### Integration tests

- **`core/tests/fabric.rs`** — Still requires a certrelay listening on `http://127.0.0.1:7779`; failure is environment-related, not the crate bump.
- **`spaces_testutil`** (git `subspaces` branch) may still pull older `spaces_protocol` 0.1.x for the test rig; workspace crates use 0.2.1 from crates.io.

## Docs drift

`VERITAS_RESOLUTION.md` may still describe `badge_for` and batch `.zones`; update that doc separately if you rely on it for onboarding.

## Quick verification

```bash
cargo test --release -p subs-core --lib
```

All 12 subs-core unit tests should pass.
