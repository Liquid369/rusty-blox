// Transaction-Related API Endpoints
//
// Endpoints for querying and broadcasting transactions.
// Confirmed transactions are immutable and cached heavily.

use axum::{http::StatusCode, Extension, Json};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::types::{BlockbookError, SendTxResponse, Transaction, TxInput, TxOutput};
use crate::cache::CacheManager;
use crate::chain_state::get_chain_state;
use crate::parser::deserialize_transaction_blocking;

pub use axum::extract::Path as AxumPath;

/// GET /api/v2/tx/{txid}
/// Returns full transaction details with inputs, outputs, and Sapling data.
///
/// **CACHED**: 300 second TTL (confirmed transactions are immutable)
pub async fn tx_v2(
    AxumPath(txid): AxumPath<String>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<BlockbookError>)> {
    let cache_key = format!("tx:{txid}");
    let db_clone = Arc::clone(&db);
    let txid_clone = txid.clone();

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(300), || async move {
            compute_transaction_details(&db_clone, &txid_clone).await
        })
        .await;

    match result {
        Ok(tx) => Ok(Json(tx)),
        Err(e) => {
            // A real storage error must surface as 500 — mapping it to 404 tells
            // clients (and their caches) that an EXISTING tx doesn't exist.
            let status = if e.downcast_ref::<rocksdb::Error>().is_some() {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::NOT_FOUND
            };
            Err((status, Json(BlockbookError::new(e.to_string()))))
        }
    }
}

/// Read a transaction's stored record (`version(4) ++ height(4) ++ raw_tx`), preferring a
/// VALID record over a malformed "phantom stub". A tx may be keyed INTERNAL (reversed
/// display) or DISPLAY; a body-LESS stub (≤8 bytes: version+height, no tx) at one key must
/// NOT shadow the real record at the other (that shadowing is the `/tx` 404 bug). Returns
/// the first record with a body (`len() > 8`), Ok(None) if none exists, or Err on a real
/// RocksDB read failure — callers on money paths must propagate the Err (a swallowed IO
/// error reads as "tx absent" and produces a confident wrong answer).
pub(crate) fn read_valid_tx_record(
    db: &DB,
    cf: &impl rocksdb::AsColumnFamilyRef,
    display_txid_bytes: &[u8],
) -> Result<Option<Vec<u8>>, rocksdb::Error> {
    let internal: Vec<u8> = display_txid_bytes.iter().rev().cloned().collect();
    // Prefer a record WITH a body (len>8) at either key order; a body-LESS stub (≤8 bytes,
    // e.g. reorg orphan-marking) must not shadow the real record.
    for txid in [internal.as_slice(), display_txid_bytes] {
        let mut key = vec![b't'];
        key.extend_from_slice(txid);
        if let Some(d) = db.get_cf(cf, &key)? {
            if d.len() > 8 {
                return Ok(Some(d));
            }
        }
    }
    Ok(None)
}

/// Like [`read_valid_tx_record`], but ALSO reports whether any record at either key
/// order — including body-less stubs — carries the `HEIGHT_ORPHAN` sentinel. A reorg
/// orphan-marks only the DISPLAY record, so a stale INTERNAL record with its old
/// positive height would otherwise shadow the mark entirely (the reader probes
/// internal first, and would count a disconnected tx). Callers whose SUMS must
/// exclude disconnected txs (xpub totals) check the flag; display callers that
/// should still serve an orphaned tx's body keep using `read_valid_tx_record`.
///
/// ponytail: an any-order orphan mark wins even against a positive height at the
/// other order. A tx orphaned during initial sync and later re-broadcast under the
/// same txid would be wrongly skipped — if that ever matters, the upgrade path is a
/// 'B'+height membership check (the authoritative canonical txid list).
pub(crate) fn read_tx_record_orphan_aware(
    db: &DB,
    cf: &impl rocksdb::AsColumnFamilyRef,
    display_txid_bytes: &[u8],
) -> Result<(Option<Vec<u8>>, bool), rocksdb::Error> {
    let internal: Vec<u8> = display_txid_bytes.iter().rev().cloned().collect();
    let mut body: Option<Vec<u8>> = None;
    let mut orphan_marked = false;
    for txid in [internal.as_slice(), display_txid_bytes] {
        let mut key = vec![b't'];
        key.extend_from_slice(txid);
        if let Some(d) = db.get_cf(cf, &key)? {
            if d.len() >= 8 {
                let h = i32::from_le_bytes([d[4], d[5], d[6], d[7]]);
                if h == crate::constants::HEIGHT_ORPHAN {
                    orphan_marked = true;
                }
            }
            if d.len() > 8 && body.is_none() {
                body = Some(d);
            }
        }
    }
    Ok((body, orphan_marked))
}

/// Build Transaction struct from raw DB data
///
/// This is a public helper used by:
/// - tx_v2 endpoint (single transaction)
/// - address/xpub endpoints (batch transaction fetching for details=txs)
///
/// Returns Transaction struct compatible with Blockbook API schema
pub(crate) async fn build_transaction_from_db(
    db: &Arc<DB>,
    txid: &str,
) -> Result<Transaction, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = Arc::clone(db);
    let txid_clone = txid.to_string();

    tokio::task::spawn_blocking(move || {
        let txid_bytes = hex::decode(&txid_clone)?;

        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;

        // Prefer a VALID record over a malformed "phantom stub": the record may be stored
        // internal-keyed (reversed) or display-keyed, and a short (<8-byte) stub at one key
        // must not shadow the real record at the other (otherwise /tx 404s a tx that exists).
        let data = read_valid_tx_record(&db_clone, &cf_transactions, &txid_bytes)?
            .ok_or("Transaction not found")?;

        if data.len() > 10_000_000 {
            return Err("Transaction data too large".into());
        }

        let _block_version = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4]));
        let block_height = i32::from_le_bytes(data[4..8].try_into().unwrap_or([0; 4]));

        let tx_data_len = data.len() - 8;
        if tx_data_len == 0 {
            return Err("Empty transaction data".into());
        }

        let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
        tx_data_with_header.extend_from_slice(&[0u8; 4]); // Dummy block_version
        tx_data_with_header.extend_from_slice(&data[8..]); // Actual tx data

        // Parse the binary transaction data
        let tx = deserialize_transaction_blocking(&tx_data_with_header)?;

        // Build vin (inputs)
        let mut vin = Vec::new();
        let mut value_in: i64 = 0;

        for (idx, input) in tx.inputs.iter().enumerate() {
            if let Some(coinbase_data) = &input.coinbase {
                // Coinbase transaction
                vin.push(TxInput {
                    txid: None,
                    vout: None,
                    sequence: Some(input.sequence as u64),
                    n: idx as u32,
                    addresses: None,
                    is_address: None,
                    value: None,
                    hex: Some(hex::encode(coinbase_data)),
                });
            } else if let Some(prevout) = &input.prevout {
                // Regular input - look up previous output for value and address
                let mut tx_input = TxInput {
                    txid: Some(prevout.hash.clone()),
                    vout: Some(prevout.n),
                    sequence: Some(input.sequence as u64),
                    n: idx as u32,
                    addresses: None,
                    is_address: None,
                    value: None,
                    hex: None,
                };

                // Try to get value and address from previous transaction
                if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                    // Prefer a record WITH a body over an 8-byte stub (same shadowing bug as
                    // read_valid_tx_record) so a stub prevout never silently zeroes valueIn.
                    // A read ERROR is deliberately treated as unresolved (master parity):
                    // input enrichment is best-effort and must not fail the whole /tx.
                    let prev_data_opt =
                        read_valid_tx_record(&db_clone, &cf_transactions, &prev_txid_bytes)
                            .unwrap_or(None);

                    if let Some(prev_data) = prev_data_opt {
                        if prev_data.len() > 10_000_000 {
                            warn!(prevout_hash = %prevout.hash, size_bytes = prev_data.len(), "Previous transaction data too large");
                        } else if prev_data.len() > 8 {
                            let prev_tx_data_len = prev_data.len() - 8;
                            if prev_tx_data_len > 0 {
                                let mut prev_tx_data_with_header = Vec::with_capacity(4 + prev_tx_data_len);
                                prev_tx_data_with_header.extend_from_slice(&[0u8; 4]);
                                prev_tx_data_with_header.extend_from_slice(&prev_data[8..]);

                                if let Ok(prev_tx) = deserialize_transaction_blocking(&prev_tx_data_with_header) {
                                    if let Some(output) = prev_tx.outputs.get(prevout.n as usize) {
                                        tx_input.value = Some(output.value.to_string());
                                        value_in += output.value;
                                        if !output.address.is_empty() {
                                            tx_input.addresses = Some(output.address.clone());
                                            tx_input.is_address = Some(true);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                vin.push(tx_input);
            }
        }

        // Build vout (outputs)
        //
        // P2-D: per-output spent/unspent status. The authoritative live-UTXO
        // source in this codebase is the addr_index 'a'+address set — a serialized
        // list of (txid_bytes, vout) tuples where the txid is stored in canonical
        // DISPLAY order (the same bytes hex(tx.txid) decodes to; see
        // api/addresses.rs::compute_utxos, which trusts this exact set for /utxo).
        // An output (txid, vout) is UNSPENT iff it appears in the 'a' set of one of
        // its own addresses; if absent it has been spent. We mirror block-detail's
        // Blockbook-ish `spent` field name (already present on this TxOutput as
        // serde `spent`). spentTxId is intentionally omitted: there is no cheap
        // forward output->spending-txid index (the utxo_undo 'S' index is keyed for
        // reorg resurrection, not query), and that field is not on the shared
        // TxOutput struct — a reverse scan would be too expensive per request.
        //
        // The txid in DISPLAY order, decoded once, matched against the 'a' set.
        let txid_display_bytes = hex::decode(&tx.txid).unwrap_or_default();
        let cf_addr_index = db_clone.cf_handle("addr_index");
        // GL-1 / 503 for the addr_index-derived `spent` flag: while the index is
        // reindexing or not yet the current (v2) format, the 'a' blobs may be legacy 40B
        // bytes a 49B stride can mis-parse (a UTXO-count multiple of 49 even passes the
        // modulo check), so do NOT read them. `spent` goes uniformly null; the rest of
        // the tx (hash/vin/vout/confirmations, from the transactions CF) stays available,
        // so /tx is never 503'd wholesale — only this one annotation is withheld.
        let addr_index_serveable = crate::chain_state::addr_index_ready(&db_clone);

        // Synchronous probe of the 'a'+address unspent set (we are inside a
        // spawn_blocking closure, so no .await). Returns Some(true) if this exact
        // (txid, vout) is still unspent for `address`, Some(false) if the address
        // is indexed but this output is absent (spent), None if the address has no
        // 'a' entry at all (can't determine from this address).
        let probe_unspent = |address: &str, vout_idx: u64| -> Option<bool> {
            if !addr_index_serveable {
                return None;
            }
            let cf = cf_addr_index.as_ref()?;
            if txid_display_bytes.len() != 32 {
                return None;
            }
            // An empty UTXO set comes in TWO shapes: the monitor DELETES the 'a' key
            // when an address empties (delete_cf at zero balance), while full
            // enrichment writes an EMPTY (0-byte) 'a' value for the same state. Treat
            // both identically: if the address is genuinely KNOWN — its 'r'
            // received-total exists AND is > 0 — an empty set means every output it
            // held is spent (Some(false)). An 'r' of exactly 0 stays None: a reorg
            // rollback leaves 'r'=0 behind for an address whose only receipt was
            // disconnected, and those outputs were never spent — asserting "spent"
            // there would be a false statement. Un-indexed addresses stay None.
            let known_and_emptied = || -> Option<bool> {
                let mut r_key = vec![b'r'];
                r_key.extend_from_slice(address.as_bytes());
                let r = db_clone.get_cf(cf, &r_key).ok().flatten()?;
                let total = <[u8; 8]>::try_from(r.as_slice())
                    .ok()
                    .map(i64::from_le_bytes)?;
                if total > 0 {
                    Some(false)
                } else {
                    None
                }
            };
            let mut key = vec![b'a'];
            key.extend_from_slice(address.as_bytes());
            let data = match db_clone.get_cf(cf, &key).ok().flatten() {
                Some(d) if d.is_empty() => return known_and_emptied(),
                Some(d) => d,
                None => return known_and_emptied(),
            };
            // v2 'a' format: repeated 49-byte [txid(32)+vout(8 LE)+value(8)+kind(1)].
            // Only txid+vout (the first 40 bytes) matter here; ignore the trailing 9.
            if data.len() % crate::parser::ADDR_UTXO_STRIDE != 0 {
                return None;
            }
            let mut found = false;
            for chunk in data.chunks_exact(crate::parser::ADDR_UTXO_STRIDE) {
                if chunk[0..32] == txid_display_bytes[..] {
                    let v = u64::from_le_bytes(chunk[32..40].try_into().unwrap_or([0u8; 8]));
                    if v == vout_idx {
                        found = true;
                        break;
                    }
                }
            }
            Some(found)
        };

        let mut vout = Vec::new();
        let mut value_out: i64 = 0;

        for (idx, output) in tx.outputs.iter().enumerate() {
            value_out += output.value;

            // Determine spent status only for outputs that carry an address (the
            // 'a' index is address-keyed). Unspendable outputs (OP_RETURN, empty
            // scripts) can't be tracked — leave `spent` as None so the frontend
            // degrades cleanly rather than mislabeling them.
            let spent = if output.address.is_empty() {
                None
            } else {
                // An output is unspent if it is still present in the 'a' set of
                // ANY of its addresses; spent only if every indexed address agrees
                // it is absent. If no address has an 'a' entry we leave it unknown.
                let mut any_known = false;
                let mut still_unspent = false;
                for address in &output.address {
                    if let Some(unspent) = probe_unspent(address, idx as u64) {
                        any_known = true;
                        if unspent {
                            still_unspent = true;
                            break;
                        }
                    }
                }
                if !any_known {
                    None
                } else {
                    Some(!still_unspent)
                }
            };

            vout.push(TxOutput {
                value: output.value.to_string(),
                n: idx as u32,
                hex: Some(hex::encode(&output.script_pubkey.script)),
                addresses: if output.address.is_empty() { None } else { Some(output.address.clone()) },
                is_address: Some(!output.address.is_empty()),
                spent,
            });
        }

        let tx_size = data.len() - 8;

        // Get block hash and time if we have a valid height
        let (block_hash, block_time) = if block_height > 0 {
            let height_key = (block_height as u32).to_le_bytes().to_vec();

            if let Some(cf_metadata) = db_clone.cf_handle("chain_metadata") {
                if let Ok(Some(hash_bytes)) = db_clone.get_cf(&cf_metadata, &height_key) {
                    let hash_hex = hex::encode(&hash_bytes);

                    if let Some(cf_blocks) = db_clone.cf_handle("blocks") {
                        let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
                        if let Ok(Some(header_bytes)) = db_clone.get_cf(&cf_blocks, &internal_hash) {
                            if header_bytes.len() >= 72 {
                                let time = u32::from_le_bytes(
                                    header_bytes[68..72].try_into().unwrap_or([0; 4])
                                ) as u64;
                                (hash_hex, time)
                            } else {
                                (hash_hex, 0)
                            }
                        } else {
                            (hash_hex, 0)
                        }
                    } else {
                        (hash_hex, 0)
                    }
                } else {
                    (String::new(), 0)
                }
            } else {
                (String::new(), 0)
            }
        } else {
            (String::new(), 0)
        };

        // Get current height for confirmations
        let chain_state = get_chain_state(&db_clone).ok();
        let current_height = chain_state.as_ref().map(|cs| cs.height).unwrap_or(0);

        let confirmations = if block_height > 0 && current_height > 0 {
            (current_height - block_height + 1).max(0) as u32
        } else {
            0
        };

        // Calculate fees (for non-coinbase)
        let fees = if value_in > 0 && value_in >= value_out {
            value_in - value_out
        } else {
            0
        };

        // Sapling (shielded) detail for version >= 3 transactions. Mirrors the
        // block-detail endpoint's mapping exactly so the tx page renders the same
        // shielded card (counts, value balance, binding sig, spend/output crypto).
        let sapling = tx.sapling_data.as_ref().map(|sap| {
            let spends = sap.vshielded_spend.iter().map(|spend| crate::block_detail::SpendInfo {
                cv: hex::encode(spend.cv),
                anchor: hex::encode(spend.anchor),
                nullifier: hex::encode(spend.nullifier),
                rk: hex::encode(spend.rk),
                zkproof: hex::encode(spend.zkproof),
                spend_auth_sig: hex::encode(spend.spend_auth_sig),
            }).collect();
            let outputs = sap.vshielded_output.iter().map(|output| crate::block_detail::OutputInfo {
                cv: hex::encode(output.cv),
                cmu: hex::encode(output.cmu),
                ephemeral_key: hex::encode(output.ephemeral_key),
                enc_ciphertext: hex::encode(output.enc_ciphertext),
                out_ciphertext: hex::encode(output.out_ciphertext),
                zkproof: hex::encode(output.zkproof),
            }).collect();
            crate::block_detail::SaplingInfo {
                value_balance: sap.value_balance as f64 / 100_000_000.0, // satoshis -> PIV
                shielded_spend_count: sap.vshielded_spend.len() as u64,
                shielded_output_count: sap.vshielded_output.len() as u64,
                binding_sig: hex::encode(sap.binding_sig),
                spends: Some(spends),
                outputs: Some(outputs),
            }
        });

        Ok(Transaction {
            txid: tx.txid,
            version: Some(tx.version as i32),
            lock_time: Some(tx.lock_time),
            vin,
            vout,
            block_hash,
            block_height,
            confirmations,
            block_time,
            size: Some(tx_size),
            vsize: Some(tx_size),
            value: value_out.to_string(),
            value_in: value_in.to_string(),
            fees: fees.to_string(),
            hex: hex::encode(&data[8..]),
            sapling,
        })
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

async fn compute_transaction_details(
    db: &Arc<DB>,
    txid: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    // Use the shared transaction builder
    let tx = build_transaction_from_db(db, txid).await?;

    // Convert to JSON for current endpoint compatibility
    // TODO: Eventually return Transaction struct directly
    Ok(serde_json::to_value(tx)?)
}

/// GET /api/v2/sendtx/{hex}
/// Legacy endpoint for broadcasting raw transactions.
///
/// **NO CACHE**: Write operation
pub async fn send_tx_v2(
    AxumPath(param): AxumPath<String>,
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    send_transaction_internal(param).await
}

/// POST /api/v2/sendtx
/// Blockbook-compatible endpoint for broadcasting transactions.
/// Accepts raw transaction hex in request body (plain text or JSON).
///
/// **NO CACHE**: Write operation
pub async fn send_tx_post_v2(
    body: String,
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    // Body can be either plain hex or JSON {"hex": "..."}
    let tx_hex = if body.trim().starts_with('{') {
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(json) => json
                .get("hex")
                .and_then(|v| v.as_str())
                .unwrap_or(&body)
                .trim()
                .to_string(),
            Err(_) => body.trim().to_string(),
        }
    } else {
        body.trim().to_string()
    };

    send_transaction_internal(tx_hex).await
}

async fn send_transaction_internal(
    tx_hex: String,
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    // Validate input BEFORE touching the node: must be hex and within PIVX's
    // 2 MB block size limit (4M hex chars). Previously arbitrary input was
    // forwarded to the node on a freshly spawned OS thread per request —
    // unbounded thread growth under load, leaked threads on timeout.
    let tx_hex = tx_hex.trim().to_string();
    if tx_hex.is_empty() || tx_hex.len() > 4_000_000 || tx_hex.len() % 2 != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(BlockbookError::new("Invalid transaction hex length")),
        ));
    }
    if !tx_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(BlockbookError::new("Transaction must be hex-encoded")),
        ));
    }

    // Async RPC call with the shared client (15s hard timeout, no thread spawn)
    match super::helpers::rpc_call_json("sendrawtransaction", serde_json::json!([tx_hex, false]))
        .await
    {
        Ok(txid) => {
            let txid_str = txid
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| txid.to_string());
            Ok(Json(SendTxResponse {
                result: Some(txid_str),
                error: None,
            }))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            // Node rejection reasons (e.g. "bad-txns-inputs-spent") are part of the
            // Blockbook contract — wallets rely on them.
            Json(BlockbookError::new(format!(
                "Failed to send transaction: {e}"
            ))),
        )),
    }
}

/// Batch fetch multiple transactions efficiently with parallel processing
///
/// Used by address/xpub endpoints when details=txs is requested
/// Processes transactions in batches to avoid overwhelming the system
pub(crate) async fn fetch_transactions_batch(db: &Arc<DB>, txids: &[String]) -> Vec<Transaction> {
    const BATCH_SIZE: usize = 50;
    let mut results = Vec::with_capacity(txids.len());

    for chunk in txids.chunks(BATCH_SIZE) {
        let futures: Vec<_> = chunk
            .iter()
            .map(|txid| build_transaction_from_db(db, txid))
            .collect();

        let batch_results = futures::future::join_all(futures).await;
        results.extend(batch_results.into_iter().filter_map(|r| r.ok()));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};

    fn seed() -> (tempfile::TempDir, DB, Vec<u8>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = DB::open_cf(&opts, temp.path(), ["transactions"]).unwrap();
        (temp, db, (0u8..32).collect())
    }
    fn put(db: &DB, txid: &[u8], val: &[u8]) {
        let cf = db.cf_handle("transactions").unwrap();
        let mut k = vec![b't'];
        k.extend_from_slice(txid);
        db.put_cf(&cf, &k, val).unwrap();
    }

    // A short stub at the internal key must NOT shadow the valid record at the display key.
    #[test]
    fn prefers_valid_display_over_internal_stub() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        put(&db, &internal, &[0u8, 1, 2]); // 3-byte stub
        let valid = vec![1u8, 0, 0, 0, 5, 0, 0, 0, 0xAA, 0xBB];
        put(&db, &display, &valid);
        let cf = db.cf_handle("transactions").unwrap();
        assert_eq!(
            read_valid_tx_record(&db, &cf, &display).unwrap(),
            Some(valid)
        );
    }

    // Valid at internal (the normal case) is returned; a display stub doesn't interfere.
    #[test]
    fn prefers_valid_internal() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        let valid = vec![1u8, 0, 0, 0, 9, 0, 0, 0, 0xCC];
        put(&db, &internal, &valid);
        put(&db, &display, &[0u8]); // 1-byte stub
        let cf = db.cf_handle("transactions").unwrap();
        assert_eq!(
            read_valid_tx_record(&db, &cf, &display).unwrap(),
            Some(valid)
        );
    }

    // Only stubs -> None (a genuine 404).
    #[test]
    fn none_when_only_stubs() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        put(&db, &internal, &[0u8, 1]);
        put(&db, &display, &[0u8, 1, 2]);
        let cf = db.cf_handle("transactions").unwrap();
        assert_eq!(read_valid_tx_record(&db, &cf, &display).unwrap(), None);
    }

    // THE reorg bug: an EXACTLY-8-byte body-less stub (version(4)+HEIGHT_ORPHAN(4), no tx
    // bytes — written by reorg.rs disconnect_transaction) passes the len>=8 check. It must
    // NOT shadow the real record (which has a body, len>8) at the other key order.
    #[test]
    fn prefers_body_over_eight_byte_stub() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        let stub = vec![0u8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]; // version=0, height=-1, no body
        put(&db, &internal, &stub);
        let valid = vec![1u8, 0, 0, 0, 5, 0, 0, 0, 0xAA, 0xBB]; // has a body
        put(&db, &display, &valid);
        let cf = db.cf_handle("transactions").unwrap();
        assert_eq!(
            read_valid_tx_record(&db, &cf, &display).unwrap(),
            Some(valid)
        );
    }

    // A reorg orphan-marks only the DISPLAY record; the stale internal record keeps its
    // positive height. The orphan-aware reader must return the body AND raise the flag,
    // so summing callers (xpub totals) can skip the disconnected tx.
    #[test]
    fn orphan_aware_flags_display_orphan_behind_internal_body() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        let confirmed = vec![1u8, 0, 0, 0, 5, 0, 0, 0, 0xAA, 0xBB]; // height 5, body
        put(&db, &internal, &confirmed);
        let orphaned = vec![1u8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xAA, 0xBB]; // height -1, body
        put(&db, &display, &orphaned);
        let cf = db.cf_handle("transactions").unwrap();
        let (body, orphan_marked) = read_tx_record_orphan_aware(&db, &cf, &display).unwrap();
        assert_eq!(body, Some(confirmed), "internal body still served");
        assert!(orphan_marked, "display orphan mark must not be shadowed");
        // No orphan mark anywhere -> flag stays false.
        let (_t2, db2, display2) = seed();
        let internal2: Vec<u8> = display2.iter().rev().cloned().collect();
        put(&db2, &internal2, &vec![1u8, 0, 0, 0, 5, 0, 0, 0, 0xCC]);
        let cf2 = db2.cf_handle("transactions").unwrap();
        let (body2, orphan2) = read_tx_record_orphan_aware(&db2, &cf2, &display2).unwrap();
        assert!(body2.is_some());
        assert!(!orphan2);
    }

    // An exactly-8-byte body-less stub at every key order has no body to serve -> None
    // (guards the len>8 vs len>=8 boundary that WAS the bug).
    #[test]
    fn none_when_only_eight_byte_stub() {
        let (_t, db, display) = seed();
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        let stub = vec![0u8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        put(&db, &internal, &stub);
        put(&db, &display, &stub);
        let cf = db.cf_handle("transactions").unwrap();
        assert_eq!(read_valid_tx_record(&db, &cf, &display).unwrap(), None);
    }
}
