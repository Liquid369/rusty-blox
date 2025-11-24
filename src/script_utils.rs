use bitcoin::util::address::Address;
use bitcoin::network::constants::Network;

/// Script utilities: attempt to extract an Address from a scriptPubKey. This
/// uses the `bitcoin` crate's helper which knows P2PKH/P2SH/P2WPKH/P2WSH
/// templates. PIVX address prefixes differ from Bitcoin's, so final address
/// rendering may need custom prefixing; this helper returns a Bitcoin-style
/// address when possible and otherwise returns None.

/// Try to extract an address from a raw scriptPubKey. Returns the address as
/// a string (bitcoin crate textual representation) when possible.
pub fn extract_address_from_script(script: &[u8]) -> Option<String> {
    let s = bitcoin::Script::from(script.to_vec());
    if let Some(addr) = Address::from_script(&s, Network::Bitcoin) {
        return Some(addr.to_string());
    }
    None
}
