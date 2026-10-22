# RustyBlox

A high-performance PIVX blockchain explorer written in Rust. RustyBlox syncs
directly from PIVX Core's block files for speed, indexes into RocksDB, and
serves a drop-in Blockbook v2 API (REST + `/websocket` JSON protocol) and a
Vue frontend with a full analytics suite.

## Highlights

- **Fast**: full mainnet sync (12M+ transactions) in ~20 minutes on desktop
  hardware: blk-file parsing guided by Core's own LevelDB block index, with
  parallel file processing and batched RocksDB writes.
- **Core-accurate**: transaction classification, script/address extraction
  (P2PKH, P2SH, P2PK, cold-staking P2CS, exchange/EXM, zerocoin, sapling) and
  the emission schedule are implemented 1:1 against PIVX Core and verified
  against mainnet RPC. Address balances reconcile exactly with the reference
  Blockbook instance.
- **Blockbook v2 drop-in**: the `api/v2` surface implements the Blockbook
  contract (status envelope, paginated blocks, addresses/xpubs/UTXOs with live
  mempool overlays, fiat tickers, broadcast, the `/websocket` JSON protocol),
  so wallets and scripts already speaking Blockbook switch by changing the
  host.
- **Analytics engine**: precomputed daily series and snapshots: transaction
  volumes by type, fees, active/new addresses, staking participation derived
  from chain difficulty, rewards and APY, coin-days destroyed, HODL age bands,
  rich list with Gini/Nakamoto coefficients, cold-staking adoption, treasury
  payout history, orphan rates, block sizes and intervals.
- **Live**: WebSocket feeds for blocks, transactions and mempool; the monitor
  follows the chain tip via RPC after the initial file-based sync.
- **Governance**: proposal vote history that survives the node's own pruning,
  the funded set and payouts per superblock, and an optional vote log of
  everything pivxd accepts and rejects, with rejected votes traced to their
  masternode and grouped by funding transaction.

## Requirements

- Rust **1.88.0** (pinned by `rust-toolchain.toml`; rustup picks it up
  automatically)
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
cd frontend && npm ci && npm run build
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

Blockbook v2 drop-in (`/api/v2/...`):

| Endpoint | Purpose |
|---|---|
| `/api/v2` (also `/api`, `/api/status`) | Blockbook status envelope |
| `status`, `health` | compact sync state, liveness (PIVX extension) |
| `block/{height\|hash}` (paginated), `block-index/{h}`, `rawblock/{id}`, `block-detail/{h}` | blocks |
| `tx/{txid}`, `tx-specific/{txid}`, `/api/rawtx/{txid}`, `sendtx` (GET/POST) | transactions, broadcast |
| `address/{addr}`, `utxo/{addr}`, `xpub/{xpub}`, `balancehistory/{addr}` | addresses, wallets (live mempool overlays) |
| `mempool`, `mempool/{txid}` | unconfirmed transactions |
| `estimatefee/{blocks}` | fee estimate |
| `tickers`, `tickers-list`, `multi-tickers/` | fiat rates, sampled every 15 min (disable via `[price] fiat_sampler = false`) |
| `mncount`, `mnlist`, `budgetinfo`, `budgetprojection`, `budgetvotes/{name}`, `budgetvotecounts`, `budgetpayouts?sb=` | masternodes, governance |
| `govlog?hours=24\|168`, `govgroups?hours=24\|168` | budget vote log and identical-ballot groups (PIVX extension; see Governance vote log below) |
| `search/{query}` | height / hash / txid / address resolution |

The full reference (params, cache TTLs, money units) is served by the explorer
itself at `/#/api`.

Analytics (`/api/v2/analytics/...`): `transactions`, `staking`, `network`,
`supply`, `richlist`, `wealth-distribution`, `hodl`, `treasury`,
`coldstaking`, `snapshots`, all served from precomputed series in
milliseconds.

WebSockets: `/websocket` speaks the Blockbook JSON protocol (queries +
subscriptions); `/ws/blocks`, `/ws/transactions`, `/ws/mempool` are simple
one-way push channels used by the explorer UI.
Prometheus metrics: `/metrics` (block this path at the edge in production;
`ops/nginx-rustyblox.conf` does).

## Operations

- `target/release/db-marker get|clear <marker>`: inspect or reset sync
  phase markers (e.g. `clear address_index_complete` forces a full address
  index + analytics rebuild on next start; takes minutes, not hours).
- `target/release/db-query`: read-only diagnostics against a running
  instance (spent-record lookups, outpoint spender search).
- Re-syncing from scratch is cheap (~20 minutes): stop the explorer, remove
  the database directory from `[paths] db_path`, start again.
- `[paths] gov_db_path` (default `gov.db` beside `db_path`) holds governance vote
  history polled from the node. It cannot be rebuilt: the node overwrites
  votes and deletes expired proposals. Keep it out of any resync and back it
  up. If this store cannot be opened or written, the explorer keeps serving
  but governance history stops; check the log for "governance". A poll is
  skipped when the system clock is more than 2 h from the tip block time.

### Governance vote log (optional)

`target/release/rustyblox-govlog` runs on the pivxd host and records the
budget votes pivxd logs but RPC never shows: arrival time, votes it rejected
and why, and proposals it removed. The explorer shows them in the
Governance "Vote log" tab and at `/api/v2/govlog`. Without it the tab answers
503 and the rest of the explorer is unaffected; `/api/v2/govgroups`
(identical ballots) needs only `gov.db` and works either way.

1. Turn on budget logging in pivxd: `debug=mnbudget` in `pivx.conf` (kept
   across restarts), or `pivx-cli logging '["mnbudget"]'` at runtime (lost on
   restart).
2. Install `ops/rustyblox-govlog.service`. Run it as the user that owns
   `debug.log` (mode 0600). Set:
   - `--log`: pivxd's `debug.log` (testnet: under the network subdirectory).
   - `--db`: where the sidecar keeps its store, e.g. beside `gov.db`.
   - `--p2p`: pivxd's P2P address, not RPC; find it with `ss -ltnp | grep
     pivxd` (51472 by default, other ports on Blockbook layouts). Use an IP
     literal. Optional: without it, votes rejected as older or too soon show
     only their hash, which is all Core logs for them.
3. Give the explorer's user read access to the `--db` directory (shared group
   or the default 022 umask). The explorer opens it as a RocksDB secondary and
   creates `govlog-secondary/` beside `gov.db`.
4. Set `[governance] govlog_db_path` to the `--db` path and restart the
   explorer. A store that is missing at start is retried on each request, so
   start order does not matter. No reindex.
5. Check: the sidecar journal shows `govlog sidecar started`, with
   `recovered rejected votes` lines once `--p2p` reads votes back; the
   explorer log shows `vote log store (sidecar, read-only)`;
   `rustyblox-govlog report --db <path>` prints a summary;
   `curl localhost:<port>/api/v2/govlog?hours=24` returns JSON.

Upgrading: rebuild, copy the new `rustyblox-govlog` binary to where the unit
runs it from, restart the unit. The sidecar and explorer restart
independently.

What the data can and cannot show:
- Coverage starts at the earliest log line the sidecar read. Lines lost to
  logrotate (copytruncate) while it was down are recorded as gaps.
- With `--p2p`, a vote rejected as older or too soon is read back from
  pivxd's seen-vote map within ~10 s and stored with its masternode,
  proposal, choice and signed time. Read-back covers rejections from the
  last 24 h; pivxd drops entries when it rebuilds that map, and older rows
  stay hash-only.
- Evidence levels: `signed` (signature verified, signed at or after the
  stored vote within an hour), `seen` (these exact vote fields are already in
  `gov.db`; the hash excludes the signature, so a resend and an identical
  re-signing look the same), `older` (signature verified, signed before the
  stored vote: resent or backdated, the data cannot tell), `claimed` (names a
  collateral whose signature was never checked).
- Rejected votes are grouped by reason, proposal, choice and evidence, and by
  the transaction that funded each collateral. Collaterals funded by one
  transaction were created together; identical-ballot groups report how many
  funding transactions their members come from. Neither proves a common
  owner.
- Future-dated votes are refused before pivxd checks their signature, so
  the collateral they name is a claim only.

Troubleshooting:
- `/api/v2/govlog` 503 `vote log unavailable`: `detail` names the cause
  (path not set, store missing, permissions).
- Sidecar `vote recovery: node unreachable over P2P`: wrong `--p2p` port or
  pivxd down; it retries every 60 s and logs once per outage.
- No vote lines at all: check that `mnbudget` logging is on (`pivx-cli
  logging`); outside voting campaigns votes arrive a few per hour.

## Repository layout

```
src/             backend (sync pipeline, parsers, API, analytics)
frontend/        the shipped Vue 3 frontend
frontend-legacy/ previous UI (kept, no longer the default)
ops/             Prometheus/Grafana monitoring, nginx edge config, sidecar unit
tools/           diagnostics, maintenance, and the governance log sidecar
```

## License

MIT, see [LICENSE](LICENSE).
