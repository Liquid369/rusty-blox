use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::types::{CTransaction, CTxIn, CTxOut, COutPoint, CScript, SaplingTxData, SpendDescription, OutputDescription};
use sha2::{Sha256, Digest};
use ripemd160::{Ripemd160};
use bs58;
use tracing::warn;

// Consensus limits to prevent DoS attacks via massive transactions
// Bitcoin Core MAX_BLOCK_WEIGHT / MIN_TRANSACTION_WEIGHT = 400,000 theoretical max
// PIVX has similar limits - use conservative 100,000 for safety
const MAX_TX_INPUTS: u64 = 100_000;
const MAX_TX_OUTPUTS: u64 = 100_000;
const MAX_SAPLING_SPENDS: u64 = 5_000;
const MAX_SAPLING_OUTPUTS: u64 = 5_000;

pub async fn reverse_bytes(array: &[u8]) -> Vec<u8> {
    let mut vec = Vec::from(array);
    vec.reverse();
    vec
}

pub async fn serialize_utxos(utxos: &Vec<(Vec<u8>, u64)>) -> Vec<u8> {
    let mut serialized = Vec::new();
    for (txid, index) in utxos {
        serialized.extend(txid);
        serialized.extend(&index.to_le_bytes());
    }
    serialized
}

// Serialize UTXOs with spent flags
pub async fn serialize_utxos_with_spent(utxos: &Vec<(Vec<u8>, u64, bool)>) -> Vec<u8> {
    let mut buffer = Vec::new();
    // Write count
    buffer.extend(&(utxos.len() as u32).to_le_bytes());
    
    for (txid, vout, is_spent) in utxos {
        // Write txid length and txid
        buffer.extend(&(txid.len() as u32).to_le_bytes());
        buffer.extend(txid);
        // Write vout
        buffer.extend(&vout.to_le_bytes());
        // Write spent flag (1 byte: 0 = unspent, 1 = spent)
        buffer.push(if *is_spent { 1 } else { 0 });
    }
    buffer
}

pub async fn deserialize_utxos(data: &[u8]) -> Vec<(Vec<u8>, u64)> {
    let mut utxos = Vec::new();
    let iter = data.chunks_exact(40); // 32 bytes for txid and 8 bytes for index
    for chunk in iter {
        let txid = chunk[0..32].to_vec();
        if let Ok(bytes) = <[u8; 8]>::try_from(&chunk[32..40]) {
            let index = u64::from_le_bytes(bytes);
            utxos.push((txid, index));
        }
    }
    utxos
}

// Deserialize UTXOs with spent flags
pub async fn deserialize_utxos_with_spent(data: &[u8]) -> Vec<(Vec<u8>, u64, bool)> {
    let mut utxos = Vec::new();
    if data.len() < 4 {
        return utxos;
    }
    
    let mut cursor = std::io::Cursor::new(data);
    use byteorder::ReadBytesExt;
    
    // Read count
    let count = match cursor.read_u32::<byteorder::LittleEndian>() {
        Ok(c) => c,
        Err(_) => return utxos,
    };
    
    // CRITICAL: Validate count to prevent memory exhaustion
    // A reasonable maximum is 1 million UTXOs per address
    if count > 1_000_000 {
        warn!(count = count, "Invalid UTXO count (too large, data likely corrupted)");
        return utxos;
    }
    
    for _ in 0..count {
        // Read txid length
        let txid_len = match cursor.read_u32::<byteorder::LittleEndian>() {
            Ok(len) => len as usize,
            Err(_) => break,
        };
        
        // CRITICAL: Validate txid_len before allocation to prevent memory exhaustion
        // TXID should always be 32 bytes for Bitcoin-derived chains
        if txid_len == 0 || txid_len > 64 {
            warn!(txid_len = txid_len, "Invalid txid length (expected 32, data likely corrupted)");
            break;
        }
        
        // Read txid
        let mut txid = vec![0u8; txid_len];
        if std::io::Read::read_exact(&mut cursor, &mut txid).is_err() {
            break;
        }
        
        // Read vout
        let vout = match cursor.read_u64::<byteorder::LittleEndian>() {
            Ok(v) => v,
            Err(_) => break,
        };
        
        // Read spent flag
        let is_spent = match cursor.read_u8() {
            Ok(flag) => flag == 1,
            Err(_) => break,
        };
        
        utxos.push((txid, vout, is_spent));
    }
    
    utxos
}

pub async fn deserialize_transaction(
    data: &[u8],
) -> Result<CTransaction, std::io::Error> {
    let txid = hash_txid(&data[4..]).await;
    let mut cursor = Cursor::new(data);
    let block_version = cursor.read_u32::<LittleEndian>()?;

    let version = cursor.read_u16::<LittleEndian>()?;
    let tx_type = cursor.read_u16::<LittleEndian>()?;
    
    let input_count = read_varint(&mut cursor).await?;
    if input_count > MAX_TX_INPUTS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Transaction input count {} exceeds maximum {}", input_count, MAX_TX_INPUTS)
        ));
    }
    // [M1] Optimize: Pre-allocate with known capacity to avoid reallocations
    let mut inputs = Vec::with_capacity(input_count as usize);
    
    // Determine transaction type based on PIVX rules
    // Coinbase: 1 input with prev_hash all zeros, 1 output
    // Coinstake: 1+ inputs with first prev_hash all zeros, 2+ outputs (first output is empty)
    // Normal: regular transaction
    // Sapling: version >= 3
    
    let is_sapling = version >= 3;
    
    for i in 0..input_count {
        inputs.push(deserialize_tx_in(
            &mut cursor,
            version as u32,
            block_version,
            i == 0, // is_first_input
        ).await?);
    }

    let output_count = read_varint(&mut cursor).await?;
    if output_count > MAX_TX_OUTPUTS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Transaction output count {} exceeds maximum {}", output_count, MAX_TX_OUTPUTS)
        ));
    }
    
    // Identify transaction type based on PIVX rules:
    // - Coinbase: first input has coinbase data (prev_hash all zeros), typically has multiple outputs
    // - Coinstake: first input has coinstake data, first output is ALWAYS empty (0 value, empty script)
    // - Normal: regular transaction with no coinbase/coinstake inputs
    
    // [M1] Optimize: Pre-allocate with known capacity
    let mut outputs = Vec::with_capacity(output_count as usize);
    for i in 0..output_count {
        let mut output = deserialize_tx_out(&mut cursor, false).await?;
        output.index = i;  // Set the correct vout index
        outputs.push(output);
    }

    let lock_time = cursor.read_u32::<LittleEndian>()?;

    // For Sapling transactions (version >= 3), parse the Sapling-specific data.
    // PIVX Core serializes sapData as Optional<SaplingTxData>: a 1-byte discriminant
    // (0x00 = absent, 0x01 = present) followed by the payload when present.
    let sapling_data = if is_sapling {
        let has_sap_data = cursor.read_u8()? != 0;
        if !has_sap_data {
            // Optional discriminant 0x00 — no sapling payload. extraPayload (if any)
            // is handled below.
            read_extra_payload(&mut cursor, tx_type, data.len()).await?;
            None
        } else {
        // Read valueBalance (net value of spends - outputs)
        let value_balance = cursor.read_i64::<LittleEndian>()?;
        
        // Read vShieldedSpend count and parse each spend (384 bytes each)
        let spend_count = read_varint(&mut cursor).await?;
        if spend_count > MAX_SAPLING_SPENDS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Sapling spend count {} exceeds maximum {}", spend_count, MAX_SAPLING_SPENDS)
            ));
        }
        // [M1] Optimize: Pre-allocate for Sapling spends
        let mut vshielded_spend = Vec::with_capacity(spend_count as usize);
        for _ in 0..spend_count {
            // Read each spend field (SPENDDESCRIPTION_SIZE = 384 bytes)
            let mut cv = [0u8; 32];           // Value commitment
            let mut anchor = [0u8; 32];       // Merkle tree root
            let mut nullifier = [0u8; 32];    // Prevents double-spending
            let mut rk = [0u8; 32];           // Randomized public key
            let mut zkproof = [0u8; 192];     // Groth16 zero-knowledge proof
            let mut spend_auth_sig = [0u8; 64]; // Spend authorization signature
            
            cursor.read_exact(&mut cv)?;
            cursor.read_exact(&mut anchor)?;
            cursor.read_exact(&mut nullifier)?;
            cursor.read_exact(&mut rk)?;
            cursor.read_exact(&mut zkproof)?;
            cursor.read_exact(&mut spend_auth_sig)?;
            
            // Reverse byte order for hash fields (network -> display format)
            cv.reverse();
            anchor.reverse();
            nullifier.reverse();
            rk.reverse();
            // Note: zkproof and spend_auth_sig are NOT reversed (kept in original order)
            
            vshielded_spend.push(SpendDescription {
                cv,
                anchor,
                nullifier,
                rk,
                zkproof,
                spend_auth_sig,
            });
        }
        
        // Read vShieldedOutput count and parse each output (948 bytes each)
        let output_count = read_varint(&mut cursor).await?;
        if output_count > MAX_SAPLING_OUTPUTS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Sapling output count {} exceeds maximum {}", output_count, MAX_SAPLING_OUTPUTS)
            ));
        }
        // [M1] Optimize: Pre-allocate for Sapling outputs
        let mut vshielded_output = Vec::with_capacity(output_count as usize);
        for _ in 0..output_count {
            // Read each output field (OUTPUTDESCRIPTION_SIZE = 948 bytes)
            let mut cv = [0u8; 32];           // Value commitment
            let mut cmu = [0u8; 32];          // Note commitment u-coordinate
            let mut ephemeral_key = [0u8; 32]; // Ephemeral Jubjub public key
            let mut enc_ciphertext = [0u8; 580]; // Encrypted note for recipient
            let mut out_ciphertext = [0u8; 80];  // Encrypted note for sender OVK
            let mut zkproof = [0u8; 192];     // Groth16 zero-knowledge proof
            
            cursor.read_exact(&mut cv)?;
            cursor.read_exact(&mut cmu)?;
            cursor.read_exact(&mut ephemeral_key)?;
            cursor.read_exact(&mut enc_ciphertext)?;
            cursor.read_exact(&mut out_ciphertext)?;
            cursor.read_exact(&mut zkproof)?;
            
            // Reverse byte order for hash fields (network -> display format)
            cv.reverse();
            cmu.reverse();
            ephemeral_key.reverse();
            // Note: ciphertexts and zkproof are NOT reversed (kept in original order)
            
            vshielded_output.push(OutputDescription {
                cv,
                cmu,
                ephemeral_key,
                enc_ciphertext,
                out_ciphertext,
                zkproof,
            });
        }
        
        // Read bindingSig (BINDINGSIG_SIZE = 64 bytes)
        let mut binding_sig = [0u8; 64];
        cursor.read_exact(&mut binding_sig)?;

        // For special transaction types (nType != 0), consume extraPayload
        read_extra_payload(&mut cursor, tx_type, data.len()).await?;

        Some(SaplingTxData {
            value_balance,
            vshielded_spend,
            vshielded_output,
            binding_sig,
        })
        }
    } else {
        None
    };

    Ok(CTransaction {
        txid,
        version: version as i16,
        inputs,
        outputs,
        lock_time,
        sapling_data,
    })
}

/// Consume the extraPayload field of a special transaction (nType != 0).
///
/// PIVX Core serializes extraPayload as Optional<std::vector<unsigned char>>:
/// a 1-byte discriminant (0x00 = absent, 0x01 = present), then CompactSize length
/// + payload bytes when present. Reading the discriminant as a length (the old
/// behavior) would leave the entire payload unconsumed and misalign every
/// subsequent transaction in the block.
async fn read_extra_payload(
    cursor: &mut Cursor<&[u8]>,
    tx_type: u16,
    data_len: usize,
) -> Result<(), std::io::Error> {
    if tx_type == 0 {
        return Ok(());
    }
    let has_payload = cursor.read_u8()? != 0;
    if has_payload {
        let payload_size = read_varint(cursor).await?;
        let new_pos = cursor
            .position()
            .checked_add(payload_size)
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "extraPayload size overflow",
            ))?;
        if new_pos > data_len as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("extraPayload size {} exceeds transaction bounds", payload_size),
            ));
        }
        cursor.set_position(new_pos);
    }
    Ok(())
}

/// Blocking wrapper for `deserialize_transaction` for use in synchronous contexts.
/// This executes the async parser to completion on the current thread.
/// Prefer using `deserialize_transaction(...).await` in async contexts.
pub fn deserialize_transaction_blocking(data: &[u8]) -> Result<CTransaction, std::io::Error> {
    // Use the futures executor to run the async function to completion synchronously.
    futures::executor::block_on(deserialize_transaction(data))
}

pub async fn deserialize_tx_in(
    cursor: &mut Cursor<&[u8]>, 
    _tx_ver_out: u32, 
    _block_version: u32,
    _is_first_input: bool,
) -> Result<CTxIn, std::io::Error> {
    // Standard Bitcoin/PIVX transaction input format:
    // - prev_hash (32 bytes)
    // - prev_index (4 bytes)
    // - script_sig_len (varint)
    // - script_sig (variable)
    // - sequence (4 bytes)
    
    let mut prev_hash = [0u8; 32];
    cursor.read_exact(&mut prev_hash)?;
    let prev_index = cursor.read_u32::<LittleEndian>()?;
    
    let script_sig = read_script(cursor).await?;
    let sequence = cursor.read_u32::<LittleEndian>()?;
    
    // Check if this is TRULY coinbase: prev_hash is all zeros AND prev_index is 0xffffffff,
    // AND the scriptSig is not a zerocoin spend. PIVX Core: zerocoin spends (OP_ZEROCOINSPEND
    // 0xc2 / OP_ZEROCOINPUBLICSPEND 0xc3) carry a null prevout but are NOT coinbase
    // (IsCoinBase() requires !ContainsZerocoins()).
    // CRITICAL: Coinstake transactions have REAL prevouts, not null!
    let is_null_prevout = prev_hash.iter().all(|&b| b == 0) && prev_index == 0xffffffff;
    let is_zerocoin_script = matches!(script_sig.first(), Some(&0xc2) | Some(&0xc3));
    let is_coinbase = is_null_prevout && !is_zerocoin_script;

    // Always create prevout structure - even for coinbase-like inputs
    // We'll determine later if it's truly coinbase or just coinstake
    let mut hash_display = prev_hash;
    hash_display.reverse();

    let prevout = Some(COutPoint {
        hash: hex::encode(hash_display),
        n: prev_index,
    });

    if is_coinbase {
        // TRUE coinbase - has null prevout (all zeros)
        Ok(CTxIn {
            prevout,
            script_sig: CScript { script: Vec::new() },
            sequence,
            index: 0,
            coinbase: Some(script_sig),
        })
    } else {
        // Regular input OR coinstake input - both have real prevouts!
        Ok(CTxIn {
            prevout,
            script_sig: CScript { script: script_sig },
            sequence,
            index: 0,
            coinbase: None,  // NOT coinbase - has real prevout
        })
    }
}

pub async fn deserialize_tx_out(cursor: &mut Cursor<&[u8]>, _is_coinstake_empty: bool) -> Result<CTxOut, std::io::Error> {
    // Standard Bitcoin/PIVX transaction output format:
    // - value (8 bytes, i64)
    // - script_pubkey_len (varint)
    // - script_pubkey (variable)
    
    let value = cursor.read_i64::<LittleEndian>()?;
    let script_pubkey = read_script(cursor).await?;
    
    // Extract address from script if it's not empty
    let address = if script_pubkey.is_empty() {
        Vec::new()
    } else {
        extract_address_from_script(&script_pubkey)
    };

    Ok(CTxOut {
        value,
        script_length: script_pubkey.len() as i32,
        script_pubkey: CScript { script: script_pubkey },
        index: 0,
        address,
    })
}

fn extract_address_from_script(script: &[u8]) -> Vec<String> {
    // P2CS (Cold Stake): 51 bytes, PIVX Core CScript::IsPayToColdStaking[LOF]
    // OP_DUP OP_HASH160 OP_ROT OP_IF OP_CHECKCOLDSTAKEVERIFY(0xd2)|OP_CHECKCOLDSTAKEVERIFY_LOF(0xd1)
    // PUSH20 <staker> OP_ELSE PUSH20 <owner> OP_ENDIF OP_EQUALVERIFY OP_CHECKSIG
    // 0xd2 is what GetScriptForStakeDelegation emits since v5.2 (mainnet ~2,927,000);
    // 0xd1 is the original "last-output-free" variant. Both must be recognized.
    if script.len() == 51 &&
       script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x7b &&
       script[3] == 0x63 && (script[4] == 0xd1 || script[4] == 0xd2) && script[5] == 0x14 &&
       script[26] == 0x67 && script[27] == 0x14 &&
       script[48] == 0x68 && script[49] == 0x88 && script[50] == 0xac {
        let staker_hash = &script[6..26];
        let owner_hash = &script[28..48];

        let mut addresses = Vec::new();
        // Staker address (version 0x3f = 63)
        if let Some(staker_addr) = encode_pivx_address(staker_hash, 63) {
            addresses.push(staker_addr);
        }
        // Owner address (version 0x1e = 30)
        if let Some(owner_addr) = encode_pivx_address(owner_hash, 30) {
            addresses.push(owner_addr);
        }
        if !addresses.is_empty() {
            return addresses;
        }
    }

    // P2PKH: 76a914{20 byte pubkey hash}88ac
    if script.len() == 25 && script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x14
        && script[23] == 0x88 && script[24] == 0xac {
        let pubkey_hash = &script[3..23];
        if let Some(address) = encode_pivx_address(pubkey_hash, 30) {
            return vec![address];
        }
    }
    // Exchange address (TX_EXCHANGEADDR): OP_EXCHANGEADDR(0xe0)-prefixed P2PKH → EXM.
    // PIVX Core requires the 0xe0 opcode; any other leading byte is nonstandard.
    if script.len() == 26 && script[0] == 0xe0
        && script[1] == 0x76 && script[2] == 0xa9 && script[3] == 0x14
        && script[24] == 0x88 && script[25] == 0xac {
        let pubkey_hash = &script[4..24];
        if let Some(address) = encode_pivx_exchange_address(pubkey_hash) {
            return vec![address];
        }
    }
    // P2SH: a914{20 byte script hash}87 — exactly ONE address (version 13).
    // PIVX Core never encodes a script hash with the EXM prefix; emitting an EXM
    // twin here previously made the enrichment classifier drop P2SH outputs entirely.
    if script.len() == 23 && script[0] == 0xa9 && script[1] == 0x14 && script[22] == 0x87 {
        let script_hash = &script[2..22];
        if let Some(address) = encode_pivx_address(script_hash, 13) {
            return vec![address];
        }
    }

    // P2PK: {push_opcode}{pubkey} OP_CHECKSIG (0xac)
    // Compressed pubkey: 0x21 (push 33) + 33 bytes + 0xac = 35 bytes total
    // Uncompressed pubkey: 0x41 (push 65) + 65 bytes + 0xac = 67 bytes total
    if script.len() == 35 && script[0] == 0x21 && script[34] == 0xac {
        let pubkey = &script[1..34];
        
        // Hash the public key: RIPEMD160(SHA256(pubkey))
        let sha_hash = Sha256::digest(pubkey);
        let ripemd_hash = Ripemd160::digest(&sha_hash);
        
        if let Some(address) = encode_pivx_address(&ripemd_hash, 30) {
            return vec![address];
        }
    }
    if script.len() == 67 && script[0] == 0x41 && script[66] == 0xac {
        let pubkey = &script[1..66];
        
        // Hash the public key: RIPEMD160(SHA256(pubkey))
        let sha_hash = Sha256::digest(pubkey);
        let ripemd_hash = Ripemd160::digest(&sha_hash);
        
        if let Some(address) = encode_pivx_address(&ripemd_hash, 30) {
            return vec![address];
        }
    }
    Vec::new()
}

pub fn get_script_type(script: &[u8]) -> &str {
    // P2CS (Cold Stake) — both OP_CHECKCOLDSTAKEVERIFY (0xd2) and the LOF variant (0xd1)
    if script.len() == 51 &&
       script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x7b &&
       script[3] == 0x63 && (script[4] == 0xd1 || script[4] == 0xd2) && script[5] == 0x14 &&
       script[26] == 0x67 && script[27] == 0x14 &&
       script[48] == 0x68 && script[49] == 0x88 && script[50] == 0xac {
        return "coldstake";
    }

    // P2PKH (Pay to Public Key Hash)
    if script.len() == 25 && script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x14
        && script[23] == 0x88 && script[24] == 0xac {
        return "pubkeyhash";
    }

    // Exchange address (TX_EXCHANGEADDR): OP_EXCHANGEADDR(0xe0) + standard P2PKH
    if script.len() == 26 && script[0] == 0xe0
        && script[1] == 0x76 && script[2] == 0xa9 && script[3] == 0x14
        && script[24] == 0x88 && script[25] == 0xac {
        return "exchangeaddress";
    }

    // P2SH (Pay to Script Hash)
    if script.len() == 23 && script[0] == 0xa9 && script[1] == 0x14 && script[22] == 0x87 {
        return "scripthash";
    }

    // P2PK (Pay to Public Key)
    if (script.len() == 35 && script[0] == 0x21 && script[34] == 0xac) ||
       (script.len() == 67 && script[0] == 0x41 && script[66] == 0xac) {
        return "pubkey";
    }

    // Zerocoin
    if !script.is_empty() {
        match script[0] {
            0xc1 => return "zerocoinmint",
            0xc2 => return "zerocoinspend",
            0xc3 => return "zerocoinpublicspend",
            0x6a => return "nulldata",
            _ => {}
        }
    }

    // Empty script
    if script.is_empty() {
        return "nonstandard";
    }

    // Unknown type
    "unknown"
}

pub fn encode_pivx_address(hash: &[u8], version: u8) -> Option<String> {
    // PIVX address encoding: version byte + 20-byte hash + 4-byte checksum
    let mut data = Vec::with_capacity(25);
    data.push(version);
    data.extend_from_slice(hash);
    
    // Calculate checksum: first 4 bytes of SHA256(SHA256(version + hash))
    let first_hash = Sha256::digest(&data);
    let second_hash = Sha256::digest(&first_hash);
    data.extend_from_slice(&second_hash[..4]);
    
    // Encode to base58
    Some(bs58::encode(&data).into_string())
}

/// Encode PIVX exchange address (EXM prefix)
/// Exchange addresses use a 3-byte prefix: [0x01, 0xb9, 0xa2]
pub fn encode_pivx_exchange_address(hash: &[u8]) -> Option<String> {
    // PIVX exchange address: 3-byte prefix + 20-byte hash + 4-byte checksum
    let mut data = Vec::with_capacity(27);
    data.extend_from_slice(&[0x01, 0xb9, 0xa2]); // EXCHANGE_ADDRESS prefix
    data.extend_from_slice(hash);
    
    // Calculate checksum: first 4 bytes of SHA256(SHA256(prefix + hash))
    let first_hash = Sha256::digest(&data);
    let second_hash = Sha256::digest(&first_hash);
    data.extend_from_slice(&second_hash[..4]);
    
    // Encode to base58
    Some(bs58::encode(&data).into_string())
}

pub async fn deserialize_out_point(cursor: &mut Cursor<&[u8]>) -> COutPoint {
    let mut hash_bytes = [0u8; 32];
    let _ = cursor.read_exact(&mut hash_bytes); // Ignore errors
    
    // Reverse hash for display (match blocks.rs and transactions.rs behavior)
    // Database keys use reversed/display format: 't' + reversed_txid
    let reversed_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
    let hash = hex::encode(&reversed_hash);
    
    let n = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    
    COutPoint { hash, n }
}

// Stub functions for missing parser functions
pub async fn hash_txid(data: &[u8]) -> String {
    // Proper implementation: SHA256(SHA256(tx_bytes)) then reverse
    let first_hash = Sha256::digest(data);
    let txid = Sha256::digest(&first_hash);
    let reversed_txid: Vec<_> = txid.iter().rev().cloned().collect();
    hex::encode(&reversed_txid)
}

pub async fn read_varint(cursor: &mut Cursor<&[u8]>) -> Result<u64, std::io::Error> {
    // Proper Bitcoin varint implementation
    let first = cursor.read_u8()?;
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => u64::from(cursor.read_u16::<LittleEndian>()?),
        0xfe => u64::from(cursor.read_u32::<LittleEndian>()?),
        0xff => cursor.read_u64::<LittleEndian>()?,
    };
    Ok(value)
}

pub async fn read_script(cursor: &mut Cursor<&[u8]>) -> Result<Vec<u8>, std::io::Error> {
    let script_length = read_varint(cursor).await?;
    // Bound the allocation against the bytes actually remaining in the buffer.
    // A corrupt or misaligned length field would otherwise request a gigantic
    // allocation and abort the whole process (OOM) rather than failing this
    // one parse. Scripts can never exceed the remaining transaction bytes.
    let remaining = cursor.get_ref().len() as u64 - cursor.position();
    if script_length > remaining {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("script length {} exceeds {} remaining bytes", script_length, remaining),
        ));
    }
    let mut script = vec![0u8; script_length as usize];
    cursor.read_exact(&mut script)?;
    Ok(script)
}
#[cfg(test)]
mod golden_script_tests {
    //! Golden-vector tests: every expected value below was produced by PIVX Core
    //! itself (mainnet RPC `getblock`/`decodescript`, height 5,452,236, 2026-06-12).
    //! These pin the script→address layer to Core's Solver() 1:1.
    use super::{extract_address_from_script, get_script_type};

    fn hex_script(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    /// Real mainnet coinstake P2CS output (block 5,452,237) — OP_CHECKCOLDSTAKEVERIFY_LOF (0xd1)
    #[test]
    fn coldstake_d1_matches_core() {
        let script = hex_script("76a97b63d114b3be8567d0190c67ca4675a0019089c55fe695f96714ef6bede7abacb6bea406f5c67a6b9e5e066ca85a6888ac");
        assert_eq!(get_script_type(&script), "coldstake");
        assert_eq!(
            extract_address_from_script(&script),
            vec![
                "SdgQDpS8jDRJDX8yK8m9KnTMarsE84zdsy".to_string(), // staker (Core addresses[0])
                "DSy3LAbb93vd7xqqNcPQW2bsFwU6JsdTiF".to_string(), // owner  (Core addresses[1])
            ]
        );
    }

    /// Same P2CS with OP_CHECKCOLDSTAKEVERIFY (0xd2) — Core decodescript: same addresses
    #[test]
    fn coldstake_d2_matches_core() {
        let script = hex_script("76a97b63d214b3be8567d0190c67ca4675a0019089c55fe695f96714ef6bede7abacb6bea406f5c67a6b9e5e066ca85a6888ac");
        assert_eq!(get_script_type(&script), "coldstake");
        assert_eq!(
            extract_address_from_script(&script),
            vec![
                "SdgQDpS8jDRJDX8yK8m9KnTMarsE84zdsy".to_string(),
                "DSy3LAbb93vd7xqqNcPQW2bsFwU6JsdTiF".to_string(),
            ]
        );
    }

    /// Real mainnet P2PKH output (block 5,452,238)
    #[test]
    fn p2pkh_matches_core() {
        let script = hex_script("76a914dddabf603190714c8db2e837e191b83a3a520ba588ac");
        assert_eq!(get_script_type(&script), "pubkeyhash");
        assert_eq!(
            extract_address_from_script(&script),
            vec!["DRN9vVxE9WNQM5XxS1RxdfH2NqqKG4VS1A".to_string()]
        );
    }

    /// Real mainnet P2SH output (block 5,451,524) — exactly ONE address, version 13 ('6...')
    #[test]
    fn p2sh_matches_core_single_address() {
        let script = hex_script("a91403d3f3e2a851686bbd533a497b9dab0373303b6087");
        assert_eq!(get_script_type(&script), "scripthash");
        assert_eq!(
            extract_address_from_script(&script),
            vec!["6Ek5jic51RAB9FLNQ7m91QNNYbrbPqPKiS".to_string()]
        );
    }

    /// P2PK compressed — Core decodescript hashes the RAW 33 bytes
    #[test]
    fn p2pk_compressed_matches_core() {
        let script = hex_script("2102b463185b1f1d24b25e0eff8a8e61f4f5bfcbef423e9be20393eef1ada303b6cdac");
        assert_eq!(get_script_type(&script), "pubkey");
        assert_eq!(
            extract_address_from_script(&script),
            vec!["D7H3y8PtYh25Y56uiLXYJ27wrMj1T8apSb".to_string()]
        );
    }

    /// P2PK uncompressed — Core hashes the RAW 65 bytes (no compression first!)
    #[test]
    fn p2pk_uncompressed_matches_core() {
        let script = hex_script("4104678afdb0fe5548271967f1a67130b7105cd6a828e03909a67962e0ea1f61deb649f6bc3f4cef38c4f35504e51ec112de5c384df7ba0b8d578a4c702b6bf11d5fac");
        assert_eq!(get_script_type(&script), "pubkey");
        assert_eq!(
            extract_address_from_script(&script),
            vec!["DEA5vGb2NpAwCiCp5yTE16F3DueQUVivQp".to_string()]
        );
    }

    /// Exchange address (TX_EXCHANGEADDR): OP_EXCHANGEADDR(0xe0) + P2PKH → EXM base58
    #[test]
    fn exchange_address_matches_core() {
        let script = hex_script("e076a914dddabf603190714c8db2e837e191b83a3a520ba588ac");
        assert_eq!(get_script_type(&script), "exchangeaddress");
        assert_eq!(
            extract_address_from_script(&script),
            vec!["EXMXEkmvHzp7Y4fkRSmJvcDPmgivhinEziDK".to_string()]
        );
    }

    /// A wrapped P2SH with a non-0xe0 leading byte is NONSTANDARD per Core —
    /// it must produce no address (the old code emitted a bogus EXM address).
    #[test]
    fn bogus_wrapped_p2sh_is_nonstandard() {
        let script = hex_script("c0a91403d3f3e2a851686bbd533a497b9dab0373303b6087");
        assert_eq!(extract_address_from_script(&script), Vec::<String>::new());
    }
}
