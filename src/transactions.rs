use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeek, AsyncSeekExt, BufReader, SeekFrom};
use rocksdb::DB;
use crate::batch_writer::BatchWriter;
use crate::constants::HEIGHT_UNRESOLVED;
use crate::db_utils::{perform_rocksdb_get, perform_rocksdb_put, perform_rocksdb_del};
use crate::types::{CTransaction, CTxIn, CTxOut, AddressType, CScript, SpendDescription, OutputDescription, SaplingTxData, COutPoint};
use crate::address::{scriptpubkey_to_address, address_type_to_string};
use crate::parser::{serialize_utxos, deserialize_utxos, deserialize_transaction, reverse_bytes};
use crate::tx_type::{detect_type_from_components, TransactionType};
use std::error::Error;
use std::io::{self, Cursor};
use std::pin::Pin;
use std::task::{Context, Poll};
use byteorder::{LittleEndian};
use tokio::fs::File;
use sha2::{Sha256, Digest};

#[allow(dead_code)] // PIVX magic bytes - may be needed for raw block validation
const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
#[allow(dead_code)] // Size limit for transaction payloads - may be needed for validation
const MAX_PAYLOAD_SIZE: usize = 10000;

/// Priority 1.3: Wrapper to make Cursor AsyncRead compatible
struct AsyncCursor<'a> {
    inner: Cursor<&'a [u8]>,
}

impl<'a> AsyncCursor<'a> {
    fn new(cursor: Cursor<&'a [u8]>) -> Self {
        Self { inner: cursor }
    }
}

impl tokio::io::AsyncRead for AsyncCursor<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        use std::io::Read as StdRead;
        let amt = StdRead::read(&mut self.inner, buf.initialize_unfilled())?;
        buf.advance(amt);
        Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncSeek for AsyncCursor<'_> {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        use std::io::Seek as StdSeek;
        StdSeek::seek(&mut self.inner, position)?;
        Ok(())
    }

    fn poll_complete(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        use std::io::Seek as StdSeek;
        Poll::Ready(Ok(StdSeek::stream_position(&mut self.inner)?))
    }
}

/// Priority 1.3: Process transactions from a bounded buffer (Cursor)
/// This prevents transaction parse errors from misaligning the file cursor
pub async fn process_transaction_from_buffer(
    cursor: Cursor<&[u8]>,
    block_version: u32,
    block_hash: &[u8],
    block_height: Option<i32>,
    _db: Arc<DB>,
    batch: &mut BatchWriter,
    fast_sync: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Wrap the cursor to make it AsyncRead
    let mut async_cursor = AsyncCursor::new(cursor);
    
    let tx_amt = read_varint(&mut async_cursor).await?;
    
    if tx_amt > 100000 {
        return Err(format!("Invalid transaction count: {} (likely corrupt)", tx_amt).into());
    }
    
    for tx_index in 0..tx_amt {
        let start_pos = async_cursor.inner.position();

        let tx_ver_out = async_cursor.read_u16_le().await?;
        let tx_type = async_cursor.read_u16_le().await?;

        if block_version == 11 {
            process_transaction_v1(
                &mut async_cursor,
                tx_ver_out.try_into().unwrap_or(1),
                tx_type,
                block_version,
                block_hash,
                block_height,
                tx_index,
                _db.clone(),
                start_pos,
                batch,
                fast_sync,
            ).await?;
        } else if (tx_ver_out <= 2 && block_version < 11) || (tx_ver_out > 1 && block_version > 7) {
            process_transaction_v1(
                &mut async_cursor,
                tx_ver_out.try_into().unwrap_or(1),
                tx_type,
                block_version,
                block_hash,
                block_height,
                tx_index,
                _db.clone(),
                start_pos,
                batch,
                fast_sync,
            ).await?;
        }
    }
    Ok(())
}

pub async fn process_transaction(
    reader: &mut BufReader<File>,
    block_version: u32,
    block_hash: &[u8],
    block_height: Option<i32>,
    _db: Arc<DB>,
    batch: &mut BatchWriter,
    fast_sync: bool,
) -> Result<(), io::Error> {
    let tx_amt = read_varint(reader).await?;
    
    if tx_amt > 100000 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, 
            format!("Invalid transaction count: {} (likely corrupt varint)", tx_amt)));
    }
    
    for tx_index in 0..tx_amt {
        let start_pos = reader.stream_position().await?;

        let tx_ver_out = reader.read_u16_le().await?;
        let tx_type = reader.read_u16_le().await?;

        if block_version == 11 {
            // For block version 11, handle all transaction versions uniformly
            if let Err(e) = process_transaction_v1(
                reader,
                tx_ver_out.try_into().unwrap_or(1),
                tx_type,
                block_version,
                block_hash,
                block_height,
                tx_index,
                _db.clone(),
                start_pos,
                batch,
                fast_sync,
            ).await {
                eprintln!("Error processing transaction (version {}): {}", tx_ver_out, e);
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }
        } else if (tx_ver_out <= 2 && block_version < 11) || (tx_ver_out > 1 && block_version > 7) {
            // For older blocks, process v1/v2 transactions
            if let Err(e) = process_transaction_v1(
                reader,
                tx_ver_out.try_into().unwrap_or(1),
                tx_type,
                block_version,
                block_hash,
                block_height,
                tx_index,
                _db.clone(),
                start_pos,
                batch,
                fast_sync,
            ).await {
                eprintln!("Error processing transaction (version {}): {}", tx_ver_out, e);
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }
        }
    }
    Ok(())
}

async fn process_transaction_v1(
    reader: &mut (impl AsyncReadExt + AsyncSeek + Unpin),
    tx_ver_out: i16,
    tx_type: u16,
    block_version: u32,
    _block_hash: &[u8],
    block_height: Option<i32>,
    tx_index: u64,
    _db: Arc<DB>,
    start_pos: u64,
    batch: &mut BatchWriter,
    fast_sync: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // CRITICAL OPTIMIZATION: In fast_sync mode, we don't track UTXOs or addresses
    // So skip expensive CF handle lookups entirely!
    let (_cf_transactions, _cf_pubkey, _cf_utxo) = if !fast_sync {
        let cf_tx = _db
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        let cf_pub = _db
            .cf_handle("pubkey")
            .ok_or("pubkey CF not found")?;
        let cf_ut = _db
            .cf_handle("utxo")
            .ok_or("utxo CF not found")?;
        (Some(cf_tx), Some(cf_pub), Some(cf_ut))
    } else {
        (None, None, None)
    };
    
    let input_count = read_varint(reader).await?;

    let mut inputs = Vec::new();
    for i in 0..input_count {
        let mut coinbase = None;
        let mut prev_output = None;
        let mut script = None;

        match (block_version, tx_ver_out) {
            (ver, 2) if ver < 3 => {
                let mut buffer = [0; 26];
                reader.read_exact(&mut buffer).await?;
                coinbase = Some(buffer.to_vec());
            }
            _ => {
                prev_output = Some(read_outpoint(reader).await?);
                script = Some(read_script(reader).await?);
            }
        }

        let sequence = reader.read_u32_le().await?;
        inputs.push(CTxIn {
            prevout: prev_output,
            script_sig: CScript {
                script: script.unwrap_or_default(),
            },
            sequence,
            index: i,
            coinbase,
        });
    }

    let output_count = read_varint(reader).await?;
    
    // First, collect all outputs WITHOUT address parsing
    let mut outputs = Vec::new();
    for i in 0..output_count {
        let value = reader.read_i64_le().await?;
        let script = read_script(reader).await?;
        
        outputs.push(CTxOut {
            value,
            script_length: script.len().try_into().unwrap_or(0),
            script_pubkey: CScript { script },
            index: i,
            address: Vec::new(), // Will populate below if needed
        });
    }

    let lock_time_buff = reader.read_u32_le().await?;
    
    // CRITICAL FIX: Use PIVX Core-conformant transaction type detection
    // This replaces the old heuristic that incorrectly classified transactions
    // See tx_type.rs for the authoritative implementation matching PIVX Core
    let detected_tx_type = detect_type_from_components(&inputs, &outputs);
    
    // Map to general_address_type for backward compatibility with existing code
    let general_address_type = match detected_tx_type {
        TransactionType::Coinbase => AddressType::CoinBaseTx,
        TransactionType::Coinstake => AddressType::CoinStakeTx,
        TransactionType::Normal => AddressType::Nonstandard,
    };
    
    // Now populate addresses if not in fast_sync mode
    if !fast_sync {
        for tx_out in &mut outputs {
            let address_type = get_address_type(tx_out, &general_address_type).await;
            tx_out.address = address_type_to_string(Some(address_type)).await;
        }
    }

    // For Sapling transactions (version >= 3), skip the Sapling-specific data
    if tx_ver_out >= 3 {
        // Read value_count varint
        let _ = read_varint(reader).await;
        
        // Read valueBalance (i64)
        let mut value_balance_buf = [0u8; 8];
        let _ = reader.read_exact(&mut value_balance_buf).await;
        
        // Read and skip vShieldSpend
        if let Ok(spend_count) = read_varint(reader).await {
            for _ in 0..spend_count {
                // Each spend: 384 bytes
                let mut spend_buf = vec![0u8; 384];
                let _ = reader.read_exact(&mut spend_buf).await;
            }
        }
        
        // Read and skip vShieldOutput
        if let Ok(output_count) = read_varint(reader).await {
            for _ in 0..output_count {
                // Each output: 948 bytes
                let mut output_buf = vec![0u8; 948];
                let _ = reader.read_exact(&mut output_buf).await;
            }
        }
        
        // Skip bindingSig (64 bytes)
        let mut binding_sig = vec![0u8; 64];
        let _ = reader.read_exact(&mut binding_sig).await;
        
        // For special transaction types (nType != 0), skip extraPayload
        if tx_type != 0 {
            if let Ok(payload_size) = read_varint(reader).await {
                if payload_size > 0 {
                    let mut payload = vec![0u8; payload_size as usize];
                    let _ = reader.read_exact(&mut payload).await;
                }
            }
        }
    }

    // Get current position as end_pos
    let end_pos = set_end_pos(reader, start_pos).await?;
    
    // CRITICAL FIX: Read transaction bytes for TXID calculation
    // This can fail with UnexpectedEof if blk file is truncated/corrupted
    // We handle this gracefully to avoid losing block metadata
    let tx_bytes_result = get_txid_bytes(reader, start_pos, end_pos).await;
    
    let (tx_bytes, reversed_txid) = match tx_bytes_result {
        Ok(bytes) => {
            let txid = hash_txid(&bytes).await?;
            (bytes, txid)
        },
        Err(e) => {
            // EOF while reading transaction bytes - likely file truncation
            // Use a placeholder TXID and store what we have
            eprintln!("Warning: EOF reading tx bytes at block height {:?}, tx index {}: {}", 
                      block_height, tx_index, e);
            eprintln!("         Using placeholder TXID. This transaction may need reindexing.");
            
            // Create a deterministic placeholder based on position
            let placeholder = format!("TRUNCATED_TX_{}_{}", 
                                     block_height.unwrap_or(0), tx_index);
            let placeholder_bytes = placeholder.as_bytes().to_vec();
            let placeholder_txid = hash_txid(&placeholder_bytes).await?;
            
            (placeholder_bytes, placeholder_txid)
        }
    };

    let transaction = CTransaction {
        txid: hex::encode(reversed_txid.clone()),
        version: tx_ver_out,
        inputs,
        outputs: outputs.clone(),
        lock_time: lock_time_buff,
        sapling_data: None,  // File-based indexing doesn't parse Sapling details (stored as raw bytes)
    };

    //println!("Transaction ID: {:?}", hex::encode(&reversed_txid));

    // UTXO tracking and address indexing
    if !fast_sync {
        // FULL MODE: Complete UTXO tracking with spent output removal
        // This is SLOW because it looks up previous transactions for every input
        let mut key_pubkey = vec![b'p'];
        for tx_out in &transaction.outputs {
            let address_type = get_address_type(tx_out, &general_address_type).await;

            // Associate by these with UTXO set
            handle_address(
                _db.clone(),
                &address_type,
                reversed_txid.clone(),
                tx_out.index.try_into().unwrap_or(0),
            ).await?;

            // 'p' + scriptpubkey -> list of (txid, output_index)
            key_pubkey.extend_from_slice(&tx_out.script_pubkey.script);

            // Fetch existing UTXOs
            let existing_data_option = perform_rocksdb_get(_db.clone(), "pubkey", key_pubkey.clone()).await;
            if let Ok(Some(existing_data)) = existing_data_option {
                let mut existing_utxos = deserialize_utxos(&existing_data).await;
                // Add new UTXO
                existing_utxos.push((reversed_txid.clone(), tx_out.index));

                // Store the updated UTXOs
                let serialized_utxos = serialize_utxos(&existing_utxos).await;
                perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialized_utxos).await.ok();
            }
            // Create a UTXO identifier (txid + output index)
            // CRITICAL FIX: Use raw bytes, not hex-encoded string
            // reversed_txid is already in internal format (raw bytes)
            let mut key_utxo = vec![b'u'];
            key_utxo.extend_from_slice(&reversed_txid);
            let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
            perform_rocksdb_put(_db.clone(), "utxo", key_utxo.clone(), serialize_utxos(&utxos_to_serialize).await).await.ok();
            // --- NEW: update addr_index transaction list ('t'+address) and received total ('r'+address)
            if !tx_out.address.is_empty() {
                for addr in &tx_out.address {
                    // Append internal txid (reversed_txid) to 't'+address
                    let mut key_t = vec![b't'];
                    key_t.extend_from_slice(addr.as_bytes());
                    match perform_rocksdb_get(_db.clone(), "addr_index", key_t.clone()).await {
                        Ok(Some(mut existing)) => {
                            existing.extend_from_slice(&reversed_txid);
                            perform_rocksdb_put(_db.clone(), "addr_index", key_t.clone(), existing).await.ok();
                        }
                        _ => {
                            let mut newb = Vec::with_capacity(32);
                            newb.extend_from_slice(&reversed_txid);
                            perform_rocksdb_put(_db.clone(), "addr_index", key_t.clone(), newb).await.ok();
                        }
                    }

                    // Update received total 'r'+address (i64 LE)
                    let mut key_r = vec![b'r'];
                    key_r.extend_from_slice(addr.as_bytes());
                    let mut current_received: i64 = 0;
                    if let Ok(Some(existing_r)) = perform_rocksdb_get(_db.clone(), "addr_index", key_r.clone()).await {
                        if existing_r.len() == 8 {
                            current_received = i64::from_le_bytes(existing_r[0..8].try_into().unwrap_or([0u8;8]));
                        }
                    }
                    current_received += tx_out.value;
                    perform_rocksdb_put(_db.clone(), "addr_index", key_r.clone(), current_received.to_le_bytes().to_vec()).await.ok();
                }
            }
        }

        for tx_in in &transaction.inputs {
            let mut key = vec![b't'];
            if let Some(actual_prevout) = &tx_in.prevout {
                key.extend_from_slice(actual_prevout.hash.as_bytes());
            }
            let tx_data_option = perform_rocksdb_get(_db.clone(), "transactions", key.clone()).await;
            if let Ok(Some(tx_data)) = tx_data_option {
                // Safely deserialize - skip if corrupted
                if let Ok(referenced_transaction) = deserialize_transaction(&tx_data).await {
                    if let Some(prevout) = &tx_in.prevout {
                        let output = &referenced_transaction.outputs[prevout.n as usize];
                        let address_type = get_address_type(output, &general_address_type).await;

                        // --- NEW: attribute this spending tx to the previous output's address(es)
                        if !output.address.is_empty() {
                            for addr in &output.address {
                                // Append internal txid to 't'+address
                                let mut key_t = vec![b't'];
                                key_t.extend_from_slice(addr.as_bytes());
                                match perform_rocksdb_get(_db.clone(), "addr_index", key_t.clone()).await {
                                    Ok(Some(mut existing)) => {
                                        existing.extend_from_slice(&reversed_txid);
                                        let _ = perform_rocksdb_put(_db.clone(), "addr_index", key_t.clone(), existing).await;
                                    }
                                    _ => {
                                        let mut newb = Vec::with_capacity(32);
                                        newb.extend_from_slice(&reversed_txid);
                                        let _ = perform_rocksdb_put(_db.clone(), "addr_index", key_t.clone(), newb).await;
                                    }
                                }

                                // Update 's' (sent total) for this address
                                let mut key_s = vec![b's'];
                                key_s.extend_from_slice(addr.as_bytes());
                                let mut current_sent: i64 = 0;
                                if let Ok(Some(existing_s)) = perform_rocksdb_get(_db.clone(), "addr_index", key_s.clone()).await {
                                    if existing_s.len() == 8 {
                                        current_sent = i64::from_le_bytes(existing_s[0..8].try_into().unwrap_or([0u8;8]));
                                    }
                                }
                                current_sent += output.value;
                                let _ = perform_rocksdb_put(_db.clone(), "addr_index", key_s.clone(), current_sent.to_le_bytes().to_vec()).await;
                            }
                        }

                        let _ = remove_utxo_addr(_db.clone(), &address_type, &prevout.hash, prevout.n).await;
                    }
                }
            }
            
            // CRITICAL FIX: UTXO keys must use raw bytes, not hex strings
            // prevout.hash is a hex string (display format), need to decode and reverse
            let mut key_utxo = vec![b'u'];
            if let Some(actual_prevout) = &tx_in.prevout {
                // Decode hex string to bytes, then reverse to internal format
                if let Ok(hash_bytes) = hex::decode(&actual_prevout.hash) {
                    let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
                    key_utxo.extend_from_slice(&internal_hash);
                }
            }
            if let Ok(Some(data)) = perform_rocksdb_get(_db.clone(), "pubkey", key_utxo.clone()).await {
                let mut utxos = deserialize_utxos(&data).await;

                // Remove the UTXO that matches the current transaction's input
                if let Some(prevout) = &tx_in.prevout {
                    let _hash = &prevout.hash;
                    let _n = prevout.n;
                    if let Some(pos) = utxos
                        .iter()
                        .position(|(txid, index)| *txid == _hash.as_bytes() && *index == _n as u64)
                    {
                        utxos.remove(pos);
                    }
                }

                // Serialize the updated list of UTXOs and store it back in the database
                if !utxos.is_empty() {
                    perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialize_utxos(&utxos).await).await.ok();
                } else {
                    perform_rocksdb_del(_db.clone(), "pubkey", key_pubkey.clone()).await.ok();
                }
            }

            // Remove the referenced UTXO from the UTXO set
            perform_rocksdb_del(_db.clone(), "utxo", key_utxo.clone()).await.ok();
        }
    } else {
        // FAST SYNC MODE: SKIP ALL ADDRESS INDEXING
        // Addresses can be built later with build_address_index tool or enrich_addresses
        // This gives maximum speed for initial sync - just index blocks and transactions
        // NO address parsing, NO async calls, NO database writes for addresses
    } // End UTXO tracking / address indexing

    // Store transaction with proper indexing
    // 1. Main transaction storage: 't' + txid → (block_version + block_height + tx_bytes)
    let mut tx_key = vec![b't'];
    tx_key.extend_from_slice(&reversed_txid);
    
    let version_bytes = block_version.to_le_bytes().to_vec();
    
    // DEFENSIVE CHECK: Prevent height=0 bug
    // Fix for Phase 2, Issue #1: Transaction Height Assignment Race
    // Should never happen if metadata validation works, but prevents silent corruption
    let height_bytes = match block_height {
        Some(h) => h.to_le_bytes().to_vec(),
        None => {
            // Block has no height - likely an orphan block not in canonical chain
            // This is EXPECTED for blocks in blk files that aren't on canonical chain
            HEIGHT_UNRESOLVED.to_le_bytes().to_vec()
        }
    };
    
    let mut full_data = version_bytes.clone();
    full_data.extend(&height_bytes);
    full_data.extend(&tx_bytes);
    
    batch.put("transactions", tx_key.clone(), full_data);
    
    // 2. Block transaction index: 'B' + height (4 bytes) + tx_index (8 bytes) → txid
    // This allows efficient listing of all transactions in a block
    if let Some(height) = block_height {
        let mut block_tx_key = vec![b'B'];
        block_tx_key.extend(&height.to_le_bytes());
        block_tx_key.extend(&tx_index.to_le_bytes());
        
        // Store the display format (reversed) txid so we can show it directly
        let display_txid = hex::encode(reversed_txid.iter().rev().cloned().collect::<Vec<u8>>());
        batch.put("transactions", block_tx_key, display_txid.as_bytes().to_vec());
    }
    
    // Safe conversion - use current position if conversion fails
    match end_pos.try_into() {
        Ok(pos) => reader.seek(SeekFrom::Start(pos)).await?,
        Err(_) => {
            eprintln!("Failed to convert end_pos, seeking to current position");
            reader.seek(SeekFrom::Current(0)).await?
        }
    };

    Ok(())
}

pub async fn hash_txid(tx_bytes: &[u8]) -> Result<Vec<u8>, io::Error> {
    //Create TXID by hashing twice and reversing result
    let first_hash = Sha256::digest(tx_bytes);
    let txid = Sha256::digest(&first_hash);
    let reversed_txid: Vec<_> = txid.iter().rev().cloned().collect();

    Ok(reversed_txid)
}

async fn read_outpoint<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<COutPoint> {
    // Set size for hash
    let mut hash = [0u8; 32];
    // Read hash asynchronously
    reader.read_exact(&mut hash).await?;
    // Read output index asynchronously
    let n = reader.read_u32_le().await?;
    let reversed_bytes = reverse_bytes(&hash).await;
    let hex_hash = hex::encode(&reversed_bytes);

    Ok(COutPoint { hash: hex_hash, n })
}

#[allow(dead_code)] // Sapling transaction parser - reserved for future Sapling support
async fn parse_sapling_tx_data(
    reader: &mut BufReader<File>,
    start_pos: u64,
    _db: Arc<DB>,
    _batch: &mut BatchWriter,
) -> Result<SaplingTxData, io::Error> {
    let _cf_transactions = _db
        .cf_handle("transactions")
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "transactions CF not found"))?;
    let _cf_pubkey = _db
        .cf_handle("pubkey")
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "pubkey CF not found"))?;
    let _cf_utxo = _db
        .cf_handle("utxo")
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "utxo CF not found"))?;

    // Set empty vectors for later access
    let _inputs: Vec<CTxIn> = Vec::new();
    let _outputs: Vec<CTxOut> = Vec::new();
    // Potential Vin Vector
    let input_count = read_varint(reader).await? as u64;
    println!("Input Count: {}", input_count);

    let mut inputs = Vec::new();
    if input_count > 0 {
        for i in 0..input_count {
            let prev_output = read_outpoint(reader).await?;
            let script = read_script(reader).await?;
            let sequence = reader.read_u32_le().await?;
            inputs.push(CTxIn {
                prevout: Some(prev_output),
                script_sig: CScript { script },
                sequence,
                index: i,
                coinbase: None,
            });
        }
    }

    let output_count = read_varint(reader).await? as u64;
    //println!("Output Count: {}", output_count);
    let general_address_type = if input_count == 1 && output_count == 1 {
        AddressType::CoinBaseTx
    } else if output_count > 1 {
        AddressType::CoinStakeTx
    } else {
        AddressType::Nonstandard
    };

    let mut outputs = Vec::new();
    if output_count > 0 {
        for i in 0..output_count {
            let value = reader.read_i64_le().await?;
            let script = read_script(reader).await?;
            let address_type = get_address_type(
                &CTxOut {
                    value,
                    script_length: script.len().try_into().unwrap_or(0),
                    script_pubkey: CScript { script: script.clone() },
                    index: i,
                    address: Vec::new(),
                },
                &general_address_type,
            ).await;
            let addresses = address_type_to_string(Some(address_type.clone())).await;

            outputs.push(CTxOut {
                value,
                script_length: script.len().try_into().unwrap_or(0),
                script_pubkey: CScript { script },
                index: i,
                address: addresses,
            });
        }
    }

    let _lock_time_buff = reader.read_u32_le().await?;
    //println!("Lock Time: {}", lock_time_buff);
    // Hacky fix for getting proper values/spends/outputs for Sapling
    let _value_count = read_varint(reader).await?;
    let value_balance = reader.read_i64_le().await?;
    //println!("Value: {}", value_balance);
    // Read the SaplingTxData
    let vshielded_spend = parse_vshield_spends(reader).await?;
    let vshielded_output = parse_vshield_outputs(reader).await?;
    // Read the binding_sig as an array of unsigned chars max size 64
    let mut binding_sig = [0u8; 64];
    reader.read_exact(&mut binding_sig).await?;

    // Create and return the SaplingTxData struct
    let sapling_tx_data = SaplingTxData {
        value_balance,
        vshielded_spend,
        vshielded_output,
        binding_sig,
    };

    let serialized_data = bincode::serialize(&sapling_tx_data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let end_pos: u64 = set_end_pos(reader, start_pos).await?;
    let tx_bytes: Vec<u8> = get_txid_bytes(reader, start_pos, end_pos).await?;
    //println!("Tx Bytes: {:?}", hex::encode(&tx_bytes));
    let reversed_txid: Vec<u8> = hash_txid(&tx_bytes).await?;
    //println!("Sapling TXID: {:?}", hex::encode(&reversed_txid));
    //println!("{:?}", sapling_tx_data);

    // CRITICAL FIX: Store internal format (reversed bytes) not hex string
    let mut referenced_utxo_internal: Option<Vec<u8>> = None;
    for tx_in in &inputs {
        let mut key_pubkey = vec![b'p'];
        if let Some(prevout) = &tx_in.prevout {
            // prevout.hash is hex string (display format) - decode and reverse
            if let Ok(hash_bytes) = hex::decode(&prevout.hash) {
                let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
                key_pubkey.extend_from_slice(&internal_hash);
                referenced_utxo_internal = Some(internal_hash);
            }
        }

        if let Ok(Some(data)) = perform_rocksdb_get(_db.clone(), "pubkey", key_pubkey.clone()).await {
            let mut utxos = deserialize_utxos(&data).await;

            if let Some(prevout) = &tx_in.prevout {
                // Compare with internal format bytes
                if let Ok(hash_bytes) = hex::decode(&prevout.hash) {
                    let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
                    if let Some(pos) = utxos.iter().position(|(txid, index)| {
                        *txid == internal_hash.as_slice() && *index == prevout.n as u64
                    }) {
                        utxos.remove(pos);
                    }
                }
            }

            if !utxos.is_empty() {
                perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialize_utxos(&utxos).await).await.ok();
            } else {
                perform_rocksdb_del(_db.clone(), "pubkey", key_pubkey.clone()).await.ok();
            }
        }

        // Only delete UTXO if we have a valid reference (now using internal bytes)
        if let Some(ref internal_hash) = &referenced_utxo_internal {
            let mut key_utxo = vec![b'u'];
            key_utxo.extend_from_slice(internal_hash);
            perform_rocksdb_del(_db.clone(), "utxo", key_utxo).await.ok();
        }
    }

    for tx_out in &outputs {
        let address_type = get_address_type(tx_out, &general_address_type).await;
        if let Err(e) = handle_address(
            _db.clone(),
            &address_type,
            reversed_txid.clone(),
            tx_out.index.try_into().unwrap_or(0),
        ).await {
            eprintln!("Warning: Failed to handle address for output: {:?}", e);
        }

        let mut key_pubkey = vec![b'p'];
        key_pubkey.extend_from_slice(&tx_out.script_pubkey.script);

        let existing_data_option = perform_rocksdb_get(_db.clone(), "pubkey", key_pubkey.clone()).await;
        if let Ok(Some(existing_data)) = existing_data_option {
            let mut existing_utxos = deserialize_utxos(&existing_data).await;
            existing_utxos.push((reversed_txid.clone(), tx_out.index));

            let serialized_utxos = serialize_utxos(&existing_utxos).await;
            perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialized_utxos.clone()).await.ok();
        }

        let mut key_utxo = vec![b'u'];
        key_utxo.extend_from_slice(&reversed_txid);
        let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
        perform_rocksdb_put(_db.clone(), "utxo", key_utxo.clone(), serialize_utxos(&utxos_to_serialize).await).await.ok();
    }

    // 't' + txid -> serialized_data
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    perform_rocksdb_put(_db.clone(), "transactions", key.clone(), serialized_data.clone()).await.ok();

    Ok(sapling_tx_data)
}

#[allow(dead_code)] // Sapling shield parser - reserved for future Sapling support
async fn parse_vshield_spends(reader: &mut BufReader<File>) -> Result<Vec<SpendDescription>, io::Error> {
    // Read the number of vShieldSpend entries
    let count = read_varint(reader).await? as usize;
    //println!("vShieldSpend Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Read each vShieldSpend entry
    let mut vshield_spends = Vec::with_capacity(count);
    for _ in 0..count {
        // Read each field (384 bytes total per spend)
        let mut cv = [0u8; 32];
        reader.read_exact(&mut cv).await?;
        let mut anchor = [0u8; 32];
        reader.read_exact(&mut anchor).await?;
        let mut nullifier = [0u8; 32];
        reader.read_exact(&mut nullifier).await?;
        let mut rk = [0u8; 32];
        reader.read_exact(&mut rk).await?;
        let mut zkproof = [0u8; 192];
        reader.read_exact(&mut zkproof).await?;
        let mut spend_auth_sig = [0u8; 64];
        reader.read_exact(&mut spend_auth_sig).await?;

        // Reverse byte order for hash fields
        cv.reverse();
        anchor.reverse();
        nullifier.reverse();
        rk.reverse();
        // Note: zkproof and spend_auth_sig are NOT reversed

        // Create and return the SpendDescription struct
        let vshield_spend = SpendDescription {
            cv,
            anchor,
            nullifier,
            rk,
            zkproof,
            spend_auth_sig,
        };
        vshield_spends.push(vshield_spend);
    }
    Ok(vshield_spends)
}

#[allow(dead_code)] // Sapling shield parser - reserved for future Sapling support
async fn parse_vshield_outputs(
    reader: &mut BufReader<File>,
) -> Result<Vec<OutputDescription>, io::Error> {
    // Read the number of vShieldOutput entries
    let count = read_varint(reader).await? as usize;
    //println!("vShieldOutput Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Read each vShieldOutput entry (948 bytes total per output)
    let mut vshield_outputs = Vec::with_capacity(count);
    for _ in 0..count {
        // Read each field
        let mut cv = [0u8; 32];
        reader.read_exact(&mut cv).await?;
        let mut cmu = [0u8; 32];
        reader.read_exact(&mut cmu).await?;
        let mut ephemeral_key = [0u8; 32];
        reader.read_exact(&mut ephemeral_key).await?;
        let mut enc_ciphertext = [0u8; 580];
        reader.read_exact(&mut enc_ciphertext).await?;
        let mut out_ciphertext = [0u8; 80];
        reader.read_exact(&mut out_ciphertext).await?;
        let mut zkproof = [0u8; 192];
        reader.read_exact(&mut zkproof).await?;

        // Reverse byte order for hash fields
        cv.reverse();
        cmu.reverse();
        ephemeral_key.reverse();
        // Note: ciphertexts and zkproof are NOT reversed

        // Create and return the OutputDescription struct
        let vshield_output = OutputDescription {
            cv,
            cmu,
            ephemeral_key,
            enc_ciphertext,
            out_ciphertext,
            zkproof,
        };
        vshield_outputs.push(vshield_output);
    }

    Ok(vshield_outputs)
}

#[allow(dead_code)] // Payload parser - reserved for future transaction payload support
async fn parse_payload_data(reader: &mut BufReader<File>) -> Result<Option<Vec<u8>>, io::Error> {
    let mut prefix_found = false;
    let mut byte_count = 0;
    let mut buffer = [0u8; 4];

    // Read byte by byte until the PREFIX is found or the end of the stream is reached
    while !prefix_found && reader.read_exact(&mut buffer).await.is_ok() {
        byte_count += 1;
        if buffer == PREFIX {
            prefix_found = true;
        }
    }

    // Adjust the byte count to exclude the PREFIX sequence
    if prefix_found && byte_count >= PREFIX.len() {
        byte_count -= PREFIX.len();
    }

    // Check if the byte count exceeds the maximum payload size
    if byte_count > MAX_PAYLOAD_SIZE {
        // Handle the case where the payload exceeds the maximum size
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Payload size exceeds the maximum.",
        ))
    } else {
        // Return the payload data as Some if prefix was found, None otherwise
        if prefix_found {
            let mut payload_data = vec![0u8; byte_count];
            reader.read_exact(&mut payload_data).await?;
            Ok(Some(payload_data))
        } else {
            Ok(None)
        }
    }
}

async fn get_txid_bytes<R: AsyncReadExt + AsyncSeek + Unpin>(
    reader: &mut R,
    start_pos: u64,
    end_pos: u64,
) -> Result<Vec<u8>, io::Error> {
    // Calculate tx_size
    let tx_size = (end_pos - start_pos) as usize;
    let mut tx_bytes = vec![0u8; tx_size];
    // Read the transaction bytes
    reader.seek(SeekFrom::Start(start_pos)).await?;
    reader.read_exact(&mut tx_bytes).await?;

    Ok(tx_bytes)
}

async fn read_script<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, io::Error> {
    let script_length = read_varint(reader).await?;
    let mut script = vec![0u8; script_length as usize];
    reader.read_exact(&mut script).await?;
    Ok(script)
}

// Bitcoin normal varint (same as read_varint2)
pub async fn read_varint<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<u64> {
    let first = reader.read_u8().await?;
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(reader.read_u16_le().await?),
        0xfe => u64::from(reader.read_u32_le().await?),
        0xff => reader.read_u64_le().await?,
    };
    Ok(value)
}

/// Priority 1.3: Synchronous varint reader for Cursor-based parsing
#[allow(dead_code)] // Alternative varint reader - may be needed for sync parsing contexts
fn read_varint_sync<R: io::Read>(reader: &mut R) -> io::Result<u64> {
    use byteorder::ReadBytesExt;
    let first = reader.read_u8()?;
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(reader.read_u16::<LittleEndian>()?),
        0xfe => u64::from(reader.read_u32::<LittleEndian>()?),
        0xff => reader.read_u64::<LittleEndian>()?,
    };
    Ok(value)
}

// Bitcoin normal varint
pub async fn read_varint2<R: AsyncReadExt + Unpin + ?Sized>(reader: &mut R) -> io::Result<u64> {
    let first = reader.read_u8().await?; // read first length byte
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(reader.read_u16_le().await?),
        0xfe => u64::from(reader.read_u32_le().await?),
        0xff => reader.read_u64_le().await?,
    };
    Ok(value)
}

// Bitcoin varint128
#[allow(dead_code)] // Alternative varint reader - may be needed for specific encoding contexts
async fn read_varint128(data: &[u8]) -> (usize, u64) {
    let mut index = 0;
    let mut value: u64 = 0;
    let mut byte: u8;

    loop {
        byte = data[index];
        index += 1;
        value = (value << 7) | (byte & 0x7F) as u64;

        if (byte & 0x80) != 0 {
            value += 1;
        }

        if (byte & 0x80) == 0 {
            break;
        }
    }

    (index, value)
}

async fn set_end_pos<R: AsyncReadExt + AsyncSeek + Unpin>(reader: &mut R, start_pos: u64) -> Result<u64, io::Error> {
    let end_pos = reader.stream_position().await?;
    reader.seek(SeekFrom::Start(start_pos)).await?;
    Ok(end_pos)
}

// Helper function to get address type from tx output
async fn get_address_type(tx_out: &CTxOut, general_address_type: &AddressType) -> AddressType {
    if !tx_out.script_pubkey.script.is_empty() {
        scriptpubkey_to_address(&tx_out.script_pubkey).await.unwrap_or_else(|| general_address_type.clone())
    } else {
        general_address_type.clone()
    }
}

// Helper function to handle address indexing
async fn handle_address(
    _db: Arc<DB>,
    address_type: &AddressType,
    reversed_txid: Vec<u8>,
    tx_out_index: u32,
) -> Result<(), io::Error> {
    let address_keys = match address_type {
        AddressType::P2PKH(address) | AddressType::P2SH(address) => vec![address.clone()],
        AddressType::P2PK(pubkey) => vec![pubkey.clone()],
        AddressType::Staking(staker, owner) => vec![staker.clone(), owner.clone()],
        _ => return Ok(()),
    };

    for address_key in &address_keys {
        let mut key_address = vec![b'a'];
        key_address.extend_from_slice(address_key.as_bytes());
        
        let existing_data = perform_rocksdb_get(_db.clone(), "addr_index", key_address.clone()).await;
        let mut existing_utxos = match existing_data {
            Ok(Some(data)) => deserialize_utxos(&data).await,
            _ => Vec::new(),
        };
        
        existing_utxos.push((reversed_txid.clone(), tx_out_index.into()));
        if let Err(e) = perform_rocksdb_put(
            _db.clone(),
            "addr_index",
            key_address,
            serialize_utxos(&existing_utxos).await
        ).await {
            eprintln!("Warning: Failed to add UTXO to address index: {:?}", e);
        }
    }

    Ok(())
}

// Helper function for fast_sync mode: Index addresses only (no spent tracking)
// This is much faster than full UTXO tracking because it doesn't look up previous transactions
#[allow(dead_code)] // Fast-sync optimization - reserved for future performance mode
async fn handle_address_outputs_only(
    _db: Arc<DB>,
    address_type: &AddressType,
    reversed_txid: Vec<u8>,
    tx_out_index: u32,
) -> Result<(), io::Error> {
    let address_keys = match address_type {
        AddressType::P2PKH(address) | AddressType::P2SH(address) => vec![address.clone()],
        AddressType::P2PK(pubkey) => vec![pubkey.clone()],
        AddressType::Staking(staker, owner) => vec![staker.clone(), owner.clone()],
        _ => return Ok(()),
    };

    for address_key in &address_keys {
        let mut key_address = vec![b'a'];
        key_address.extend_from_slice(address_key.as_bytes());
        
        let existing_data = perform_rocksdb_get(_db.clone(), "addr_index", key_address.clone()).await;
        let mut existing_utxos = match existing_data {
            Ok(Some(data)) => deserialize_utxos(&data).await,
            _ => Vec::new(),
        };
        
        existing_utxos.push((reversed_txid.clone(), tx_out_index.into()));
        if let Err(e) = perform_rocksdb_put(
            _db.clone(),
            "addr_index",
            key_address,
            serialize_utxos(&existing_utxos).await
        ).await {
            eprintln!("Warning: Failed to add zerocoin UTXO to address index: {:?}", e);
        }
    }

    Ok(())
}

// Helper function to remove UTXO from address index
async fn remove_utxo_addr(
    _db: Arc<DB>,
    address_type: &AddressType,
    txid: &str,
    index: u32,
) -> Result<(), io::Error> {
    let address_keys = match address_type {
        AddressType::P2PKH(address) | AddressType::P2SH(address) => vec![address.clone()],
        AddressType::P2PK(pubkey) => vec![pubkey.clone()],
        AddressType::Staking(staker, owner) => vec![staker.clone(), owner.clone()],
        _ => return Ok(()),
    };

    for address_key in &address_keys {
        let mut key_address = vec![b'a'];
        key_address.extend_from_slice(address_key.as_bytes());

        // Fetch existing UTXOs associated with this address
        let existing_data = perform_rocksdb_get(_db.clone(), "addr_index", key_address.clone()).await;
        let mut existing_utxos = match existing_data {
            Ok(Some(data)) => deserialize_utxos(&data).await,
            _ => Vec::new(),
        };

        // Find and remove the UTXO
        if let Some(pos) = existing_utxos.iter().position(|(stored_txid, stored_index)| {
            stored_txid.as_slice() == txid.as_bytes() && *stored_index == index as u64
        }) {
            existing_utxos.remove(pos);
        }

        // Update or delete the UTXO entry for this address
        if !existing_utxos.is_empty() {
            if let Err(e) = perform_rocksdb_put(
                _db.clone(),
                "addr_index",
                key_address,
                serialize_utxos(&existing_utxos).await
            ).await {
                eprintln!("Warning: Failed to update address index after UTXO removal: {:?}", e);
            }
        } else {
            perform_rocksdb_del(_db.clone(), "addr_index", key_address).await.ok();
        }
    }

    Ok(())
}