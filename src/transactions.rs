use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeek, AsyncSeekExt, BufReader, SeekFrom};
use rocksdb::DB;
use crate::db_utils::{RocksDBOperations, perform_rocksdb_get, perform_rocksdb_put, perform_rocksdb_del};
use crate::blocks::CBlockHeader;
use crate::types::{CTransaction, CTxIn, CTxOut, AddressType, CScript};
use crate::address::{compute_address_hash, hash_address, scriptpubkey_to_address, address_type_to_string};
use std::error::Error;
use futures::stream::{self, StreamExt};
use std::io::Cursor;
use crate::api::VarInt;
use serde_json::json;
use byteorder::{LittleEndian, ReadBytesExt};
use tokio::fs::File;

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const MAX_PAYLOAD_SIZE: usize = 10000;
async fn process_transaction(
    mut reader: &mut BufReader<File>,
    block_version: u32,
    block_hash: &[u8],
    _db: Arc<DB>,
) -> Result<(), io::Error> {
    let tx_amt = read_varint(reader).await?;
    for _ in 0..tx_amt {
        let start_pos = reader.stream_position().await?;

        let tx_ver_out = reader.read_u16_le().await?;
        let tx_type = reader.read_u16_le().await?;

        if block_version == 11 {
            if tx_ver_out < 3 {
                process_transaction_v1(
                    reader,
                    tx_ver_out.try_into().unwrap(),
                    block_version,
                    block_hash,
                    _db.clone(),
                    start_pos,
                ).await;
            } else {
                parse_sapling_tx_data(reader, start_pos, _db.clone()).await?;
            }
        } else if (tx_ver_out <= 2 && block_version < 11) || (tx_ver_out > 1 && block_version > 7) {
            if tx_ver_out <= 2 {
                process_transaction_v1(
                    reader,
                    tx_ver_out.try_into().unwrap(),
                    block_version,
                    block_hash,
                    _db.clone(),
                    start_pos,
                ).await;
            } else {
                parse_sapling_tx_data(reader, start_pos, _db.clone()).await?;
            }
        }
    }
    Ok(())
}

async fn process_transaction_v1(
    reader: &mut (impl AsyncReadExt + AsyncSeek + Unpin),
    tx_ver_out: i16,
    block_version: u32,
    block_hash: &[u8],
    _db: Arc<DB>,
    start_pos: u64,
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
            script_length: script.len().try_into().unwrap(),
            script_pubkey: CScript { script: script.clone() },
            index: i as u64,
            address: Vec::new(),
        }, &general_address_type.clone()).await;

        let addresses = address_type_to_string(Some(address_type.clone()));
        outputs.push(CTxOut {
            value,
            script_length: script.len().try_into().unwrap(),
            script_pubkey: CScript { script },
            index: i as u64,
            address: addresses.await,
        });
    }

    let lock_time_buff = reader.read_u32_le().await?;

    reader.seek(SeekFrom::Start(start_pos)).await?; // Find starting position
    let mut tx_bytes = vec![];
    let current_position = reader.stream_position().await?; // Get current position and ensure this operation completes
    let mut some_buffer = Vec::new();
    let end_pos = reader.take((start_pos + (output_count * 4) as u64 - current_position).into()).read_to_end(&mut some_buffer).await?;

    let reversed_txid = hash_txid(&tx_bytes).await?;

    let transaction = CTransaction {
        txid: hex::encode(reversed_txid.clone()),
        version: tx_ver_out,
        inputs,
        outputs: outputs.clone(),
        lock_time: lock_time_buff,
    };

    //println!("Transaction ID: {:?}", hex::encode(&reversed_txid));

    let mut key_pubkey = vec![b'p'];
    for tx_out in &transaction.outputs {
        let address_type = get_address_type(tx_out, &general_address_type).await;

        // Associate by these with UTXO set
        handle_address(
            _db.clone(),
            &address_type,
            reversed_txid.clone(),
            tx_out.index.try_into().unwrap(),
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
        let spent_output: Option<&CTxOut> = None;
        let tx_data_option = perform_rocksdb_get(_db.clone(), "transactions", key.clone()).await;
        if let Ok(Some(tx_data)) = tx_data_option {
            let referenced_transaction = deserialize_transaction(&tx_data).await.unwrap();

            if let Some(prevout) = &tx_in.prevout {
                let output = &referenced_transaction.outputs[prevout.n as usize];
                let address_type = get_address_type(output, &general_address_type).await;

                let _ = remove_utxo_addr(_db.clone(), &address_type, &prevout.hash, prevout.n).await;
            }
        }
        let mut key_utxo = vec![b'u'];
        if let Some(actual_prevout) = &tx_in.prevout {
            key.extend_from_slice(&actual_prevout.hash.as_bytes());
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

    // 't' + txid -> tx_bytes
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    let mut version_bytes = vec![0u8; 4];
    LittleEndian::write_u32(&mut version_bytes, block_version);
    version_bytes.extend(&tx_bytes);
    perform_rocksdb_put(_db.clone(), "transactions", key.clone(), version_bytes.clone()).await;
    reader.seek(SeekFrom::Start(end_pos.try_into().unwrap())).await?;

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

    let inputs = if input_count > 0 {
        stream::iter(0..input_count)
            .then(|i| async move {
                let prev_output = read_outpoint(reader).await?;
                let script = read_script(reader).await?;
                let sequence = reader.read_u32_le().await?;
                Ok::CTxIn(CTxIn {
                    prevout: Some(prev_output),
                    script_sig: CScript { script },
                    sequence,
                    index: i,
                    coinbase: None,
                })
            })
            .try_collect::<Vec<_>>()
            .await?
    } else {
        Vec::new()
    };

    let output_count = read_varint(reader).await? as u64;
    //println!("Output Count: {}", output_count);
    let mut general_address_type = if input_count == 1 && output_count == 1 {
        AddressType::CoinBaseTx
    } else if output_count > 1 {
        AddressType::CoinStakeTx
    } else {
        AddressType::Nonstandard
    };

    let outputs = if output_count > 0 {
        stream::iter(0..output_count)
            .then(|i| async move {
                let value = reader.read_i64_le().await?;
                let script = read_script(reader).await?;
                let address_type = get_address_type(
                    &CTxOut {
                        value,
                        script_length: script.len().try_into().unwrap(),
                        script_pubkey: CScript { script: script.clone() },
                        index: i,
                        address: Vec::new(), // Temporary dummy value
                    },
                    &general_address_type,
                ).await;
                let addresses = address_type_to_string(Some(address_type.clone())).await;

                Ok(CTxOut {
                    value,
                    script_length: script.len().try_into().unwrap(),
                    script_pubkey: CScript { script },
                    index: i,
                    address: addresses, // directly assign the Vec<String>
                })
            })
            .try_collect::<Vec<_>>()
            .await?
    } else {
        Vec::new()
    };

    let lock_time_buff = reader.read_u32_le().await?;
    //println!("Lock Time: {}", lock_time_buff);
    // Hacky fix for getting proper values/spends/outputs for Sapling
    let value_count = read_varint(reader).await?;
    let value = reader.read_i64_le().await?;
    //println!("Value: {}", value);
    // Read the SaplingTxData
    let vshield_spend = parse_vshield_spends(reader).await?;
    let vshield_output = parse_vshield_outputs(reader).await?;
    // Read the binding_sig as an array of unsigned chars max size 64
    let mut binding_sig = [0u8; 64];
    reader.read_exact(&mut binding_sig).await?;

    // Create and return the SaplingTxData struct
    let sapling_tx_data = SaplingTxData {
        value,
        vshield_spend,
        vshield_output,
        binding_sig: binding_sig.to_vec(),
    };

    let serialized_data = bincode::serialize(&sapling_tx_data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let end_pos: u64 = set_end_pos(reader, start_pos).await?;
    let tx_bytes: Vec<u8> = get_txid_bytes(reader, start_pos, end_pos).await?;
    //println!("Tx Bytes: {:?}", hex::encode(&tx_bytes));
    let reversed_txid: Vec<u8> = hash_txid(&tx_bytes).await?;
    //println!("Sapling TXID: {:?}", hex::encode(&reversed_txid));
    //println!("{:?}", sapling_tx_data);

    let mut key_utxo = vec![b'u'];
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

        key_utxo.extend_from_slice(referenced_utxo_id.as_ref().unwrap().as_bytes());
        _db.delete_cf(&cf_utxo.clone(), &key_utxo).unwrap();
    }

    for tx_out in &outputs {
        let address_type = get_address_type(tx_out, &general_address_type).await;
        handle_address(
            _db,
            &address_type,
            reversed_txid.clone(),
            tx_out.index.try_into().unwrap(),
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

        key_utxo.extend_from_slice(&reversed_txid);
        let utxos_to_serialize = vec![(reversed_txid.clone(), tx_out.index)];
        perform_rocksdb_put(_db.clone(), "pubkey", key_utxo.clone(), serialize_utxos(&utxos_to_serialize).await).await;
    }

    // 't' + txid -> serialized_data
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed_txid);
    perform_rocksdb_put(_db.clone(), "transactions", key.clone(), serialized_data.clone()).await;

    Ok(sapling_tx_data)
}

async fn parse_vshield_spends(reader: &mut BufReader<File>) -> Result<Vec<VShieldSpend>, io::Error> {
    // Read the number of vShieldSpend entries
    let count = read_varint(reader).await? as usize;
    //println!("vShieldSpend Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Define buffer sizes for respective fields
    let buff_32 = [0u8; 32];
    let buff_64 = [0u8; 64];
    let buff_192 = [0u8; 192];

    // Read each vShieldSpend entry
    let mut vshield_spends = Vec::with_capacity(count.try_into().unwrap());
    for _ in 0..count {
        // Read each field
        let mut cv = buff_32;
        reader.read_exact(&mut cv).await?;
        let mut anchor = buff_32;
        reader.read_exact(&mut anchor).await?;
        let mut nullifier = buff_32;
        reader.read_exact(&mut nullifier).await?;
        let mut rk = buff_32;
        reader.read_exact(&mut rk).await?;
        let mut proof = buff_192;
        reader.read_exact(&mut proof).await?;
        let mut spend_auth_sig = buff_64;
        reader.read_exact(&mut spend_auth_sig).await?;

        // Create and return the VShieldSpend struct
        let vshield_spend = VShieldSpend {
            cv: reverse_bytes(&cv).await,
            anchor: reverse_bytes(&anchor).await,
            nullifier: reverse_bytes(&nullifier).await,
            rk: reverse_bytes(&rk).await,
            proof: proof.to_vec(),
            spend_auth_sig: spend_auth_sig.to_vec(),
        };
        vshield_spends.push(vshield_spend);
        //println!("{:?}", vshield_spends);
    }
    Ok(vshield_spends)
}

async fn parse_vshield_outputs(
    reader: &mut BufReader<File>,
) -> Result<Vec<VShieldOutput>, io::Error> {
    // Read the number of vShieldOutput entries
    let count = read_varint(reader).await? as usize;
    //println!("vShieldOutput Count: {}", count);
    if count == 0 {
        return Ok(Vec::new());
    }

    // Define buffer sizes for respective fields
    let buff_32 = [0u8; 32];
    let buff_80 = [0u8; 80];
    let buff_192 = [0u8; 192];
    let buff_580 = [0u8; 580];

    // Read each vShieldOutput entry
    let mut vshield_outputs = Vec::with_capacity(count);
    for _ in 0..count {
        // Read each field
        let mut cv = buff_32;
        reader.read_exact(&mut cv).await?;
        let mut cmu = buff_32;
        reader.read_exact(&mut cmu).await?;
        let mut ephemeral_key = buff_32;
        reader.read_exact(&mut ephemeral_key).await?;
        let mut enc_ciphertext = buff_580;
        reader.read_exact(&mut enc_ciphertext).await?;
        let mut out_ciphertext = buff_80;
        reader.read_exact(&mut out_ciphertext).await?;
        let mut proof = buff_192;
        reader.read_exact(&mut proof).await?;

        // Create and return the VShieldOutput struct
        let vshield_output = VShieldOutput {
            cv: reverse_bytes(&cv).await,
            cmu: reverse_bytes(&cmu).await,
            ephemeral_key: reverse_bytes(&ephemeral_key).await,
            enc_ciphertext: enc_ciphertext.to_vec(),
            out_ciphertext: out_ciphertext.to_vec(),
            proof: proof.to_vec(),
        };
        vshield_outputs.push(vshield_output);
        //println!("{:?}", vshield_outputs);
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

pub async fn read_varint<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<u64> {
    let varint = VarInt::consensus_decode(reader)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(varint.0)
}

// Bitcoin normal varint
pub async fn read_varint2<R: AsyncReadExt + Unpin + ?Sized>(reader: &mut R) -> io::Result<u64> {
    let first = reader.read_u8().await?; // read first length byte
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(reader.read_u16_le().await?),
        0xfe => u64::from(reader.read_u32_le().await?),
        0xff => reader.read_u64_le().await?,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid varint")),
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