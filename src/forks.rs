use std::collections::HashMap;

/// Fork detection utilities.
///
/// Given a `by_hash` map and the canonical `chain_active` (vector of hashes in
/// genesis->tip order), this module helps compute fork branches (side-chains)
/// and nearest common ancestors.

/// Find set of hashes that are part of the active chain.
pub fn active_set(chain_active: &Vec<Vec<u8>>) -> HashMap<Vec<u8>, usize> {
    let mut map = HashMap::new();
    for (i, h) in chain_active.iter().enumerate() {
        map.insert(h.clone(), i);
    }
    map
}

/// Given the full `by_hash` map and canonical `chain_active`, produce a list
/// of side-chain tips (hashes that are present in by_hash but not on the
/// active chain). This is a simple helper; callers can walk each tip back to
/// the main chain using prev_hash references stored in BlockIndexEntry.
pub fn find_sidechain_tips(by_hash: &HashMap<Vec<u8>, super::block_index::BlockIndexEntry>, chain_active: &Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let active = active_set(chain_active);
    let mut tips = Vec::new();

    for (h, _entry) in by_hash.iter() {
        if !active.contains_key(h) {
            // For now consider every non-active block as a potential tip; a
            // more advanced variant would filter only those that have no
            // children in by_hash (true tips).
            tips.push(h.clone());
        }
    }

    tips
}
