use std::collections::HashMap;

use crate::chainstate_leveldb;
use crate::types::{AddressType, CScript};
use secp256k1::{PublicKey, Secp256k1};

/// Decompress amount following Bitcoin Core / PIVX Core CompressAmount scheme.
pub fn decompress_amount(x: u64) -> u64 {
    if x == 0 {
        return 0;
    }
    let mut v = x - 1;
    let e = (v % 10) as u32;
    v /= 10;
    if e < 9 {
        let d = (v % 9) + 1;
        let mut value = d;
        for _ in 0..e {
            value *= 10;
        }
        value
    } else {
        v + 1
    }
}

/// Decompress script pubkey as used by Core's CompressScript.
/// Decompress a script from PIVX's ScriptCompression format.
///
/// Special scripts (types 0-5) are passed with the type byte first:
/// - Type 0x00 + 20 bytes: P2PKH
/// - Type 0x01 + 20 bytes: P2SH  
/// - Type 0x02/0x03 + 32 bytes: P2PK compressed
/// - Type 0x04/0x05 + 32 bytes: P2PK uncompressed (needs decompression)
///
/// Non-special scripts are just the raw script bytes (returned as-is).
pub fn decompress_script(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return vec![];
    }

    // Check if this is a special compressed script (has type byte + data)
    if data.len() == 21 || data.len() == 33 {
        let nsize = data[0];

        // P2PKH: 0x00 + 20
        if nsize == 0x00 && data.len() == 21 {
            let mut script = Vec::with_capacity(25);
            // OP_DUP OP_HASH160 PUSH20 <20> OP_EQUALVERIFY OP_CHECKSIG
            script.push(0x76);
            script.push(0xa9);
            script.push(0x14);
            script.extend_from_slice(&data[1..21]);
            script.push(0x88);
            script.push(0xac);
            return script;
        }

        // P2SH: 0x01 + 20
        if nsize == 0x01 && data.len() == 21 {
            let mut script = Vec::with_capacity(23);
            // OP_HASH160 PUSH20 <20> OP_EQUAL
            script.push(0xa9);
            script.push(0x14);
            script.extend_from_slice(&data[1..21]);
            script.push(0x87);
            return script;
        }

        // P2PK compressed: 0x02/0x03 + 32
        if (nsize == 0x02 || nsize == 0x03) && data.len() == 33 {
            let mut script = Vec::with_capacity(35);
            // push 33 + pubkey + OP_CHECKSIG
            script.push(0x21);
            script.push(nsize);
            script.extend_from_slice(&data[1..33]);
            script.push(0xac);
            return script;
        }

        // P2PK uncompressed: 0x04/0x05 + 32
        // These are stored as 32 bytes + a type indicating which y parity to use.
        // Reconstruct compressed pubkey (0x02/0x03 + 32) and decompress to 65 bytes.
        if (nsize == 0x04 || nsize == 0x05) && data.len() == 33 {
            // data[0] is nsize (4 or 5). The compressed pubkey prefix is (nsize - 2) => 0x02 or 0x03
            let prefix = nsize - 2; // 2 or 3
            let mut compressed: Vec<u8> = Vec::with_capacity(33);
            compressed.push(prefix);
            compressed.extend_from_slice(&data[1..33]);

            // Use secp256k1 to decompress
            if let Ok(pk) = PublicKey::from_slice(&compressed) {
                let _secp = Secp256k1::new();
                let uncompressed = pk.serialize_uncompressed(); // [u8;65]
                let mut script = Vec::with_capacity(67);
                // push 65-byte pubkey as PUSHDATA
                script.push(65u8);
                script.extend_from_slice(&uncompressed[..]);
                // OP_CHECKSIG
                script.push(0xac);
                return script;
            } else {
                // Failed to parse compressed pubkey - return empty to mark unknown
                return vec![];
            }
        }
    }

    // Non-special script: return as-is
    data.to_vec()
}

/// Read a CompactSize (Bitcoin varint) from bytes at pos, advancing pos.
fn read_compact_size(data: &[u8], pos: &mut usize) -> Option<u64> {
    if *pos >= data.len() {
        return None;
    }
    let first = data[*pos];
    *pos += 1;
    match first {
        0..=0xfc => Some(first as u64),
        0xfd => {
            if *pos + 2 > data.len() {
                return None;
            }
            let v = u16::from_le_bytes([data[*pos], data[*pos + 1]]) as u64;
            *pos += 2;
            Some(v)
        }
        0xfe => {
            if *pos + 4 > data.len() {
                return None;
            }
            let v = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
                as u64;
            *pos += 4;
            Some(v)
        }
        0xff => {
            if *pos + 8 > data.len() {
                return None;
            }
            let v = u64::from_le_bytes([
                data[*pos],
                data[*pos + 1],
                data[*pos + 2],
                data[*pos + 3],
                data[*pos + 4],
                data[*pos + 5],
                data[*pos + 6],
                data[*pos + 7],
            ]);
            *pos += 8;
            Some(v)
        }
    }
}

/// Parsed representation of a CCoins entry
pub struct ParsedCoins {
    pub height: u32,
    pub is_coinbase: bool,
    /// Vec of (vout_index, amount_satoshis, script_pubkey_bytes, output_kind, resolved_addresses)
    /// resolved_addresses: Vec<String> - empty when no address could be resolved (e.g., shielded)
    pub unspent_outputs: Vec<(usize, u64, Vec<u8>, OutputKind, Vec<String>)>,
}

/// Kind of an unspent output after basic script/address analysis
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputKind {
    Standard,
    CoinStake,
    ColdStake { staker: String, owner: String },
    Coinbase,
    Zerocoin(String), // subtype string (mint/spend/publicspend)
    Sapling,
    Unknown,
}

/// Parse a single raw CCoins value (as stored in chainstate LevelDB) into
/// a ParsedCoins structure. Returns None on parse error.
///
/// PIVX COIN FORMAT (different from Bitcoin!):
/// code = VARINT((coinbase ? 2 : 0) | (coinstake ? 1 : 0) | (height << 2))
/// - Bit 0: coinstake flag (PIVX-specific)
/// - Bit 1: coinbase flag
/// - Bits 2+: height
pub fn parse_coins_value(raw: &[u8]) -> Option<ParsedCoins> {
    let mut pos = 0usize;
    // PIVX format: code = nHeight * 4 + (fCoinBase ? 2 : 0) + (fCoinStake ? 1 : 0)
    let code = read_compact_size(raw, &mut pos)?;
    let height = (code >> 2) as u32; // Extract height from bits 2+
    let is_coinbase = (code & 2) != 0; // Check bit 1
    let is_coinstake = (code & 1) != 0; // Check bit 0 (PIVX-specific)

    // mask: vector<unsigned char> (compact size length + bytes)
    // mask: vector<unsigned char> (compact size length + bytes)
    // mask_len is the number of bytes in the bitmap; each bit represents an output (vout)
    let mask_len = read_compact_size(raw, &mut pos)? as usize;
    if pos + mask_len > raw.len() {
        return None;
    }
    let mask_bytes = &raw[pos..pos + mask_len];
    pos += mask_len;

    let mut unspent_outputs: Vec<(usize, u64, Vec<u8>, OutputKind, Vec<String>)> = Vec::new();

    // Count set bits in mask to know how many outputs to expect
    let _expected_bits = mask_bytes
        .iter()
        .map(|b| b.count_ones() as usize)
        .sum::<usize>();
    let mut parsed_count = 0usize;

    // Walk mask bits; for each set bit read amount and script
    // Be defensive: don't assume mask_len is small; clearly bound vout indices
    let max_vouts = mask_len.checked_mul(8).unwrap_or(usize::MAX);
    'outer: for (byte_i, &b) in mask_bytes.iter().enumerate() {
        for bit in 0..8 {
            // Stop early if we've consumed all input bytes
            if pos >= raw.len() {
                break 'outer;
            }

            if (b >> bit) & 1 == 1 {
                let vout_index = byte_i * 8 + bit;
                if vout_index >= max_vouts {
                    break;
                }

                // LENIENT: Try to read amount, but break loop if it fails (don't return None)
                let amt_compact = match read_compact_size(raw, &mut pos) {
                    Some(v) => v,
                    None => break 'outer, // Can't read more, but keep what we have
                };
                let amount = decompress_amount(amt_compact);

                // PIVX ScriptCompression format (from compressor.h):
                // 1. Read VARINT nSize
                // 2. If nSize < 6: special compressed script
                //    - Types 0-1: 20 bytes (P2PKH/P2SH)
                //    - Types 2-5: 32 bytes (P2PK)
                // 3. If nSize >= 6: non-special script
                //    - Actual script size = nSize - 6

                let nsize = match read_compact_size(raw, &mut pos) {
                    Some(v) => v as usize,
                    None => break 'outer,
                };

                let script_comp: Vec<u8>;

                const N_SPECIAL_SCRIPTS: usize = 6;

                if nsize < N_SPECIAL_SCRIPTS {
                    // Special compressed script - read fixed-size data
                    let data_len = if nsize <= 1 { 20 } else { 32 };
                    if pos + data_len > raw.len() {
                        // Not enough data for this script
                        break 'outer;
                    }
                    // For special scripts, we need to include the type byte + data
                    script_comp = {
                        let mut v = Vec::with_capacity(1 + data_len);
                        v.push(nsize as u8); // Type byte (0-5)
                        v.extend_from_slice(&raw[pos..pos + data_len]);
                        v
                    };
                    pos += data_len;
                } else {
                    // Non-special script: actual size = nSize - 6
                    let script_len = nsize - N_SPECIAL_SCRIPTS;
                    if script_len > 10000 {
                        // Sanity check - scripts shouldn't be this large
                        break 'outer;
                    }
                    if pos + script_len > raw.len() {
                        break 'outer;
                    }
                    script_comp = raw[pos..pos + script_len].to_vec();
                    pos += script_len;
                }

                let script = decompress_script(&script_comp);

                // Try to identify special PIVX output kinds (staking, zerocoin, sapling)
                let cs = CScript {
                    script: script.clone(),
                };
                let addr_type_opt = crate::address::scriptpubkey_to_address_blocking(&cs);

                // Determine output kind based on flags and script type
                let kind = if is_coinbase {
                    OutputKind::Coinbase
                } else if is_coinstake {
                    // PIVX-specific: coinstake flag from chainstate
                    OutputKind::CoinStake
                } else {
                    // If the script decompresses to an empty vector, this likely represents
                    // a shielded-only (Sapling) output in the chainstate encoding. Treat
                    // empty scripts as Sapling for aggregation heuristics.
                    if script.is_empty() {
                        OutputKind::Sapling
                    } else {
                        match addr_type_opt.as_ref() {
                            Some(AddressType::Staking(staker, owner)) => OutputKind::ColdStake {
                                staker: staker.clone(),
                                owner: owner.clone(),
                            },
                            Some(AddressType::CoinStakeTx) => OutputKind::CoinStake,
                            Some(AddressType::ZerocoinMint) => {
                                OutputKind::Zerocoin("mint".to_string())
                            }
                            Some(AddressType::ZerocoinSpend) => {
                                OutputKind::Zerocoin("spend".to_string())
                            }
                            Some(AddressType::ZerocoinPublicSpend) => {
                                OutputKind::Zerocoin("publicspend".to_string())
                            }
                            Some(AddressType::Sapling) => OutputKind::Sapling,
                            Some(_) => OutputKind::Standard,
                            None => OutputKind::Unknown,
                        }
                    }
                };

                // Resolve address strings now (once) to avoid re-resolving later in aggregator
                let mut resolved_addrs: Vec<String> = Vec::new();
                if let Some(addr_type) = addr_type_opt {
                    resolved_addrs =
                        crate::address::address_type_to_string_blocking(Some(addr_type));
                }

                unspent_outputs.push((vout_index, amount, script, kind, resolved_addrs));
                parsed_count += 1;
            }
        }
    }

    // Allow partial parses - return results even if we didn't get all expected outputs
    // This is LENIENT parsing - we accept incomplete data rather than rejecting everything
    // As long as we got SOME outputs, the data is useful
    if parsed_count > 0 || !unspent_outputs.is_empty() {
        Some(ParsedCoins {
            height,
            is_coinbase,
            unspent_outputs,
        })
    } else {
        // Completely empty or failed to parse anything - return None
        None
    }
}

/// Fully aggregate chainstate balances by address. This opens the copied
/// LevelDB at `chainstate_path`, parses every 'C' entry, and sums amounts
/// per extracted address. Addresses are derived using `address::scriptpubkey_to_address`.
pub fn aggregate_chainstate_balances(
    chainstate_path: &str,
) -> Result<HashMap<String, u64>, Box<dyn std::error::Error>> {
    // Default options: don't include shielded/unknown outputs
    aggregate_chainstate_balances_with_opts(chainstate_path, AggregateOptions::default())
}

/// Options controlling aggregation behavior
pub struct AggregateOptions {
    pub include_shielded: bool,
    pub include_unknown: bool,
    /// Whether to include coinbase outputs in aggregated balances. Default: true
    pub include_coinbase: bool,
    /// If provided alongside `current_height`, coinbase outputs will only be included
    /// when they are matured: current_height >= (coin_height + maturity)
    pub coinbase_maturity: Option<u32>,
    /// Current chain height used for maturity checks (optional).
    pub current_height: Option<u32>,
}

impl Default for AggregateOptions {
    fn default() -> Self {
        AggregateOptions {
            include_shielded: false,
            include_unknown: false,
            include_coinbase: true,
            coinbase_maturity: None,
            current_height: None,
        }
    }
}

/// Aggregate balances with options. Keeps behavior internal-only.
pub fn aggregate_chainstate_balances_with_opts(
    chainstate_path: &str,
    opts: AggregateOptions,
) -> Result<HashMap<String, u64>, Box<dyn std::error::Error>> {
    let agg = aggregate_chainstate_with_coinbase_opts(chainstate_path, opts)?;
    Ok(agg.balances)
}

use serde::Serialize;

#[derive(Serialize)]
pub struct AggregationResult {
    pub balances: HashMap<String, u64>,
    /// Per-address totals that originate from coinbase outputs (maturity/filtering applies when
    /// computing `balances` but `coinbase_balances` always report the raw coinbase split by
    /// resolved address when possible).
    pub coinbase_balances: HashMap<String, u64>,
    /// Sum of all coinbase output amounts encountered (after maturity filtering if opts applied
    /// when computing balances this may differ from coinbase_balances sum present here).
    pub coinbase_total: u64,
}

/// Core aggregator that returns both per-address balances and separate coinbase totals.
pub fn aggregate_chainstate_with_coinbase_opts(
    chainstate_path: &str,
    opts: AggregateOptions,
) -> Result<AggregationResult, Box<dyn std::error::Error>> {
    let raw_map = chainstate_leveldb::read_chainstate_map(chainstate_path)?;
    let mut balances: HashMap<String, u64> = HashMap::new();
    let mut coinbase_balances: HashMap<String, u64> = HashMap::new();
    let mut coinbase_total: u64 = 0;

    for (_key_hex, raw_val) in raw_map.into_iter() {
        if let Some(parsed) = parse_coins_value(&raw_val) {
            for (_vout, amount, script, kind, addrs) in parsed.unspent_outputs {
                if amount == 0 {
                    continue;
                }

                let mut is_coinbase_output = false;
                if let OutputKind::Coinbase = kind {
                    is_coinbase_output = true;
                }

                // For coinbase outputs, always accumulate into coinbase_balances (per-address) when
                // resolvable; also maintain coinbase_total sum. The `balances` map will include
                // coinbase amounts only if opts.include_coinbase and maturity checks pass.
                if is_coinbase_output {
                    coinbase_total = coinbase_total.saturating_add(amount);
                }

                // Determine maturity acceptance for balances
                let mut coinbase_accepted_for_balances = true;
                if is_coinbase_output {
                    if !opts.include_coinbase {
                        coinbase_accepted_for_balances = false;
                    }
                    if let (Some(maturity), Some(current_h)) =
                        (opts.coinbase_maturity, opts.current_height)
                    {
                        let coin_h = parsed.height;
                        if current_h < coin_h.saturating_add(maturity) {
                            coinbase_accepted_for_balances = false;
                        }
                    }
                }

                // Use resolved addresses produced by parse_coins_value when available
                if !addrs.is_empty() {
                    for a in addrs {
                        if a == "Nonstandard" || a == "CoinBaseTx" || a == "CoinStakeTx" {
                            continue;
                        }
                        if is_coinbase_output {
                            let entry = coinbase_balances.entry(a.clone()).or_insert(0);
                            *entry = entry.saturating_add(amount);
                        }
                        if !is_coinbase_output {
                            let entry = balances.entry(a.clone()).or_insert(0);
                            *entry = entry.saturating_add(amount);
                        } else if coinbase_accepted_for_balances {
                            let entry = balances.entry(a.clone()).or_insert(0);
                            *entry = entry.saturating_add(amount);
                        }
                    }
                } else {
                    // Unknown or non-standard script: optionally include under special key
                    // assign to unknown coinbase bucket
                    let key_snip = hex::encode(script.iter().take(6).cloned().collect::<Vec<u8>>());
                    if is_coinbase_output {
                        let key = format!("UNKNOWN_COINBASE_{key_snip}");
                        let entry = coinbase_balances.entry(key).or_insert(0);
                        *entry = entry.saturating_add(amount);
                    }
                    if opts.include_unknown {
                        let key = format!("UNKNOWN_{key_snip}");
                        let entry = balances.entry(key).or_insert(0);
                        *entry = entry.saturating_add(amount);
                    }
                    // Shielded/Sapling outputs are not represented as scriptPubKey; skip unless requested
                    if opts.include_shielded {
                        let key = "SHIELDED_UNKNOWN".to_string();
                        if is_coinbase_output {
                            let entry = coinbase_balances.entry(key.clone()).or_insert(0);
                            *entry = entry.saturating_add(amount);
                        }
                        let entry = balances.entry(key).or_insert(0);
                        *entry = entry.saturating_add(amount);
                    }
                }
            }
        } else {
            // skip parse errors
            continue;
        }
    }

    Ok(AggregationResult {
        balances,
        coinbase_balances,
        coinbase_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_amount_basic() {
        // compressed 0 -> 0
        assert_eq!(decompress_amount(0), 0);
        // compressed 1 -> 1
        assert_eq!(decompress_amount(1), 1);
        // some larger values (sanity checks)
        assert!(decompress_amount(10) > 0);
        assert!(decompress_amount(1000) > decompress_amount(10));
    }

    // ---- PIVX CCoins fixture builders (match parse_coins_value's format) ----

    /// Minimal CompactSize encoder.
    fn compact(n: u64) -> Vec<u8> {
        if n < 253 {
            vec![n as u8]
        } else if n <= 0xffff {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        } else {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }

    /// On-disk compressed-script field for a SPECIAL script: nsize == the type
    /// code (0=P2PKH, 1=P2SH, 2..=5=P2PK) followed by its 20- or 32-byte payload.
    fn special_script(type_code: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![type_code];
        v.extend_from_slice(payload);
        v
    }

    /// On-disk field for a FULL (non-special) script: nsize == script.len() + 6,
    /// then the raw script bytes.
    fn full_script(script: &[u8]) -> Vec<u8> {
        let mut v = compact(script.len() as u64 + 6);
        v.extend_from_slice(script);
        v
    }

    /// Build a CCoins raw value in PIVX encoding:
    ///   code = height*4 + coinbase*2 + coinstake*1, a mask with each `vout` bit
    ///   set, then per-output `compact(amt) ++ script_field` in ascending vout.
    fn coins(
        height: u32,
        coinbase: bool,
        coinstake: bool,
        outs: &[(usize, u64, Vec<u8>)],
    ) -> Vec<u8> {
        let code = height as u64 * 4 + u64::from(coinbase) * 2 + u64::from(coinstake);
        let max_vout = outs.iter().map(|(v, _, _)| *v).max().unwrap_or(0);
        let mask_len = max_vout / 8 + 1;
        let mut mask = vec![0u8; mask_len];
        for (v, _, _) in outs {
            mask[v / 8] |= 1 << (v % 8);
        }
        let mut raw = compact(code);
        raw.extend(compact(mask_len as u64));
        raw.extend_from_slice(&mask);
        let mut sorted: Vec<&(usize, u64, Vec<u8>)> = outs.iter().collect();
        sorted.sort_by_key(|(v, _, _)| *v);
        for (_, amt, field) in sorted {
            raw.extend(compact(*amt));
            raw.extend_from_slice(field);
        }
        raw
    }

    #[test]
    fn test_parse_coins_value_simple_p2pkh() {
        let raw = coins(
            100,
            false,
            false,
            &[(0, 1, special_script(0x00, &[0x11u8; 20]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.height, 100);
        assert!(!parsed.is_coinbase);
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (vout, amount, _script, _kind, _addrs) = &parsed.unspent_outputs[0];
        assert_eq!(*vout, 0usize);
        assert_eq!(*amount, 1u64);
        // P2PKH decompresses to a 25-byte OP_DUP OP_HASH160 ... script.
        let decomp = decompress_script(&special_script(0x00, &[0x11u8; 20]));
        assert_eq!(decomp.len(), 25);
        assert_eq!(decomp[0], 0x76);
        assert_eq!(decomp[1], 0xa9);
    }

    #[test]
    fn test_parse_coins_value_multi_byte_mask() {
        // vout 10 -> mask byte 1, bit 2 (a 2-byte mask).
        let raw = coins(
            300,
            false,
            false,
            &[(10, 1, special_script(0x00, &[0x22u8; 20]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.height, 300);
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (vout, amount, _script, _kind, _addrs) = &parsed.unspent_outputs[0];
        assert_eq!(*vout, 10usize);
        assert_eq!(*amount, 1u64);
    }

    #[test]
    fn test_parse_coins_value_p2sh() {
        let raw = coins(
            50,
            false,
            false,
            &[(0, 5, special_script(0x01, &[0x33u8; 20]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (_vout, _amount, script, _kind, _addrs) = &parsed.unspent_outputs[0];
        // P2SH: OP_HASH160 (0xa9) ... OP_EQUAL (0x87)
        assert_eq!(script[0], 0xa9);
        assert_eq!(script[script.len() - 1], 0x87);
    }

    #[test]
    fn test_parse_coins_value_p2pk_compressed() {
        let raw = coins(
            10,
            false,
            false,
            &[(0, 2, special_script(0x02, &[0x44u8; 32]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (_vout, _amount, script, _kind, _addrs) = &parsed.unspent_outputs[0];
        // Compressed P2PK decompresses to PUSH33 (0x21) <type 0x02> <32-byte key> ...
        assert_eq!(script[0], 0x21);
        assert_eq!(script[1], 0x02);
    }

    #[test]
    fn test_parse_coins_value_zero_amount_present() {
        let raw = coins(
            5,
            false,
            false,
            &[(0, 0, special_script(0x00, &[0x55u8; 20]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (_vout, amount, _script, _kind, _addrs) = &parsed.unspent_outputs[0];
        assert_eq!(*amount, 0u64);
    }

    #[test]
    fn test_parse_coins_value_multiple_vouts() {
        // vouts 0 and 3 set in a single mask byte.
        let raw = coins(
            7,
            false,
            false,
            &[
                (0, 1, special_script(0x00, &[0x66u8; 20])),
                (3, 3, special_script(0x00, &[0x77u8; 20])),
            ],
        );

        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        // ensure amounts match the CompressAmount decompression
        assert_eq!(parsed.unspent_outputs.len(), 2);
        let (v0, a0, _s0, _k0, _addrs0) = &parsed.unspent_outputs[0];
        let (v1, a1, _s1, _k1, _addrs1) = &parsed.unspent_outputs[1];
        assert_eq!(*v0, 0usize);
        assert_eq!(*a0, 1u64);
        assert_eq!(*v1, 3usize);
        assert_eq!(*a1, decompress_amount(3));
    }

    #[test]
    fn test_parse_coins_value_coldstake_detection() {
        // Canonical 51-byte P2CS script (see scriptpubkey_to_staking_address):
        // OP_DUP OP_HASH160 OP_ROT OP_IF OP_CHECKCOLDSTAKEVERIFY 0x14 <staker20>
        // OP_ELSE 0x14 <owner20> OP_ENDIF OP_EQUALVERIFY OP_CHECKSIG.
        let mut cs: Vec<u8> = vec![0x76, 0xa9, 0x7b, 0x63, 0xd2, 0x14];
        cs.extend_from_slice(&[0x11u8; 20]); // staker hash
        cs.push(0x67); // OP_ELSE
        cs.push(0x14);
        cs.extend_from_slice(&[0x22u8; 20]); // owner hash
        cs.extend_from_slice(&[0x68, 0x88, 0xac]); // OP_ENDIF OP_EQUALVERIFY OP_CHECKSIG
        let raw = coins(20, false, false, &[(0, 10, full_script(&cs))]);
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (_v, _a, _s, kind, _addrs) = &parsed.unspent_outputs[0];
        assert!(matches!(kind, OutputKind::ColdStake { .. }));
    }

    #[test]
    fn test_parse_coins_value_zerocoin_detection() {
        // 0xc1 = OP_ZEROCOINMINT.
        let raw = coins(
            2,
            false,
            false,
            &[(0, 1, full_script(&[0xc1u8, 0x00, 0x01]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert_eq!(parsed.unspent_outputs.len(), 1);
        let (_v, _a, _s, kind, _addrs) = &parsed.unspent_outputs[0];
        match kind {
            OutputKind::Zerocoin(sub) => assert_eq!(sub, "mint"),
            _ => panic!("expected Zerocoin kind"),
        }
    }

    #[test]
    fn test_parse_coins_value_coinbase_flag() {
        // code = height*4 + 2 sets the coinbase bit.
        let raw = coins(
            1,
            true,
            false,
            &[(0, 7, special_script(0x00, &[0x11u8; 20]))],
        );
        let parsed = parse_coins_value(&raw).expect("parse should succeed");
        assert!(parsed.is_coinbase);
        let (_v, _a, _s, kind, _addrs) = &parsed.unspent_outputs[0];
        assert!(matches!(kind, OutputKind::Coinbase));
    }
}
