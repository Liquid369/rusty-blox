# Maintenance & Diagnostic Tools

All tools live under `tools/<category>/` and are declared as `[[bin]]` in the root
`Cargo.toml`, so each runs with:

```bash
cargo run --release --bin <tool-name> -- [args]
```

Two rules of thumb:
- **Read-only** tools open RocksDB as a *secondary* instance and are safe to run
  **while the explorer is running**.
- **Read-write** tools take the RocksDB lock and require the **backend stopped**.

Every tool resolves the RocksDB path from **`config.toml`** (`paths.db_path`), so
run each from the repo root (where `config.toml` lives) — no hardcoded paths. To
point a tool at a different DB without editing config, set the `DB_PATH` env var:
`DB_PATH=/srv/rustyblox/data/blocks.db cargo run --release --bin <tool>`.

---

## Live daily-analytics (`live_analytics` feature)

The live updater keeps `analytics_tx_day:<date>` blobs current as the monitor
advances. The metric is gated by `analytics_live_ready` (set only by a full
enrich) and tracked by the `analytics_live_height` watermark.

### `build-orphan-index` — rebuild the orphan index *(RW, backend stopped)*
```bash
cargo run --release --bin build-orphan-index
```
Builds the persistent orphan index (`orphanseen:<hash>` / `orphancount:<date>`)
from the `blocks` CF + blk-tail's `tail_blocks`, then rewrites every
`analytics_tx_day` blob's `orphan_blocks` from the persistent count. **Run once**
after first enabling the orphan-index feature (the existing orphans are unmarked
until this runs). Idempotent — re-running is a no-op. After this the live
tail-only path maintains the index automatically.

### `reset-live-watermark [days=2]` — repair a stale/partial live window *(RW, backend stopped)*
```bash
cargo run --release --bin reset-live-watermark 2
```
Deletes the last `N` calendar days' `analytics_tx_day` blobs + their difficulty/
interval side keys, prunes their treasury, and resets `analytics_live_height` to
before them, so Lane I rebuilds those days completely on the next tick. Use when
the live watermark ended up ahead of the actual analytics coverage (e.g. a day
shows too few `blocks` and is dropped from the chart).

### `live-debug` — dump live-analytics state *(read-only, safe while running)*
```bash
cargo run --release --bin live-debug
```
Prints `sync_height`, the live watermark, the ready/complete flags, the
`analytics_tx_days` index tail, and recent day blobs (blocks / tx_count / fees /
orphan). First stop when the analytics look frozen or wrong.

---

## Address index & balances

### `rebuild-address-index` — clear + rebuild the address index *(RW, backend stopped)*
Removes the old address index and rebuilds it with proper UTXO tracking. Use after
an enrichment-logic change or suspected balance drift.

### `import-chainstate` — verify balances vs PIVX Core *(RW)*
Imports and verifies UTXO balances from PIVX Core's chainstate LevelDB — the
ground-truth balance check.

---

## Heights & orphans

### `revalidate-heights` / `validate-all-tx-heights` — re-validate tx heights *(RW)*
Re-validate every transaction's height against the canonical chain (fixes stale /
height-0 / orphaned heights after a reorg or interrupted sync).

### `mark-orphaned-zero-height` — mark height=0 txs as orphaned (-1) *(RW)*

### `count-orphans` / `analyze_orphans` — orphan diagnostics *(read-only)*

---

## Diagnostics (read-only, safe while running)

| Tool | Purpose |
|---|---|
| `check-db` | general DB health / key-space sanity |
| `db-query spent <txid> <vout>` | look up a spent-UTXO undo record |
| `db-marker get\|set\|clear <marker>` | read/write a `chain_state` marker (e.g. `address_index_complete`) |
| `diagnose-address <addr>` | dump an address's UTXOs / balance / tx history |
| `validate-sync` | end-to-end sync validation vs the node |
| `inspect-leveldb` | inspect the PIVX Core LevelDB |
| `export-txids` | export the txid set |

---

## Re-enabling a flag-gated feature

Both new features are **off by default** in `config.toml`:

- `sync.live_analytics` — the live daily-analytics updater. Activates only after a
  full enrich sets `analytics_live_ready`. Run `build-orphan-index` once for the
  orphan metric. Use `sync.live_analytics_shadow = true` +
  `sync.live_analytics_shadow_validate_days = N` to validate against the enrich
  before promoting (`shadow = false`).
- `sync.live_tail_blkfiles` — blk-file tailing for live orphan capture (feeds the
  orphan index via `tail_blocks`).
