//! Chain parameters for the network this instance indexes, picked by
//! RUSTYBLOX_NETWORK (unset or "mainnet", or "testnet6"). Every value here was
//! hard-coded to mainnet, so a testnet6 explorer parsed zero blk*.dat blocks
//! and indexed y... addresses under their D... encoding.

pub struct Params {
    pub magic: [u8; 4],
    pub pubkey_prefix: u8,
    pub script_prefix: u8,
    pub staking_prefix: u8,
    pub exchange_prefix: [u8; 3],
    pub genesis: &'static str,
    /// UPGRADE_POS / UPGRADE_ZC_V2 / UPGRADE_V5_5 activation heights.
    pub pos_height: i32,
    pub zc_v2_height: i32,
    pub v5_5_height: i32,
    /// consensus.nBudgetCycleBlocks.
    pub budget_cycle_blocks: i32,
    /// GetBlockValue's testnet branch: blocks 2..=200 pay 250,000 PIV.
    pub testnet_inflation: bool,
    /// BIP44 coin type (EXT_COIN_TYPE), for derivation path labels.
    pub coin_type: u32,
    /// PIVX Core base58Prefixes[EXT_PUBLIC_KEY]. Bitcoin's 0488b21e "xpub",
    /// which PIVX wallets also export, is accepted on every network.
    pub ext_pubkey_version: [u8; 4],
}

pub(crate) const MAINNET: Params = Params {
    magic: [0x90, 0xc4, 0xfd, 0xe9],
    pubkey_prefix: 30,
    script_prefix: 13,
    staking_prefix: 63,
    exchange_prefix: [0x01, 0xb9, 0xa2],
    genesis: "0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818",
    pos_height: crate::emission::POS_START_HEIGHT,
    zc_v2_height: crate::emission::ZC_V2_HEIGHT,
    v5_5_height: crate::emission::V5_5_HEIGHT,
    budget_cycle_blocks: crate::emission::BUDGET_CYCLE_BLOCKS,
    testnet_inflation: false,
    coin_type: 119,
    ext_pubkey_version: [0x02, 0x2d, 0x25, 0x33],
};

// PIVX CTestNetParams (chainparams.cpp, 2026_testnet6 merge d93a3981b6).
// magic, pubkey, staking and exchange also match live testnet6 data 2026-10-19.
pub(crate) const TESTNET6: Params = Params {
    magic: [0xf6, 0xe7, 0xd6, 0xcb],
    pubkey_prefix: 139,
    script_prefix: 19,
    staking_prefix: 73,
    exchange_prefix: [0x01, 0xb9, 0xb1],
    genesis: "00000f254358fe046377c224341c794b7faf942a6311a7ece773d3c378239c54",
    pos_height: 201,
    zc_v2_height: 275,
    v5_5_height: 650,
    budget_cycle_blocks: 144,
    testnet_inflation: true,
    coin_type: 1,
    ext_pubkey_version: [0x3a, 0x80, 0x61, 0xa0],
};

pub fn params() -> &'static Params {
    static P: std::sync::OnceLock<&'static Params> = std::sync::OnceLock::new();
    P.get_or_init(|| match std::env::var("RUSTYBLOX_NETWORK").as_deref() {
        Err(_) | Ok("") | Ok("mainnet") => &MAINNET,
        Ok("testnet6") => &TESTNET6,
        Ok(other) => panic!("RUSTYBLOX_NETWORK={other}: expected mainnet or testnet6"),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn mainnet_is_default() {
        assert_eq!(super::MAINNET.magic, crate::blk_tail::PIVX_MAGIC);
        assert_eq!(super::MAINNET.genesis.len(), 64);
        assert_eq!(super::TESTNET6.genesis.len(), 64);
    }

    // y7QPbh... received testnet6 coins on 2026-10-02; a wrong pubkey_prefix
    // or checksum path fails here rather than as an empty address page.
    #[test]
    fn testnet6_encodes_live_address() {
        let hash = hex::decode("7255bb45c6dcec2e9a47b014cb71d7e07fc3a6b3").unwrap();
        assert_eq!(
            crate::parser::encode_pivx_address(&hash, super::TESTNET6.pubkey_prefix).as_deref(),
            Some("y7QPbhEav6X8J4DbLFLtVyKg7rttbzCzaH")
        );
        assert_eq!(super::TESTNET6.magic, [0xf6, 0xe7, 0xd6, 0xcb]);
    }
}
