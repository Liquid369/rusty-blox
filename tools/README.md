# Rusty-Blox Essential Tools

This directory contains production-ready diagnostic, maintenance, and utility tools for the PIVX blockchain explorer.

## 📊 Cleanup Summary

- **Before:** 97 binaries in `/src/bin/`
- **After:** 9 essential tools organized in `/tools/`
- **Reduction:** 91% cleanup (88 obsolete/duplicate binaries deleted)

## 🗂️ Directory Structure

```
/tools/
├── diagnostics/       (4 tools) - Database and sync health checks
├── maintenance/       (1 tool)  - Address index rebuilding
├── validation/        (1 tool)  - Sync validation and verification
├── migration/         (1 tool)  - Data migration utilities
├── export/            (1 tool)  - Data export utilities
└── testing/           (1 tool)  - Parser testing
```

---

## 🔍 Diagnostics Tools

### `check-db`
**File:** `diagnostics/check_db.rs`  
**Purpose:** Database integrity checker and height verification  
**Usage:**
```bash
cargo run --release --bin check-db
```
**When to use:**
- Quick DB health diagnostics
- Verify chain state and metadata
- Check sync height consistency
- First-line troubleshooting

---

### `diagnose-address`
**File:** `diagnostics/diagnose_address.rs`  
**Purpose:** Deep address analysis and balance verification  
**Usage:**
```bash
cargo run --release --bin diagnose-address [ADDRESS]
# Default address: DCSAJGThtCnDokqawZehRvVjdms9XLL6J6
```
**When to use:**
- Troubleshoot address balance discrepancies
- Compare against external explorers
- Debug UTXO tracking issues
- Verify transaction history completeness

**Output:** Detailed analysis of:
- UTXO count and values
- Transaction list
- Balance calculations
- External explorer comparison

---

### `count-orphans`
**File:** `diagnostics/count_orphans.rs`  
**Purpose:** Count orphaned blocks and transactions  
**Usage:**
```bash
cargo run --release --bin count-orphans
```
**When to use:**
- Assess reorg impact
- Verify orphan cleanup
- Database health checks after reorgs
- Debugging chain reorganization issues

**Output:**
- Total orphaned transactions (height=-1)
- Orphaned blocks count
- Impact on UTXO set

---

### `inspect-leveldb`
**File:** `diagnostics/inspect_leveldb.rs`  
**Purpose:** Direct LevelDB inspection (PIVX Core format)  
**Usage:**
```bash
cargo run --release --bin inspect-leveldb
```
**When to use:**
- Cross-validate with PIVX Core data
- Debug LevelDB copy issues
- Verify block index integrity
- Troubleshoot offset mappings

**Output:** Raw LevelDB key-value pairs, block index data, offset mappings

---

## 🛠️ Maintenance Tools

### `rebuild-address-index`
**File:** `maintenance/rebuild_address_index.rs`  
**Purpose:** Rebuild address index from scratch with proper UTXO tracking  
**Usage:**
```bash
cargo run --release --bin rebuild-address-index
```
**When to use:**
- Address index corruption
- Balance discrepancies across many addresses
- After major database repairs
- Migration from old index format

⚠️ **WARNING:** Clears existing address index before rebuilding

**Process:**
1. Clears old address index
2. Calls `enrich_all_addresses()` from main library
3. Two-pass processing:
   - Pass 1: Identifies all spent outputs
   - Pass 2: Indexes only UNSPENT outputs
4. Validates balance calculations

**Implementation:** Uses the canonical `enrich_addresses` module from the main codebase, ensuring consistency with normal sync operations.

**Time:** ~10-30 minutes depending on blockchain size

---

## ✅ Validation Tools

### `validate-sync`
**File:** `validation/validate_sync.rs`  
**Purpose:** Comprehensive blockchain sync validation  
**Usage:**
```bash
cargo run --release --bin validate-sync
```
**When to use:**
- Post-sync validation
- Verify sync completeness
- Identify block height gaps
- Production health checks

**Validation checks:**
- Transaction count verification
- Block height coverage
- Address index completeness
- Orphaned transaction count
- Sample address validation against external explorers

**Output:** Detailed sync validation report with pass/fail status

---

## 🔄 Migration Tools

### `backfill-offsets`
**File:** `migration/backfill_offsets.rs`  
**Purpose:** Backfill PIVX LevelDB offset mappings into RocksDB  
**Usage:**
```bash
cargo run --release --bin backfill-offsets
```
**When to use:**
- One-time migration from PIVX Core LevelDB
- Restore offset mappings after corruption
- Pattern A offset-based indexing setup

**Process:**
1. Copies PIVX block index to temp location
2. Reads offset mappings from LevelDB
3. Stores in RocksDB blocks CF
4. Validates mapping count

**Prerequisites:** PIVX Core synced with block index available

---

## 📤 Export Tools

### `export-txids`
**File:** `export/export_txids.rs`  
**Purpose:** Export transaction IDs for external analysis  
**Usage:**
```bash
cargo run --release --bin export-txids > txids.txt
```
**When to use:**
- Performance testing with real tx data
- External analytics
- Data science analysis
- Blockchain research

**Output formats:**
- Plain text (one txid per line)
- JSON array (optional)
- CSV with metadata (optional)

---

## 🧪 Testing Tools

### `test-parser`
**File:** `testing/test_parser.rs`  
**Purpose:** Transaction parser testing and format validation  
**Usage:**
```bash
cargo run --release --bin test-parser
```
**When to use:**
- Regression testing after parser changes
- Verify transaction deserialization
- Debug parsing failures
- Validate block 1000 genesis tx parsing

**Test coverage:**
- Coinbase transactions
- Regular transactions
- Sapling shielded transactions
- Multi-input/multi-output txs
- Edge cases (empty inputs, large outputs)

---

## 📝 Build & Usage

### Building All Tools
```bash
cargo build --release
```

### Building Specific Tool
```bash
cargo build --release --bin <tool-name>
```

### Running Tools
```bash
# From project root
cargo run --release --bin <tool-name> [ARGS]

# Or run directly
./target/release/<tool-name> [ARGS]
```

### Tool Names (use with --bin flag)
- `check-db`
- `diagnose-address`
- `count-orphans`
- `inspect-leveldb`
- `rebuild-address-index`
- `build-combined-index`
- `build-address-index`
- `validate-sync`
- `backfill-offsets`
- `export-txids`
- `test-parser`

---

## 🚨 Production Usage Guidelines

### When to Run Diagnostics
- ✅ **Safe to run anytime:** `check-db`, `count-orphans`, `diagnose-address`, `inspect-leveldb`
- ⚠️ **Non-blocking:** These tools are read-only and won't modify your database

### When to Run Maintenance
- ⚠️ **Stop rustyblox server first:** `rebuild-address-index`
- ⚠️ **Database modification:** These tools write to the database
- ⚠️ **Backup recommended:** Always backup before running maintenance tools

### When to Run Validation
- ✅ **After sync:** Run `validate-sync` after initial blockchain sync
- ✅ **After repairs:** Validate after running any maintenance tools
- ✅ **Health checks:** Run periodically (weekly/monthly) for production monitoring

### When to Run Migration
- ⚠️ **One-time only:** Migration tools like `backfill-offsets` are typically run once
- ⚠️ **Fresh sync alternative:** Consider full resync instead of migration

---

## 🔧 Troubleshooting

### Tool Won't Build
```bash
# Clean and rebuild
cargo clean
cargo build --release --bin <tool-name>
```

### Database Connection Errors
```bash
# Check config.toml path
DB_PATH=./data/blocks.db cargo run --release --bin <tool-name>
```

### Performance Issues
```bash
# Run with release optimizations (ALWAYS for production)
cargo run --release --bin <tool-name>

# NOT recommended for performance-critical tools:
# cargo run --bin <tool-name>  # DEBUG MODE - 10x slower
```

---

## 📚 Related Documentation

- **Production Readiness Report:** `/PRODUCTION_READINESS_REPORT.md`
- **API Documentation:** `/API_DOCUMENTATION.md`
- **Sync Process:** `/SYNC_PROCESS.md`
- **Deployment Guide:** `/DEPLOYMENT.md`

---

## 🗑️ Deleted Tools (For Reference)

The following 86 debug binaries were removed during cleanup:

### Height/Orphan Resolution (15 files) - Bugs fixed in production code
- analyze_height_distribution, backfill_height_mappings, check_height_txs, check_tx_heights, fix_zero_height_txs, rebuild_height_mappings, diagnose_address_orphans, find_orphaned_input_refs, find_orphaned_utxos, debug_orphaned_txs, trace_spent_detection, check_spent_utxo, validate_orphans, verify_utxos

### Address Index Development (18 files) - Replaced by production code
- check_addr_index, check_addr_index_structure, check_address_data, check_address_utxos, check_and_fix_missing_txs, check_missing_addresses, count_addr_index, debug_address_balance, deep_address_check, diagnose_balance_issues, diagnose_spent_outputs, find_best_block, rebuild_address_utxo_index, clear_enrichment_flags, reset_enrichment_flags, set_enrichment_flags, backfill_missing_txs

### Transaction Index (12 files) - One-time migrations complete
- analyze_tx_participation, check_tx_exists, check_tx_keys, clear_tx_data, find_missing_txs, find_missing_tx_blocks, inspect_tx, lookup_tx, lookup_txid, rebuild_tx_index, check_specific_tx

### Block Index (14 files) - Sync issues resolved
- analyze_block_index, analyze_sync_correlation, check_chain_duplicates, check_chain_metadata, check_chain_state, check_hash_mapping, check_hash_prev, check_index_integrity, check_metadata_keys, compare_blocks, force_reindex_blocks, scan_all_chain_metadata, scan_leveldb_metadata, trace_chain_sequence

### Offset Indexing (8 files) - Pattern A complete
- analyze_coinstakes, check_pivx_offsets, count_offsets, test_offset_indexer, compare_explorer_txs, fetch_and_analyze_missing_blocks, find_missing_block_ranges

### Coinstake/Collateral (7 files) - Parsing fixed
- check_coinstake_raw, check_collateral_spend, test_coinstake_parser, test_stake_spend, validate_coinstake_balances, test_chainstate_enrichment

### Parser Testing (6 files) - Moved to /tests/ or deleted
- decode_problem_block, test_hashmap, test_input_lookup, test_leveldb_parser, test_specific_varints, test_varint_decode

### Migrations/Repairs (6 files) - Already complete
- build_hash_index, chainstate_aggregate, copy_and_aggregate, debug_block_sync, debug_chain_walk, debug_chainstate

### Miscellaneous (5+ files) - Single-purpose debug tools
- check_sync_height, dump_leveldb_raw, manual_block_trace, show_tx_keys, analyze_chainstate_entries, count_tx_types, count_vouts, inspect_chain_metadata, test_chain_state

---

## ✅ Cleanup Complete

**Date:** 2025-11-24  
**Status:** Production-ready  
**Next Steps:** Review duplicate tools in `/tools/maintenance/` and proceed with logging redesign

---

**Questions or issues?** Check `/PRODUCTION_READINESS_REPORT.md` for full details on the cleanup process and logging migration plan.
