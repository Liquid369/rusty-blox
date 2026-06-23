/// Synchronous/blocking wrapper for scriptpubkey_to_address
pub fn scriptpubkey_to_address_blocking(cs: &CScript) -> Option<AddressType> {
    futures::executor::block_on(scriptpubkey_to_address(cs))
}

/// Synchronous/blocking wrapper for address_type_to_string
pub fn address_type_to_string_blocking(address: Option<AddressType>) -> Vec<String> {
    futures::executor::block_on(address_type_to_string(address))
}
// address.rs

use sha2::{Sha256, Digest};
use ripemd160::{Ripemd160};
use bs58;
use crate::types::{CScript, AddressType};

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

#[allow(dead_code)] // Crypto utility - may be needed for address validation
fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

#[allow(dead_code)] // Crypto utility - may be needed for address validation
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
    hasher.update(first);
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

async fn extract_pubkey_from_script(script: &[u8]) -> Option<&[u8]> {
    const OP_CHECKSIG: u8 = 0xAC;

    // P2PK: <PUSH33> <33-byte compressed key> OP_CHECKSIG  (35 bytes)
    //       <PUSH65> <65-byte uncompressed key> OP_CHECKSIG (67 bytes)
    match script.len() {
        67 if script[0] == 0x41
            && script[66] == OP_CHECKSIG
            && script[1] == 0x04 =>
        {
            Some(&script[1..66])
        }
        35 if script[0] == 0x21
            && script[34] == OP_CHECKSIG
            && (script[1] == 0x02 || script[1] == 0x03) =>
        {
            Some(&script[1..34])
        }
        _ => None,
    }
}

async fn scriptpubkey_to_p2pk(script: &CScript) -> Option<String> {
    let pubkey = extract_pubkey_from_script(&script.script).await?;

    // PIVX Core (CPubKey::GetID): Hash160 over the pubkey bytes EXACTLY as they
    // appear in the script — uncompressed keys are NOT compressed first.
    let pubkey_hash = compute_address_hash(pubkey).await;
    let pubkey_addr = hash_address(&pubkey_hash, 30).await;

    Some(pubkey_addr)
}

/// Match a canonical P2CS (pay-to-cold-staking) script and extract both hashes.
///
/// PIVX Core (CScript::IsPayToColdStaking / MatchPayToColdStaking), 51 bytes:
/// OP_DUP OP_HASH160 OP_ROT OP_IF OP_CHECKCOLDSTAKEVERIFY[_LOF] 0x14 <staker20>
/// OP_ELSE 0x14 <owner20> OP_ENDIF OP_EQUALVERIFY OP_CHECKSIG
/// staker hash = bytes 6..26, owner hash = bytes 28..48 (fixed offsets).
async fn scriptpubkey_to_staking_address(script: &CScript) -> Option<(String, String)> {
    const OP_CHECKCOLDSTAKEVERIFY: u8 = 0xd2;
    const OP_CHECKCOLDSTAKEVERIFY_LOF: u8 = 0xd1;

    let s = &script.script;
    if s.len() == 51
        && s[0] == 0x76 // OP_DUP
        && s[1] == 0xa9 // OP_HASH160
        && s[2] == 0x7b // OP_ROT
        && s[3] == 0x63 // OP_IF
        && (s[4] == OP_CHECKCOLDSTAKEVERIFY || s[4] == OP_CHECKCOLDSTAKEVERIFY_LOF)
        && s[5] == 0x14
        && s[26] == 0x67 // OP_ELSE
        && s[27] == 0x14
        && s[48] == 0x68 // OP_ENDIF
        && s[49] == 0x88 // OP_EQUALVERIFY
        && s[50] == 0xac // OP_CHECKSIG
    {
        let staker_key_hash = &s[6..26];
        let owner_key_hash = &s[28..48];

        let staker_address = hash_address(staker_key_hash, 63).await;
        let owner_address = hash_address(owner_key_hash, 30).await;

        return Some((staker_address, owner_address));
    }
    None
}

/// Match a PIVX exchange-address script (TX_EXCHANGEADDR), 26 bytes:
/// OP_EXCHANGEADDR(0xe0) OP_DUP OP_HASH160 0x14 <keyhash20> OP_EQUALVERIFY OP_CHECKSIG
/// Encoded with the 3-byte EXM base58 prefix [0x01, 0xb9, 0xa2].
async fn scriptpubkey_to_exchange_address(script: &CScript) -> Option<String> {
    let s = &script.script;
    if s.len() == 26
        && s[0] == 0xe0 // OP_EXCHANGEADDR
        && s[1] == 0x76
        && s[2] == 0xa9
        && s[3] == 0x14
        && s[24] == 0x88
        && s[25] == 0xac
    {
        let key_hash = &s[4..24];
        let mut data = Vec::with_capacity(27);
        data.extend_from_slice(&[0x01, 0xb9, 0xa2]);
        data.extend_from_slice(key_hash);
        let checksum = sha256d(&data);
        data.extend_from_slice(&checksum[0..4]);
        return Some(bs58::encode(data).into_string());
    }
    None
}

pub async fn scriptpubkey_to_address(script: &CScript) -> Option<AddressType> {
    if script.script.is_empty() {
        return Some(AddressType::Nonstandard);
    }

    // Structural matching, mirroring PIVX Core's Solver() — each script template is
    // identified by exact layout (length + opcode positions), never by byte content
    // heuristics (hash/pubkey bytes can contain any value, including opcode bytes).

    // P2PKH: OP_DUP OP_HASH160 0x14 <hash20> OP_EQUALVERIFY OP_CHECKSIG (25 bytes)
    if let Some(address) = scriptpubkey_to_p2pkh_address(script).await {
        return Some(AddressType::P2PKH(address));
    }

    // P2SH: OP_HASH160 0x14 <hash20> OP_EQUAL (23 bytes)
    if let Some(address) = scriptpubkey_to_p2sh_address(script).await {
        return Some(AddressType::P2SH(address));
    }

    // P2CS cold staking (51 bytes, canonical layout, 0xd1 or 0xd2)
    if let Some((staker_address, owner_address)) = scriptpubkey_to_staking_address(script).await {
        return Some(AddressType::Staking(staker_address, owner_address));
    }

    // Exchange address: OP_EXCHANGEADDR-prefixed P2PKH (26 bytes) → EXM base58
    if let Some(address) = scriptpubkey_to_exchange_address(script).await {
        return Some(AddressType::P2PKH(address));
    }

    // P2PK: <push><pubkey> OP_CHECKSIG (35 or 67 bytes)
    if let Some(pubkey_addr) = scriptpubkey_to_p2pk(script).await {
        return Some(AddressType::P2PK(pubkey_addr));
    }

    // Zerocoin markers
    match script.script.first() {
        Some(0xc1) => Some(AddressType::ZerocoinMint),
        Some(0xc2) => Some(AddressType::ZerocoinSpend),
        Some(0xc3) => Some(AddressType::ZerocoinPublicSpend),
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
                // Return both the staker (delegated stake address, usually S-prefixed)
                // and the owner (actual coin owner, usually D-prefixed) separately so
                // callers (frontend/indexers) can show delegation relationships.
                vec![staker, owner]
            }
        Some(AddressType::Sapling) => vec!["Sapling".to_string()],
        None => vec!["None".to_string()],
    }
}
