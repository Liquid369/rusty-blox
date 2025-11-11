use serde::Serialize;
use std::path::PathBuf;
use futures::stream;
use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::types::{CTransaction, CTxIn, CTxOut, COutPoint, CScript};
use sha2::{Sha256, Digest};

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

pub async fn deserialize_utxos(data: &[u8]) -> Vec<(Vec<u8>, u64)> {
    let mut utxos = Vec::new();
    let mut iter = data.chunks_exact(40); // 32 bytes for txid and 8 bytes for index
    while let Some(chunk) = iter.next() {
        let txid = chunk[0..32].to_vec();
        if let Ok(bytes) = <[u8; 8]>::try_from(&chunk[32..40]) {
            let index = u64::from_le_bytes(bytes);
            utxos.push((txid, index));
        }
    }
    utxos
}

pub async fn deserialize_transaction(
    data: &[u8],
) -> Result<CTransaction, std::io::Error> {
    let txid = hash_txid(&data[4..]).await;
    let mut cursor = Cursor::new(data);
    let block_version = cursor.read_u32::<LittleEndian>().unwrap_or_default();

    let version = cursor.read_i16::<LittleEndian>().unwrap_or_default();
    let input_count = read_varint(&mut cursor).await?;
    let mut inputs = Vec::new();
    for _ in 0..input_count {
        inputs.push(deserialize_tx_in(
            &mut cursor,
            version as u32,
            block_version,
        ).await);
    }

    let output_count = read_varint(&mut cursor).await?;
    let mut outputs = Vec::new();
    for _ in 0..output_count {
        outputs.push(deserialize_tx_out(&mut cursor).await);
    }

    let lock_time = cursor.read_u32::<LittleEndian>().unwrap();

    Ok(CTransaction {
        txid: txid,
        version: version,
        inputs: inputs,
        outputs: outputs,
        lock_time: lock_time,
    })
}

pub async fn deserialize_tx_in(cursor: &mut Cursor<&[u8]>, tx_ver_out: u32, block_version: u32) -> CTxIn {
    if block_version < 3 && tx_ver_out == 2 {
        // It's a coinbase transaction
        let mut buffer = [0; 26];
        cursor.read_exact(&mut buffer).unwrap();
        let coinbase = buffer.to_vec();
        let sequence = cursor.read_u32::<LittleEndian>().unwrap();

        CTxIn {
            prevout: None,
            script_sig: CScript { script: Vec::new() },
            sequence: sequence,
            index: 0,
            coinbase: Some(coinbase),
        }
    } else {
        // It's a regular transaction
        let prevout = deserialize_out_point(cursor).await;
        let script_sig = read_script(cursor).await.unwrap();
        let sequence = cursor.read_u32::<LittleEndian>().unwrap();
        let index = cursor.read_u64::<LittleEndian>().unwrap();

        CTxIn {
            prevout: Some(prevout),
            script_sig: CScript { script: script_sig },
            sequence: sequence,
            index: index,
            coinbase: None,
        }
    }
}

pub async fn deserialize_tx_out(cursor: &mut Cursor<&[u8]>) -> CTxOut {
    let value = cursor.read_i64::<LittleEndian>().unwrap();
    let script_length = read_varint(cursor).await;
    let mut script_pubkey = vec![0; script_length.unwrap() as usize];
    cursor.read_exact(&mut script_pubkey).unwrap();
    let index = cursor.read_u64::<LittleEndian>().unwrap();

    let mut address_data = Vec::new();
    cursor.read_to_end(&mut address_data).unwrap();
    let address = String::from_utf8(address_data).unwrap();

    CTxOut {
        value: value,
        script_length: script_pubkey.len() as i32,
        script_pubkey: CScript {
            script: script_pubkey,
        },
        index: index,
        address: vec![address],
    }
}

pub async fn deserialize_out_point(cursor: &mut Cursor<&[u8]>) -> COutPoint {
    let mut hash_bytes = [0u8; 32];
    cursor.read_exact(&mut hash_bytes).unwrap();
    let hash = hex::encode(hash_bytes);
    let n = cursor.read_u32::<LittleEndian>().unwrap();
    
    COutPoint { hash, n }
}

// Stub functions for missing parser functions
pub async fn hash_txid(data: &[u8]) -> String {
    // Proper implementation: SHA256(SHA256(tx_bytes)) then reverse
    let first_hash = Sha256::digest(&data);
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