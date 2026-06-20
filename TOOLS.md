# Maintenance & Diagnostic Tools

Production diagnostic, maintenance, and utility tools for the explorer, organized
under `tools/<category>/` and declared as `[[bin]]` in the root `Cargo.toml`:

```bash
cargo run --release --bin <tool-name> -- [args]      # always use --release
```

**DB path:** every tool resolves the RocksDB path from **`config.toml`**
(`paths.db_path`), so run each from the repo root (where `config.toml` lives) — no
hardcoded paths. To point a tool at a different DB without editing config, set
`DB_PATH`:

```bash
DB_PATH=/srv/rustyblox/data/blocks.db cargo run --release --bin <tool>
```

**Safety:**
- **Read-only** tools open RocksDB as a *secondary* instance — safe to run **while
  the explorer is running**.
- **Read-write (RW)** tools take the RocksDB lock — **stop the backend first**, and
  back up the DB before maintenance.

---

## Live daily-analytics ops (`live_analytics` feature)

The live updater keeps the `analytics_tx_day:<date>` blobs current as the monitor
advances. Gated by `analytics_live_ready` (set only by a full enrich) + the
`analytics_live_height` watermark.

### `build-orphan-index` — rebuild the orphan index *(RW, backend stopped)*
Builds the persistent orphan index (`orphanseen:` / `orphancount:`) from the
`blocks` CF + blk-tail's `tail_blocks`, then rewrites every `analytics_tx_day`
blob's `orphan_blocks` from the persistent count. **Run once** after first enabling
the feature (existing orphans are unmarked until this runs). Idempotent. Afterwards
the live tail-only path maintains it automatically.

### `reset-live-watermark [days=2]` — repair a stale/partial live window *(RW, backend stopped)*
Deletes the last `N` calendar days' `analytics_tx_day` blobs + their difficulty/
interval side keys, prunes their treasury, and resets `analytics_live_height` so
Lane I rebuilds those days completely on the next tick. Use when the watermark
ended up ahead of actual coverage (e.g. a day shows too few `blocks` and drops off
the chart).

### `live-debug` — dump live-analytics state *(read-only)*
Prints `sync_height`, the live watermark, the ready/complete flags, the
`analytics_tx_days` index tail, and recent day blobs. First stop when the analytics
look frozen or wrong.

---

## Address index & balances

### `rebuild-address-index` — clear + rebuild the address index *(RW, backend stopped, ~10–30 min)*
Clears the old index and rebuilds it via the canonical `enrich_addresses` module
(Pass 1 finds spent outputs, Pass 2 indexes only unspent), so it stays consistent
with normal sync. Use after an enrichment-logic change, index corruption, or
balance drift across many addresses.

### `import-chainstate` — verify balances vs PIVX Core *(RW)*
Imports and verifies UTXO balances from Core's chainstate LevelDB — the
ground-truth balance check.

### `diagnose-address [ADDRESS]` — deep address analysis *(read-only)*
Dumps an address's UTXOs, tx history, and balance, with an external-explorer
comparison. Use to troubleshoot a specific balance discrepancy.

---

## Heights & orphans

### `revalidate-heights` / `validate-all-tx-heights` — re-validate tx heights *(RW)*
Re-validate every transaction's height against the canonical chain (fixes stale /
height-0 / orphaned heights after a reorg or interrupted sync).

### `mark-orphaned-zero-height` — mark height=0 txs as orphaned (-1) *(RW)*

### `count-orphans` / `analyze-orphans` — orphan diagnostics *(read-only)*
Count orphaned blocks/transactions and assess reorg impact.

---

## Sync validation

### `validate-sync` — comprehensive sync validation *(read-only)*
Verifies tx counts, block-height coverage, address-index completeness, orphan
counts, and samples addresses against external explorers. Run after the initial
sync, after any maintenance tool, and periodically as a production health check.

---

## Diagnostics *(read-only, safe while running)*

| Tool | Purpose |
|---|---|
| `check-db` | DB integrity / height + chain-state sanity (first-line troubleshooting) |
| `db-query <spent\|find-spender> <txid> <vout>` | look up a spent-UTXO undo record |
| `db-marker <get\|clear> <marker>` / `db-marker set-height <marker> <i32>` | read/write a `chain_state` marker (e.g. `address_index_complete`) |
| `inspect-leveldb` | inspect PIVX Core's block-index LevelDB (cross-validation) |

Additional ad-hoc diagnostics live in the nested `tools/diagnostics/` sub-crate
(`verify-txid-format`, `check-txid-format`, `diagnose-tx`, `find-xpub-address`) —
run them from there: `cd tools/diagnostics && cargo run --release --bin <name>`.

---

## Migration & export

### `backfill-offsets` — backfill offset mappings from Core LevelDB *(RW, one-time)*
Reads offset mappings from PIVX's block index into the RocksDB `blocks` CF.
Prerequisite: Core synced with the block index available. Consider a full resync
as an alternative.

### `export-txids` — export the txid set *(read-only)*
`cargo run --release --bin export-txids > txids.txt`

---

## Testing

### `test-parser` — transaction parser regression test *(read-only)*
Exercises coinbase / regular / Sapling-shielded / multi-IO / edge-case
deserialization. Run after parser changes.

---

## Re-enabling a flag-gated feature

Both new features are **off by default** in `config.toml`:

- `sync.live_analytics` — the live daily-analytics updater. Activates only after a
  full enrich sets `analytics_live_ready`. Run `build-orphan-index` once for the
  orphan metric. Use `sync.live_analytics_shadow = true` +
  `sync.live_analytics_shadow_validate_days = N` to validate against the enrich
  before promoting (`shadow = false`).
- `sync.live_tail_blkfiles` — blk-file tailing for live orphan capture (feeds the
  orphan index via the private `tail_blocks` CF).

---

## Troubleshooting

- **Database connection errors:** confirm `paths.db_path` in `config.toml`, or pass
  `DB_PATH=…`. Read-only tools can run while the backend is up; RW tools cannot.
- **Slow:** always build/run with `--release` (debug is ~10× slower).
- **Won't build:** `cargo clean && cargo build --release --bin <tool>`.
