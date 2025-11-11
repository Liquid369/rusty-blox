// address.rs

use sha2::{Sha256, Digest};
use ripemd160::{Ripemd160, Digest as RipemdDigest};
use bs58;
use crate::types::{CScript, AddressType, CTxOut};
use std::error::Error;
use std::sync::Arc;
use rocksdb::DB;

pub async fn compute_address_hash(data: &[u8]) -> Vec<u8> {
    let sha = Sha256::digest(data);
    Ripemd160::digest(&sha).to_vec()
}

pub async fn hash_address(hash: &[u8], prefix: u8) -> String {
    let mut extended_hash = vec![prefix];
    extended_hash.extend_from_slice(hash);

    let checksum = sha256d(&extended_hash);
    extended_hash.extend_from_slice(&checksum[0..4]);

    bs58::encode(extended_hash).into_string()
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn ripemd160_hash(data: &[u8]) -> Vec<u8> {
    let mut hasher = Ripemd160::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn sha256d(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let first = hasher.finalize();

    let mut hasher = Sha256::new();
    hasher.update(&first);
    hasher.finalize().to_vec()
}

async fn scriptpubkey_to_p2pkh_address(script: &CScript) -> Option<String> {
    if script.script.len() == 25
        && script.script[0] == 0x76
        && script.script[1] == 0xa9
        && script.script[2] == 0x14
        && script.script[23] == 0x88
        && script.script[24] == 0xac
    {
        let address_hash = &script.script[3..23];
        Some(hash_address(address_hash, 30).await)
    } else {
        None
    }
}

async fn scriptpubkey_to_p2sh_address(script: &CScript) -> Option<String> {
    if script.script.len() == 23
        && script.script[0] == 0xa9
        && script.script[1] == 0x14
        && script.script[22] == 0x87
    {
        let address_hash = &script.script[2..22];
        Some(hash_address(address_hash, 13).await)
    } else {
        None
    }
}

async fn compress_pubkey(pub_key_bytes: &[u8]) -> Option<Vec<u8>> {
    match pub_key_bytes.len() {
        65 if pub_key_bytes[0] == 0x04 => {
            let x = &pub_key_bytes[1..33];
            let y = &pub_key_bytes[33..65];
            let parity = if y[31] % 2 == 0 { 2 } else { 3 };
            let mut compressed_key: Vec<u8> = vec![parity];
            compressed_key.extend_from_slice(x);
            Some(compressed_key)
        }
        33 if pub_key_bytes[0] == 0x02 || pub_key_bytes[0] == 0x03 => {
            Some(pub_key_bytes.to_vec())
        }
        _ => None,
    }
}

async fn extract_pubkey_from_script(script: &[u8]) -> Option<&[u8]> {
    const OP_CHECKSIG: u8 = 0xAC;

    if script.last()? != &OP_CHECKSIG {
        return None;
    }

    match script.len() {
        67 => Some(&script[1..66]),
        35 => Some(&script[1..34]),
        _ => None,
    }
}

async fn scriptpubkey_to_p2pk(script: &CScript) -> Option<String> {
    const OP_DUP: u8 = 0x76;

    if script.script.contains(&OP_DUP) {
        return None;
    }

    let pubkey = extract_pubkey_from_script(&script.script).await?;

    let pubkey_compressed = compress_pubkey(pubkey).await?;
    let pubkey_hash = compute_address_hash(&pubkey_compressed).await;
    let pubkey_addr = hash_address(&pubkey_hash, 30).await;

    Some(pubkey_addr)
}

async fn scriptpubkey_to_staking_address(script: &CScript) -> Option<(String, String)> {
    const HASH_LEN: usize = 20;
    const OP_CHECKCOLDSTAKEVERIFY: u8 = 0xd2;
    const OP_CHECKCOLDSTAKEVERIFY_LOF: u8 = 0xd1;
    const OP_ELSE: u8 = 0x67;

    let pos_checkcoldstakeverify = script
        .script
        .iter()
        .position(|&x| x == OP_CHECKCOLDSTAKEVERIFY || x == OP_CHECKCOLDSTAKEVERIFY_LOF)?;

    if script.script.len() < pos_checkcoldstakeverify + 1 + HASH_LEN {
        return None;
    }

    let staker_key_hash =
        &script.script[(pos_checkcoldstakeverify + 1)..(pos_checkcoldstakeverify + 1 + HASH_LEN)];

    let pos_else = script.script.iter().position(|&x| x == OP_ELSE)?;
    if script.script.len() < pos_else + 1 + HASH_LEN {
        return None;
    }

    let owner_key_hash = &script.script[(pos_else + 1)..(pos_else + 1 + HASH_LEN)];

    let staker_address = hash_address(staker_key_hash, 63).await;
    let owner_address = hash_address(owner_key_hash, 30).await;

    Some((staker_address, owner_address))
}

pub async fn scriptpubkey_to_address(script: &CScript) -> Option<AddressType> {
    if script.script.is_empty() {
        return Some(AddressType::Nonstandard);
    }

    // Define op_codes
    const OP_DUP: u8 = 0x76;
    const OP_HASH160: u8 = 0xa9;
    const OP_EQUAL: u8 = 0x87;
    const OP_EQUALVERIFY: u8 = 0x88;
    const OP_CHECKSIG: u8 = 0xac;
    const OP_CHECKCOLDSTAKEVERIFY_LOF: u8 = 0xd1;
    const OP_CHECKCOLDSTAKEVERIFY: u8 = 0xd2;

    // Check the first byte and script length
    match script.script.as_slice() {
        [OP_DUP, OP_HASH160, 0x14, .., OP_EQUALVERIFY, OP_CHECKSIG]
            if script.script.len() == 25 =>
        {
            if let Some(address) = scriptpubkey_to_p2pkh_address(script).await {
                Some(AddressType::P2PKH(address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        [OP_HASH160, 0x14, .., OP_EQUAL] if script.script.len() == 23 => {
            if let Some(address) = scriptpubkey_to_p2sh_address(script).await {
                Some(AddressType::P2SH(address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        [0xc1, ..] => Some(AddressType::ZerocoinMint),
        [0xc2, ..] => Some(AddressType::ZerocoinSpend),
        [0xc3, ..] => Some(AddressType::ZerocoinPublicSpend),
        [.., OP_CHECKSIG]
            if !script.script.contains(&OP_DUP)
                && script.script.len() > 1
                && !script.script.contains(&OP_CHECKCOLDSTAKEVERIFY)
                && !script.script.contains(&OP_CHECKCOLDSTAKEVERIFY_LOF) =>
        {
            if let Some(pubkey) = scriptpubkey_to_p2pk(script).await {
                Some(AddressType::P2PK(pubkey))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        _ if script.script.contains(&OP_CHECKCOLDSTAKEVERIFY)
            || script.script.contains(&OP_CHECKCOLDSTAKEVERIFY_LOF) =>
        {
            if let Some((staker_address, owner_address)) = scriptpubkey_to_staking_address(script).await {
                Some(AddressType::Staking(staker_address, owner_address))
            } else {
                Some(AddressType::Nonstandard)
            }
        }
        _ => Some(AddressType::Nonstandard),
    }
}

pub async fn address_type_to_string(address: Option<AddressType>) -> Vec<String> {
    match address {
        Some(AddressType::CoinStakeTx) => vec!["CoinStakeTx".to_string()],
        Some(AddressType::CoinBaseTx) => vec!["CoinBaseTx".to_string()],
        Some(AddressType::Nonstandard) => vec!["Nonstandard".to_string()],
        Some(AddressType::P2PKH(addr)) => vec![addr],
        Some(AddressType::P2PK(pubkey)) => vec![pubkey],
        Some(AddressType::P2SH(addr)) => vec![addr],
        Some(AddressType::ZerocoinMint) => vec!["ZerocoinMint".to_string()],
        Some(AddressType::ZerocoinSpend) => vec!["ZerocoinSpend".to_string()],
        Some(AddressType::ZerocoinPublicSpend) => vec!["ZerocoinPublicSpend".to_string()],
        Some(AddressType::Staking(staker, owner)) => {
            vec![format!("Staking({}, {})", staker, owner)]
        }
        Some(AddressType::Sapling) => vec!["Sapling".to_string()],
        None => vec!["None".to_string()],
    }
}
