use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::types::{CTransaction, CTxIn, CTxOut, COutPoint, CScript, SaplingTxData, SpendDescription, OutputDescription};
use sha2::{Sha256, Digest};
use ripemd160::{Ripemd160, Digest as RipemdDigest};
use bs58;

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
    
    for _ in 0..count {
        // Read txid length
        let txid_len = match cursor.read_u32::<byteorder::LittleEndian>() {
            Ok(len) => len as usize,
            Err(_) => break,
        };
        
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
    let block_version = cursor.read_u32::<LittleEndian>().unwrap_or_default();

    let version = cursor.read_u16::<LittleEndian>().unwrap_or_default();
    let tx_type = cursor.read_u16::<LittleEndian>().unwrap_or_default();
    
    let input_count = read_varint(&mut cursor).await?;
    let mut inputs = Vec::new();
    
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
        ).await);
    }

    let output_count = read_varint(&mut cursor).await?;
    
    // Identify transaction type based on PIVX rules:
    // - Coinbase: first input has coinbase data (prev_hash all zeros), typically has multiple outputs
    // - Coinstake: first input has coinstake data, first output is ALWAYS empty (0 value, empty script)
    // - Normal: regular transaction with no coinbase/coinstake inputs
    
    let mut outputs = Vec::new();
    for i in 0..output_count {
        let mut output = deserialize_tx_out(&mut cursor, false).await;
        output.index = i;  // Set the correct vout index
        outputs.push(output);
    }

    let lock_time = cursor.read_u32::<LittleEndian>()?;

    // For Sapling transactions (version >= 3), parse the Sapling-specific data
    let sapling_data = if is_sapling {
        // Read the value_count varint (from old transactions.rs code)
        let _value_count = read_varint(&mut cursor).await.ok();
        
        // Read valueBalance (net value of spends - outputs)
        let value_balance = cursor.read_i64::<LittleEndian>().ok().unwrap_or(0);
        
        // Read vShieldedSpend count and parse each spend (384 bytes each)
        let spend_count = read_varint(&mut cursor).await.unwrap_or(0);
        let mut vshielded_spend = Vec::new();
        for _ in 0..spend_count {
            // Read each spend field (SPENDDESCRIPTION_SIZE = 384 bytes)
            let mut cv = [0u8; 32];           // Value commitment
            let mut anchor = [0u8; 32];       // Merkle tree root
            let mut nullifier = [0u8; 32];    // Prevents double-spending
            let mut rk = [0u8; 32];           // Randomized public key
            let mut zkproof = [0u8; 192];     // Groth16 zero-knowledge proof
            let mut spend_auth_sig = [0u8; 64]; // Spend authorization signature
            
            if cursor.read_exact(&mut cv).is_ok() &&
               cursor.read_exact(&mut anchor).is_ok() &&
               cursor.read_exact(&mut nullifier).is_ok() &&
               cursor.read_exact(&mut rk).is_ok() &&
               cursor.read_exact(&mut zkproof).is_ok() &&
               cursor.read_exact(&mut spend_auth_sig).is_ok() {
                
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
        }
        
        // Read vShieldedOutput count and parse each output (948 bytes each)
        let output_count = read_varint(&mut cursor).await.unwrap_or(0);
        let mut vshielded_output = Vec::new();
        for _ in 0..output_count {
            // Read each output field (OUTPUTDESCRIPTION_SIZE = 948 bytes)
            let mut cv = [0u8; 32];           // Value commitment
            let mut cmu = [0u8; 32];          // Note commitment u-coordinate
            let mut ephemeral_key = [0u8; 32]; // Ephemeral Jubjub public key
            let mut enc_ciphertext = [0u8; 580]; // Encrypted note for recipient
            let mut out_ciphertext = [0u8; 80];  // Encrypted note for sender OVK
            let mut zkproof = [0u8; 192];     // Groth16 zero-knowledge proof
            
            if cursor.read_exact(&mut cv).is_ok() &&
               cursor.read_exact(&mut cmu).is_ok() &&
               cursor.read_exact(&mut ephemeral_key).is_ok() &&
               cursor.read_exact(&mut enc_ciphertext).is_ok() &&
               cursor.read_exact(&mut out_ciphertext).is_ok() &&
               cursor.read_exact(&mut zkproof).is_ok() {
                
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
        }
        
        // Read bindingSig (BINDINGSIG_SIZE = 64 bytes)
        let mut binding_sig = [0u8; 64];
        if cursor.position() + 64 <= cursor.get_ref().len() as u64 {
            cursor.read_exact(&mut binding_sig).ok();
        }
        
        // For special transaction types (nType != 0), read extraPayload
        if tx_type != 0 {
            // Read extraPayload length and skip it
            if let Ok(payload_size) = read_varint(&mut cursor).await {
                cursor.set_position(cursor.position() + payload_size);
            }
        }
        
        Some(SaplingTxData {
            value_balance,
            vshielded_spend,
            vshielded_output,
            binding_sig,
        })
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
) -> CTxIn {
    // Standard Bitcoin/PIVX transaction input format:
    // - prev_hash (32 bytes)
    // - prev_index (4 bytes)
    // - script_sig_len (varint)
    // - script_sig (variable)
    // - sequence (4 bytes)
    
    let mut prev_hash = [0u8; 32];
    let _ = cursor.read_exact(&mut prev_hash);
    let prev_index = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    
    let script_sig = read_script(cursor).await.unwrap_or(Vec::new());
    let sequence = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    
    // Check if this is TRULY coinbase: prev_hash is all zeros AND prev_index is 0xffffffff
    // CRITICAL: Coinstake transactions have REAL prevouts, not null!
    let is_coinbase = prev_hash.iter().all(|&b| b == 0) && prev_index == 0xffffffff;
    
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
        CTxIn {
            prevout,
            script_sig: CScript { script: Vec::new() },
            sequence,
            index: 0,
            coinbase: Some(script_sig),
        }
    } else {
        // Regular input OR coinstake input - both have real prevouts!
        CTxIn {
            prevout,
            script_sig: CScript { script: script_sig },
            sequence,
            index: 0,
            coinbase: None,  // NOT coinbase - has real prevout
        }
    }
}

pub async fn deserialize_tx_out(cursor: &mut Cursor<&[u8]>, _is_coinstake_empty: bool) -> CTxOut {
    // Standard Bitcoin/PIVX transaction output format:
    // - value (8 bytes, i64)
    // - script_pubkey_len (varint)
    // - script_pubkey (variable)
    
    let value = cursor.read_i64::<LittleEndian>().unwrap_or(0);
    let script_pubkey = read_script(cursor).await.unwrap_or(Vec::new());
    
    // Extract address from script if it's not empty
    let address = if script_pubkey.is_empty() {
        Vec::new()
    } else {
        extract_address_from_script(&script_pubkey)
    };

    CTxOut {
        value,
        script_length: script_pubkey.len() as i32,
        script_pubkey: CScript { script: script_pubkey },
        index: 0,
        address,
    }
}

fn extract_address_from_script(script: &[u8]) -> Vec<String> {
    // P2CS (Cold Stake): 76a97b63d114{20 byte staker}6714{20 byte owner}6888ac (51 bytes)
    // Format: OP_DUP OP_HASH160 OP_ROT OP_IF OP_CHECKCOLDSTAKEVERIFY(0xd1) PUSH20 <staker> OP_ELSE PUSH20 <owner> OP_ENDIF OP_EQUALVERIFY OP_CHECKSIG
    if script.len() == 51 && 
       script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x7b && 
       script[3] == 0x63 && script[4] == 0xd1 && script[5] == 0x14 &&
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
    // P2PKH with a single leading prefix byte (exchange wrapper -> EXM)
    if script.len() == 26 && script[1] == 0x76 && script[2] == 0xa9 && script[3] == 0x14
        && script[24] == 0x88 && script[25] == 0xac {
        let pubkey_hash = &script[4..24];
        // This variant is used by exchange-wrapped P2PKH outputs where a
        // custom OP_EXCHANGEADDR (0xe0) prefix is prepended. Encode using
        // the 3-byte EXM prefix so the explorer surfaces the EXM address.
        if let Some(address) = encode_pivx_exchange_address(pubkey_hash) {
            return vec![address];
        }
    }
    // P2SH: a914{20 byte script hash}87
    if script.len() == 23 && script[0] == 0xa9 && script[1] == 0x14 && script[22] == 0x87 {
        let script_hash = &script[2..22];
        let mut addresses = Vec::new();
        // Standard P2SH encoding (version 13)
        if let Some(address) = encode_pivx_address(script_hash, 13) {
            addresses.push(address);
        }
        // Also include Exchange (EXM) encoding when present - some services
        // use the same script pattern but different base58 prefix.
        if let Some(ex_addr) = encode_pivx_exchange_address(script_hash) {
            if !addresses.contains(&ex_addr) {
                addresses.push(ex_addr);
            }
        }
        if !addresses.is_empty() {
            return addresses;
        }
    }
    // P2SH with a single leading prefix byte
    if script.len() == 24 && script[1] == 0xa9 && script[2] == 0x14 && script[23] == 0x87 {
        let script_hash = &script[3..23];
        let mut addresses = Vec::new();
        if let Some(address) = encode_pivx_address(script_hash, 13) {
            addresses.push(address);
        }
        if let Some(ex_addr) = encode_pivx_exchange_address(script_hash) {
            if !addresses.contains(&ex_addr) {
                addresses.push(ex_addr);
            }
        }
        if !addresses.is_empty() {
            return addresses;
        }
    }
    
    // P2SH Exchange Address: Same pattern as P2SH but encoded with exchange prefix
    // Note: Exchange addresses use the same script pattern but different encoding
    // This is detected the same way as P2SH, but we'll keep standard P2SH for now
    // and let the wallet handle exchange address encoding when needed
    
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
    // P2CS (Cold Stake)
    if script.len() == 51 && 
       script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x7b && 
       script[3] == 0x63 && script[4] == 0xd1 && script[5] == 0x14 &&
       script[26] == 0x67 && script[27] == 0x14 &&
       script[48] == 0x68 && script[49] == 0x88 && script[50] == 0xac {
        return "coldstake";
    }
    
    // P2PKH (Pay to Public Key Hash)
    if script.len() == 25 && script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x14 
        && script[23] == 0x88 && script[24] == 0xac {
        return "pubkeyhash";
    }
    
    // Exchange-wrapped P2PKH: single leading prefix byte + standard P2PKH
    // Example: 0xe0 76 a9 14 {20} 88 ac  (26 bytes)
    if script.len() == 26 && script[1] == 0x76 && script[2] == 0xa9 && script[3] == 0x14
        && script[24] == 0x88 && script[25] == 0xac {
        return "exchangeaddress";
    }
    
    // P2SH (Pay to Script Hash)
    if script.len() == 23 && script[0] == 0xa9 && script[1] == 0x14 && script[22] == 0x87 {
        return "scripthash";
    }
    
    // Exchange-wrapped P2SH: single leading prefix byte + standard P2SH
    // Example: <prefix> a9 14 {20} 87 (24 bytes)
    if script.len() == 24 && script[1] == 0xa9 && script[2] == 0x14 && script[23] == 0x87 {
        return "exchangeaddress";
    }
    
    // P2PK (Pay to Public Key)
    if (script.len() == 35 && script[0] == 0x21 && script[34] == 0xac) ||
       (script.len() == 67 && script[0] == 0x41 && script[66] == 0xac) {
        return "pubkey";
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
    let hash = hex::encode(hash_bytes);
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
    let mut script = vec![0u8; script_length as usize];
    cursor.read_exact(&mut script)?;
    Ok(script)
}