/// Find which derivation path produces a specific address from an xpub

use bitcoin::util::bip32::{ExtendedPubKey, ChildNumber};
use std::str::FromStr;

fn main() {
    let xpub_str = "xpub6CKRnGxzF2Ln6ECB9bfL81HZvgY7RyHrqAyU4YwNpGeHWyvVarpst1ofiTfdkVAiDoNzrfvgb7fghBfAKHB7dTYjGcmx92pr4T3DUynWWEF";
    let target_address = "DDrJz4oZKqx68mgBLKuYZiDgE1drGGYZCK";
    
    println!("Searching for address: {}", target_address);
    println!("XPUB: {}\n", xpub_str);
    
    let xpub = ExtendedPubKey::from_str(xpub_str).expect("Invalid xpub");
    let secp = bitcoin::secp256k1::Secp256k1::new();
    
    // Try both receive (0) and change (1) chains, up to index 500
    for chain in 0..=1 {
        let chain_name = if chain == 0 { "receive" } else { "change" };
        
        if let Ok(chain_key) = xpub.ckd_pub(&secp, ChildNumber::from_normal_idx(chain).unwrap()) {
            for i in 0..500 {
                if let Ok(child) = chain_key.ckd_pub(&secp, ChildNumber::from_normal_idx(i).unwrap()) {
                    let pubkey_hash = child.public_key.pubkey_hash();
                    
                    // Encode as PIVX address (version 30 for mainnet)
                    if let Some(address) = encode_pivx_address(pubkey_hash.as_ref(), 30) {
                        if address == target_address {
                            println!("✅ FOUND!");
                            println!("   Address: {}", address);
                            println!("   Path: m/44'/119'/{}'/{}/ {}", xpub.depth.saturating_sub(3), chain, i);
                            println!("   Chain: {} ({})", chain, chain_name);
                            println!("   Index: {}", i);
                            return;
                        }
                    }
                }
            }
        }
    }
    
    println!("❌ Address not found in first 500 indices of receive and change chains");
}

fn encode_pivx_address(pubkey_hash: &[u8], version: u8) -> Option<String> {
    use bitcoin::hashes::{Hash, sha256d};
    
    let mut data = Vec::new();
    data.push(version);
    data.extend_from_slice(pubkey_hash);
    
    let checksum_full = sha256d::Hash::hash(&data);
    let checksum = &checksum_full[0..4];
    
    data.extend_from_slice(checksum);
    
    Some(bs58::encode(data).into_string())
}
