# RustyBlox

A high-performance PIVX blockchain explorer written in Rust. RustyBlox syncs
directly from PIVX Core's block files for speed, indexes into RocksDB, and
serves a Blockbook-compatible REST API, WebSocket feeds, and a Vue frontend
with a full analytics suite.

## Highlights

- **Fast**: full mainnet sync (12M+ transactions) in ~20 minutes on desktop
  hardware — blk-file parsing guided by Core's own LevelDB block index, with
  parallel file processing and batched RocksDB writes.
- **Core-accurate**: transaction classification, script/address extraction
  (P2PKH, P2SH, P2PK, cold-staking P2CS, exchange/EXM, zerocoin, sapling) and
  the emission schedule are implemented 1:1 against PIVX Core and verified
  against mainnet RPC. Address balances reconcile exactly with the reference
  Blockbook instance.
- **Blockbook-compatible API**: drop-in `api/v2` endpoints for wallets
  (addresses, xpubs, UTXOs, transactions, broadcast).
- **Analytics engine**: precomputed daily series and snapshots — transaction
  volumes by type, fees, active/new addresses, staking participation derived
  from chain difficulty, rewards and APY, coin-days destroyed, HODL age bands,
  rich list with Gini/Nakamoto coefficients, cold-staking adoption, treasury
  payout history, orphan rates, block sizes and intervals.
- **Live**: WebSocket feeds for blocks, transactions and mempool; the monitor
  follows the chain tip via RPC after the initial file-based sync.

## Requirements

- Rust **1.85+** (the lockfile uses edition-2024 dependencies)
- A synced **PIVX Core** node on the same machine, with RPC enabled and
  readable `blocks/` data (blk files + LevelDB block index)
- Clang, CMake (for RocksDB/LevelDB bindings)
- Node 20.19+ (only if rebuilding the frontend)

## Quick start

```bash
# 1. Configure
cp config.toml.example config.toml
#    set [rpc] host/user/pass to match your pivx.conf,
#    and [paths] blk_dir to your PIVX blocks directory

# 2. Build and run (initial sync, then serves the API + frontend)
cargo build --release
./target/release/rustyblox

# 3. Build the frontend once (served by the backend on the same port)
cd frontend-legacy && npm ci && npm run build
```

The explorer listens on `http://localhost:3005` (configurable via
`[server]` in `config.toml`) and serves both the API and the built frontend.

## Docker

```bash
cp config.toml.example config.toml   # edit RPC credentials first
docker compose up -d                  # backend + frontend + monitoring
```

`docker-compose.prod.yml` adds resource limits, log rotation, a
localhost-bound API (nginx is the public entry point) and localhost-bound
Prometheus/Grafana. See `ops/` for the native monitoring stack.

## API overview

Blockbook-compatible core (`/api/v2/...`):

| Endpoint | Purpose |
|---|---|
| `status`, `health` | sync state, liveness |
| `block/{height}`, `block-index/{h}`, `block-detail/{h}` | blocks |
| `tx/{txid}`, `sendtx` (GET/POST) | transactions, broadcast |
| `address/{addr}`, `utxo/{addr}`, `xpub/{xpub}` | addresses, wallets |
| `mempool`, `mempool/{txid}` | unconfirmed transactions |
| `mncount`, `mnlist`, `budgetinfo`, `budgetprojection`, `budgetvotes/{name}` | masternodes, governance |
| `search/{query}` | height / hash / txid / address resolution |

Analytics (`/api/v2/analytics/...`): `transactions`, `staking`, `network`,
`supply`, `richlist`, `wealth-distribution`, `hodl`, `treasury`,
`coldstaking`, `snapshots` — all served from precomputed series in
milliseconds.

WebSockets: `/ws/blocks`, `/ws/transactions`, `/ws/mempool`.
Prometheus metrics: `/metrics` (keep this port private in production).

## Operations

- `target/release/db-marker get|clear <marker>` — inspect or reset sync
  phase markers (e.g. `clear address_index_complete` forces a full address
  index + analytics rebuild on next start; takes minutes, not hours).
- `target/release/db-query` — read-only diagnostics against a running
  instance (spent-record lookups, outpoint spender search).
- Re-syncing from scratch is cheap (~20 minutes): stop the explorer, remove
  the database directory from `[paths] db_path`, start again.

## Repository layout

```
src/             backend (sync pipeline, parsers, API, analytics)
frontend-legacy/ the shipped Vue 3 frontend
frontend-vue/    alternative UI (complete, not currently served)
ops/             Prometheus/Grafana monitoring
tools/           diagnostics (e.g. reference-explorer comparison)
```

## License

MIT — see [LICENSE](LICENSE).
