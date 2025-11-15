use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeek, AsyncSeekExt, BufReader, SeekFrom};
use rocksdb::DB;
use crate::batch_writer::BatchWriter;
use crate::db_utils::{perform_rocksdb_get, perform_rocksdb_put, perform_rocksdb_del};
use crate::types::{CTransaction, CTxIn, CTxOut, AddressType, CScript, SpendDescription, OutputDescription, SaplingTxData, COutPoint};
use crate::address::{scriptpubkey_to_address, address_type_to_string};
use crate::parser::{serialize_utxos, deserialize_utxos, deserialize_transaction, reverse_bytes};
use std::error::Error;
use std::io::{self, Cursor};
use serde_json::json;
use byteorder::{LittleEndian, ReadBytesExt};
use tokio::fs::File;
use sha2::{Sha256, Digest};

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const MAX_PAYLOAD_SIZE: usize = 10000;

pub async fn process_transaction(
    mut reader: &mut BufReader<File>,
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
    block_hash: &[u8],
    block_height: Option<i32>,
    tx_index: u64,
    _db: Arc<DB>,
    start_pos: u64,
    batch: &mut BatchWriter,
    fast_sync: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let cf_transactions = Arc::new(_db
        .cf_handle("transactions")
        .expect("Transaction column family not found"));
    let cf_pubkey = Arc::new(_db
        .cf_handle("pubkey")
        .expect("Pubkey column family not found"));
    let cf_utxo = Arc::new(_db
        .cf_handle("utxo")
        .expect("UTXO column family not found"));
    
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
            index: i as u64,
            coinbase,
        });
    }


    let output_count = read_varint(reader).await?;
    let mut general_address_type = if input_count == 1 && output_count == 1 {
        AddressType::CoinBaseTx
    } else if output_count > 1 {
        AddressType::CoinStakeTx
    } else {
        AddressType::Nonstandard
    };

    let mut outputs = Vec::new();
    for i in 0..output_count {
        let value = reader.read_i64_le().await?;
        let script = read_script(reader).await?;
        let address_type = get_address_type(&CTxOut {
            value,
            script_length: script.len().try_into().unwrap_or(0),
            script_pubkey: CScript { script: script.clone() },
            index: i as u64,
            address: Vec::new(),
        }, &general_address_type.clone()).await;

        let addresses = address_type_to_string(Some(address_type.clone()));
        outputs.push(CTxOut {
            value,
            script_length: script.len().try_into().unwrap_or(0),
            script_pubkey: CScript { script },
            index: i as u64,
            address: addresses.await,
        });
    }

    let lock_time_buff = reader.read_u32_le().await?;

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
    
    // Read transaction bytes for TXID calculation
    let tx_bytes = get_txid_bytes(reader, start_pos, end_pos).await?;

    let reversed_txid = hash_txid(&tx_bytes).await?;

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
                perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialized_utxos).await;
            }
            // Create a UTXO identifier (txid + output index)
            let mut key_utxo = vec![b'u'];
            key_utxo.extend_from_slice(&hex::encode(&reversed_txid).into_bytes());
            let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
            perform_rocksdb_put(_db.clone(), "utxo", key_utxo.clone(), serialize_utxos(&utxos_to_serialize).await).await;
        }

        for tx_in in &transaction.inputs {
            let mut key = vec![b't'];
            if let Some(actual_prevout) = &tx_in.prevout {
                key.extend_from_slice(&actual_prevout.hash.as_bytes());
            }
            let tx_data_option = perform_rocksdb_get(_db.clone(), "transactions", key.clone()).await;
            if let Ok(Some(tx_data)) = tx_data_option {
                // Safely deserialize - skip if corrupted
                if let Ok(referenced_transaction) = deserialize_transaction(&tx_data).await {
                    if let Some(prevout) = &tx_in.prevout {
                        let output = &referenced_transaction.outputs[prevout.n as usize];
                        let address_type = get_address_type(output, &general_address_type).await;

                        let _ = remove_utxo_addr(_db.clone(), &address_type, &prevout.hash, prevout.n).await;
                    }
                }
            }
            
            let mut key_utxo = vec![b'u'];
            if let Some(actual_prevout) = &tx_in.prevout {
                key_utxo.extend_from_slice(&actual_prevout.hash.as_bytes());
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
                    perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialize_utxos(&utxos).await).await;
                } else {
                    perform_rocksdb_del(_db.clone(), "pubkey", key_pubkey.clone()).await;
                }
            }

            // Remove the referenced UTXO from the UTXO set
            perform_rocksdb_del(_db.clone(), "utxo", key_utxo.clone()).await;
        }
    } else {
        // FAST SYNC MODE: Index addresses only (no spent tracking)
        // This is 10-15x faster because we don't look up previous transactions
        // We still index addresses so they can be searched, but totalSent will be 0
        for tx_out in &transaction.outputs {
            let address_type = get_address_type(tx_out, &general_address_type).await;
            
            // Index address without checking if UTXOs are spent
            handle_address_outputs_only(
                _db.clone(),
                &address_type,
                reversed_txid.clone(),
                tx_out.index.try_into().unwrap_or(0),
            ).await?;
        }
    } // End UTXO tracking / address indexing

    // Store transaction with proper indexing
    // 1. Main transaction storage: 't' + txid → (block_version + block_height + tx_bytes)
    let mut tx_key = vec![b't'];
    tx_key.extend_from_slice(&reversed_txid);
    
    let version_bytes = block_version.to_le_bytes().to_vec();
    let height_bytes = block_height.unwrap_or(0).to_le_bytes().to_vec();
    
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
    let first_hash = Sha256::digest(&tx_bytes);
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

async fn parse_sapling_tx_data(
    reader: &mut BufReader<File>,
    start_pos: u64,
    _db: Arc<DB>,
    batch: &mut BatchWriter,
) -> Result<SaplingTxData, io::Error> {
    let cf_transactions = Arc::new(_db
        .cf_handle("transactions")
        .expect("Transaction column family not found"));
    let cf_pubkey = Arc::new(_db
        .cf_handle("pubkey")
        .expect("Pubkey column family not found"));
    let cf_utxo = Arc::new(_db
        .cf_handle("utxo")
        .expect("UTXO column family not found"));

    // Set empty vectors for later access
    let mut inputs: Vec<CTxIn> = Vec::new();
    let mut outputs: Vec<CTxOut> = Vec::new();
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

    let lock_time_buff = reader.read_u32_le().await?;
    //println!("Lock Time: {}", lock_time_buff);
    // Hacky fix for getting proper values/spends/outputs for Sapling
    let value_count = read_varint(reader).await?;
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

    let mut referenced_utxo_id: Option<String> = None;
    for tx_in in &inputs {
        let mut key_pubkey = vec![b'p'];
        if let Some(prevout) = &tx_in.prevout {
            key_pubkey.extend_from_slice(&prevout.hash.as_bytes());
            referenced_utxo_id = Some(hex::encode(&prevout.hash));
        }

        if let Ok(Some(data)) = perform_rocksdb_get(_db.clone(), "pubkey", key_pubkey.clone()).await {
            let mut utxos = deserialize_utxos(&data).await;

            if let Some(prevout) = &tx_in.prevout {
                if let Some(pos) = utxos.iter().position(|(txid, index)| {
                    *txid == prevout.hash.as_bytes() && *index == prevout.n as u64
                }) {
                    utxos.remove(pos);
                }
            }

            if !utxos.is_empty() {
                perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialize_utxos(&utxos).await).await;
            } else {
                perform_rocksdb_del(_db.clone(), "pubkey", key_pubkey.clone()).await;
            }
        }

        // Only delete UTXO if we have a valid reference
        if let Some(ref_id) = &referenced_utxo_id {
            let mut key_utxo = vec![b'u'];
            key_utxo.extend_from_slice(ref_id.as_bytes());
            perform_rocksdb_del(_db.clone(), "utxo", key_utxo).await;
        }
    }

    for tx_out in &outputs {
        let address_type = get_address_type(tx_out, &general_address_type).await;
        handle_address(
            _db.clone(),
            &address_type,
            reversed_txid.clone(),
            tx_out.index.try_into().unwrap_or(0),
        ).await;

        let mut key_pubkey = vec![b'p'];
        key_pubkey.extend_from_slice(&tx_out.script_pubkey.script);

        let existing_data_option = perform_rocksdb_get(_db.clone(), "pubkey", key_pubkey.clone()).await;
        if let Ok(Some(existing_data)) = existing_data_option {
            let mut existing_utxos = deserialize_utxos(&existing_data).await;
            existing_utxos.push((reversed_txid.clone(), tx_out.index));

            let serialized_utxos = serialize_utxos(&existing_utxos).await;
            perform_rocksdb_put(_db.clone(), "pubkey", key_pubkey.clone(), serialized_utxos.clone()).await;
        }

        let mut key_utxo = vec![b'u'];
        key_utxo.extend_from_slice(&reversed_txid);
        let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
        perform_rocksdb_put(_db.clone(), "utxo", key_utxo.clone(), serialize_utxos(&utxos_to_serialize).await).await;
    }

    // 't' + txid -> serialized_data
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    perform_rocksdb_put(_db.clone(), "transactions", key.clone(), serialized_data.clone()).await;

    Ok(sapling_tx_data)
}

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
        perform_rocksdb_put(
            _db.clone(),
            "addr_index",
            key_address,
            serialize_utxos(&existing_utxos).await
        ).await;
    }

    Ok(())
}

// Helper function for fast_sync mode: Index addresses only (no spent tracking)
// This is much faster than full UTXO tracking because it doesn't look up previous transactions
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
        perform_rocksdb_put(
            _db.clone(),
            "addr_index",
            key_address,
            serialize_utxos(&existing_utxos).await
        ).await;
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
            perform_rocksdb_put(
                _db.clone(),
                "addr_index",
                key_address,
                serialize_utxos(&existing_utxos).await
            ).await;
        } else {
            perform_rocksdb_del(_db.clone(), "addr_index", key_address).await;
        }
    }

    Ok(())
}