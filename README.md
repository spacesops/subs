<p align="center">
  <h2>subs</h2>
  <p>
    🟠 <i>create, prove & verify Bitcoin handles off-chain</i>
    <br/>
   </p>
</p>

<img src="https://github.com/spacesprotocol/subs/blob/main/screenshot.png?raw=true">

## How it works

**Basic principle**

1. Add handles to a Merkle tree & commit the 32-byte root to Bitcoin.

2. New handles must prove non-existence in the previous root(s).

3. Subs compresses these proofs: STARK or SNARK → root cert.

4. Owners get an inclusion proof → leaf cert.

5. Certificates are non-revocable: once bound to a script pubkey, it’s yours.

Note: Only the tree root gets committed to Bitcoin - certificates remain off-chain (low footprint!).

See https://spacesprotocol.org/paper


### Who gets to be the operator?

Operators are chosen via permissionless auctions on Bitcoin. They manage top-level spaces: https://explorer.spacesprotocol.org


## Installation

**Prereq (RISC Zero [toolchain](https://dev.risczero.com/api/zkvm/install)):**

```
curl -L https://risczero.com/install | bash
rzup install
```

Install subs:

```
git clone https://github.com/spacesprotocol/subs && cd subs
cargo install --path subs
cargo install --path prover
```

For operators, use `--features cuda` on `subs-prover` for nvidia machines to enable GPU acceleration.

## Configuration

Each binary accepts the same settings via **CLI flags**, **environment variables**, or a **`.env` file** in the current working directory. Command-line flags override environment variables.

Load a custom env file path with:

- `subs`: `SUBS_ENV_FILE=/path/to/subs.env`
- `subs-prover`: `SUBS_PROVER_ENV_FILE=/path/to/prover.env`
- `registry-server`: `REGISTRY_SERVER_ENV_FILE=/path/to/registry.env`

See [.env.example](.env.example) for a full template.

### `subs`

| Variable | CLI flag | Description |
|----------|----------|-------------|
| `SUBS_PORT` | `--port` | HTTP server port (default `7777`) |
| `SUBS_DATA_DIR` | `--data-dir` | Data directory (default `./data`) |
| `SUBS_WALLET` | `--wallet` | Wallet name for signing |
| `SUBS_SPACED_RPC_URL` | `--rpc-url` | `spaced` RPC URL |
| `SUBS_SPACED_RPC_USER` | `--rpc-user` | `spaced` RPC username |
| `SUBS_SPACED_RPC_PASSWORD` | `--rpc-password` | `spaced` RPC password |
| `SUBS_SPACED_RPC_COOKIE` | `--rpc-cookie` | `spaced` RPC cookie file path |
| `SUBS_PROVER_ENDPOINT` | *(Settings UI)* | Prover URL written to `config.db` at startup |
| `SUBS_REGISTRY_ENDPOINT` | *(Settings UI)* | Registry URL written to `config.db` at startup |
| `SUBS_TEST_RIG` | `--test-rig` | Enable test rig (`1`, `true`, `yes`) |
| `SUBS_TEST_RIG_DIR` | `--test-rig-dir` | Test rig data directory |

### `subs-prover`

| Variable | CLI flag | Description |
|----------|----------|-------------|
| `SUBS_PROVER_SERVER` | `--server` | Run as HTTP server (`1`, `true`, `yes`) |
| `SUBS_PROVER_PORT` | `--server-port` | Server port (default `8888`) |
| `SUBS_PROVER_INPUT` | `-i` / `--input` | Input file (prove/compress subcommands) |
| `SUBS_PROVER_OUTPUT` | `-o` / `--output` | Output file (prove/compress subcommands) |
| `SUBS_PROVER_BENCH_EXISTING` | `--existing` | Bench: existing handle count |
| `SUBS_PROVER_BENCH_INSERT` | `--insert` | Bench: handles to insert |

### `registry-server`

| Variable | CLI flag | Description |
|----------|----------|-------------|
| `REGISTRY_SERVER_PORT` | `--port` | HTTP server port (default `8080`) |

### Examples

Using `export`:

```bash
export SUBS_SPACED_RPC_URL=http://127.0.0.1:7225
export SUBS_WALLET=my-wallet
export SUBS_DATA_DIR=./data
export SUBS_PROVER_ENDPOINT=http://127.0.0.1:8888
subs
```

Using a `.env` file:

```bash
cp .env.example .env
# edit .env, then:
subs
```

```bash
# subs-prover from .env
export SUBS_PROVER_SERVER=1
export SUBS_PROVER_PORT=8888
subs-prover
```

```bash
# registry-server
export REGISTRY_SERVER_PORT=8080
registry-server
```

Log verbosity uses the standard `RUST_LOG` variable (e.g. `RUST_LOG=subs=debug,tower_http=debug`).

On startup, each binary prints its **effective configuration** to the console with the **origin** of each value: `param` (CLI flag), `environment` (`export`), `.env` (dotenv file), or `default`. Sensitive values (passwords) are shown as `(set)` without revealing the secret. Example:

```
subs configuration:
  (loaded env file: .env)
  port = 7777 (.env)
  data_dir = ./datamad (.env)
  wallet = mad (environment)
  rpc_url = http://127.0.0.1:7225 (.env)
  rpc_password = (set) (.env)
  server_url = http://127.0.0.1:7777 (derived from port)
```

CLI flags override environment variables; process environment overrides `.env` for the same key.

## Usage

### 1. Start the prover server

```
subs-prover --server --server-port 8888
```

### 2. Start subs

Point it at your `spaced` RPC and the wallet that will be used to operate spaces:

```
subs \
  --rpc-url http://127.0.0.1:7225 \
  --wallet my-wallet \
  --data-dir ./data \
  --port 7777
```

Then open http://localhost:7777 in a browser. Under **Settings**, set the prover URL to `http://127.0.0.1:8888` so subs can dispatch proofs.

From the UI you can stage handles, run local commits, broadcast on-chain commitments, and issue / export certificates.

### Test rig (local dev)

The `test-rig` feature spins up a fresh regtest `bitcoind` + `spaced` automatically, so no external setup is needed. Useful for hacking on subs without a live node.

```
cargo install --path subs --features test-rig
subs --test-rig --test-rig-dir ./testrig-data
```

`--wallet` and `--rpc-url` are not required in this mode. The rig persists chain data in `--test-rig-dir` so restarts keep state. The UI exposes an RPC console and a "mine N blocks" helper for driving the chain forward.

## License

Apache 2.0
