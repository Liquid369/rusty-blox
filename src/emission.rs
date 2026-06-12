//! PIVX mainnet emission schedule (ground truth: PIVX Core v5.6.1).
//!
//! `era_block_reward` is a satoshi-exact transcription of
//! `GetBlockValue(int nHeight)` from PIVX Core `src/validation.cpp` (tag
//! v5.6.1), with the mainnet upgrade heights from `src/chainparams.cpp`
//! (CMainParams) inlined:
//!
//!   consensus.vUpgrades[UPGRADE_POS].nActivationHeight   = 259201;
//!   consensus.vUpgrades[UPGRADE_ZC_V2].nActivationHeight = 1153160;
//!   consensus.vUpgrades[UPGRADE_V5_5].nActivationHeight  = 3715200;
//!   consensus.nMNBlockReward    = 3 * COIN;
//!   consensus.nNewMNBlockReward = 6 * COIN;
//!   consensus.nBudgetCycleBlocks = 43200;  // approx. 1 every 30 days
//!
//! Schedule (mainnet, total minted per block, excluding budget superblocks):
//!
//!   height 1                    : 60_001 PIV   (premine, "6 masternodes")
//!   2        ..=    86_400      : 250 PIV      (PoW)
//!   86_401   ..=   151_200      : 225 PIV      (PoW until 259_200, PoS after)
//!   151_201  ..=   302_400      : 45 PIV
//!   302_401  ..=   345_600      : 40.5 PIV
//!   345_601  ..=   388_800      : 36 PIV
//!   388_801  ..=   432_000      : 31.5 PIV
//!   432_001  ..=   475_200      : 27 PIV
//!   475_201  ..=   518_400      : 22.5 PIV
//!   518_401  ..=   561_600      : 18 PIV
//!   561_601  ..=   604_800      : 13.5 PIV
//!   604_801  ..=   648_000      : 9 PIV
//!   648_001  ..= 1_153_160      : 4.5 PIV
//!   1_153_161..= 3_715_200      : 5 PIV        (Zerocoin v2 era; 3 MN / 2 staker)
//!   3_715_201..                 : 10 PIV       (v5.5 era; 6 MN / 4 staker)
//!
//! Empirical verification against a mainnet PIVX node (RPC, `getblock`
//! verbosity 2 + `getrawtransaction` of every coinstake prevout; minted =
//! coinstake outputs - inputs, or total coinbase outputs in the PoW era):
//!
//!   h=100_000   coinbase outputs = 225.0          == era 225 PIV       OK
//!   h=300_000   coinstake minted = 44.9999896     ~= era 45 PIV        OK
//!               (outputs [0, 1665.8499792, 29.25]; MN seesaw out 29.25)
//!   h=1_000_000 coinstake minted = 4.4999896      ~= era 4.5 PIV       OK
//!               (outputs [0, 527.4599688, 2.34]; MN seesaw out 2.34)
//!   h=2_000_000 coinstake minted = 5.0            == era 5 PIV         OK
//!               (MN out 3.0, staker 2.0 -> matches nMNBlockReward = 3)
//!   h=4_000_000 coinstake minted = 10.0           == era 10 PIV        OK
//!               (MN out 6.0, staker 4.0 -> matches nNewMNBlockReward = 6)
//!   h=5_400_000 coinstake minted = 12_210.0 = 10 era + 12_200 budget   OK
//!   h=5_400_001 coinstake minted = 100_110.0 = 10 era + 100_100 budget OK
//!   h=5_443_200 coinstake minted = 100_810.0 = 10 era + 100_800 budget OK
//!
//! Budget payouts (verified above) are minted ON TOP of GetBlockValue and are
//! NOT part of the era reward: in the PoW era they ride in the coinbase
//! (h=86_400 coinbase paid 1_000_250 = 250 era + 1_000_000 budget; h=129_600
//! paid 325 = 225 era + 100 budget), and in the PoS era they ride inside the
//! COINSTAKE at heights at/after each 43_200-block cycle boundary, one
//! proposal per block (h=1_296_000 minted 11_005 = 5 + 11_000; h=1_296_001
//! minted 205 = 5 + 200; h=2_462_400 minted 27_505 = 5 + 27_500).

/// One PIV in satoshis.
pub const COIN: i64 = 100_000_000;

/// First PoS block (mainnet UPGRADE_POS activation height, chainparams.cpp).
pub const POS_START_HEIGHT: i32 = 259_201;

/// Mainnet UPGRADE_ZC_V2 activation height (chainparams.cpp v5.6.1).
pub const ZC_V2_HEIGHT: i32 = 1_153_160;

/// Mainnet UPGRADE_V5_5 activation height (chainparams.cpp v5.6.1).
pub const V5_5_HEIGHT: i32 = 3_715_200;

/// Budget cycle length (consensus.nBudgetCycleBlocks, ~30 days).
pub const BUDGET_CYCLE_BLOCKS: i32 = 43_200;

/// Total scheduled block reward at `height` in satoshis — exact transcription
/// of PIVX Core v5.6.1 `GetBlockValue(nHeight)` (mainnet branch). Excludes
/// budget superblock payouts, which are minted on top of this value.
pub fn era_block_reward(height: i32) -> i64 {
    if height > V5_5_HEIGHT {
        return 10 * COIN;
    }
    if height > ZC_V2_HEIGHT {
        return 5 * COIN;
    }
    if height > 648_000 {
        return 9 * COIN / 2; // 4.5 PIV
    }
    if height > 604_800 {
        return 9 * COIN;
    }
    if height > 561_600 {
        return 27 * COIN / 2; // 13.5 PIV
    }
    if height > 518_400 {
        return 18 * COIN;
    }
    if height > 475_200 {
        return 45 * COIN / 2; // 22.5 PIV
    }
    if height > 432_000 {
        return 27 * COIN;
    }
    if height > 388_800 {
        return 63 * COIN / 2; // 31.5 PIV
    }
    if height > 345_600 {
        return 36 * COIN;
    }
    if height > 302_400 {
        return 81 * COIN / 2; // 40.5 PIV
    }
    if height > 151_200 {
        return 45 * COIN;
    }
    if height > 86_400 {
        return 225 * COIN;
    }
    if height != 1 {
        return 250 * COIN;
    }
    // Premine for 6 masternodes at block 1.
    60_001 * COIN
}

/// Staker share of the block reward at `height` in satoshis (block reward
/// minus the masternode payment), i.e. what the wallet that won the stake
/// keeps. Returns 0 before PoS activation (no coinstakes exist).
///
/// Masternode payment per PIVX Core v5.6.1 `GetMasternodePayment(nHeight)`:
/// 3 PIV up to and including V5_5_HEIGHT, 6 PIV after. Verified empirically:
/// h=2_000_000 MN coinstake output = 3.0 (staker 2.0); h=4_000_000 MN output
/// = 6.0 (staker 4.0).
///
/// CAVEAT — seesaw era (259_201 ..= 1_153_160): historical PIVX (pre-v4
/// "seesaw", GetSeeSaw in legacy main.cpp) computed the masternode share per
/// block from the live masternode count vs money supply (1%..90% of block
/// value), so the exact split is NOT derivable from height alone. Empirically
/// the MN took 29.25 of 45 (65%) at h=300_000 and 2.34 of 4.5 (52%) at
/// h=1_000_000. We approximate the staker share as blockValue / 2 for that
/// era and document the error bound here; the modern (>= ZC_V2) eras that
/// drive current APY are exact.
pub fn era_staker_reward(height: i32) -> i64 {
    if height < POS_START_HEIGHT {
        return 0; // PoW era: no coinstakes, no staker share.
    }
    if height > V5_5_HEIGHT {
        return 4 * COIN; // 10 PIV block - 6 PIV nNewMNBlockReward
    }
    if height > ZC_V2_HEIGHT {
        return 2 * COIN; // 5 PIV block - 3 PIV nMNBlockReward
    }
    // Seesaw era: per-block MN-count-dependent split, ~50% on average.
    era_block_reward(height) / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_reward_premine_and_pow_boundaries() {
        // Block 1 premine (validation.cpp: "Premine for 6 masternodes").
        assert_eq!(era_block_reward(1), 60_001 * COIN);
        // 250 PIV PoW era through 86_400 inclusive ("> 86400" boundary).
        assert_eq!(era_block_reward(2), 250 * COIN);
        assert_eq!(era_block_reward(86_400), 250 * COIN);
        assert_eq!(era_block_reward(86_401), 225 * COIN);
        // 225 PIV through 151_200 inclusive ("> 151200" boundary).
        assert_eq!(era_block_reward(151_200), 225 * COIN);
        assert_eq!(era_block_reward(151_201), 45 * COIN);
    }

    #[test]
    fn block_reward_pos_step_down_boundaries() {
        // Every "> X" boundary in GetBlockValue, both sides.
        assert_eq!(era_block_reward(302_400), 45 * COIN);
        assert_eq!(era_block_reward(302_401), 4_050_000_000); // 40.5 PIV
        assert_eq!(era_block_reward(345_600), 4_050_000_000);
        assert_eq!(era_block_reward(345_601), 36 * COIN);
        assert_eq!(era_block_reward(388_800), 36 * COIN);
        assert_eq!(era_block_reward(388_801), 3_150_000_000); // 31.5 PIV
        assert_eq!(era_block_reward(432_000), 3_150_000_000);
        assert_eq!(era_block_reward(432_001), 27 * COIN);
        assert_eq!(era_block_reward(475_200), 27 * COIN);
        assert_eq!(era_block_reward(475_201), 2_250_000_000); // 22.5 PIV
        assert_eq!(era_block_reward(518_400), 2_250_000_000);
        assert_eq!(era_block_reward(518_401), 18 * COIN);
        assert_eq!(era_block_reward(561_600), 18 * COIN);
        assert_eq!(era_block_reward(561_601), 1_350_000_000); // 13.5 PIV
        assert_eq!(era_block_reward(604_800), 1_350_000_000);
        assert_eq!(era_block_reward(604_801), 9 * COIN);
        assert_eq!(era_block_reward(648_000), 9 * COIN);
        assert_eq!(era_block_reward(648_001), 450_000_000); // 4.5 PIV
    }

    #[test]
    fn block_reward_modern_era_boundaries() {
        // ZC_V2 boundary (1_153_160): 4.5 -> 5 PIV.
        assert_eq!(era_block_reward(ZC_V2_HEIGHT), 450_000_000);
        assert_eq!(era_block_reward(ZC_V2_HEIGHT + 1), 5 * COIN);
        // V5.5 boundary (3_715_200): 5 -> 10 PIV.
        assert_eq!(era_block_reward(V5_5_HEIGHT), 5 * COIN);
        assert_eq!(era_block_reward(V5_5_HEIGHT + 1), 10 * COIN);
    }

    #[test]
    fn block_reward_matches_node_verified_heights() {
        // Heights verified against a mainnet node (see module docs).
        assert_eq!(era_block_reward(100_000), 225 * COIN);
        assert_eq!(era_block_reward(300_000), 45 * COIN);
        assert_eq!(era_block_reward(1_000_000), 450_000_000);
        assert_eq!(era_block_reward(2_000_000), 5 * COIN);
        assert_eq!(era_block_reward(4_000_000), 10 * COIN);
        assert_eq!(era_block_reward(5_400_000), 10 * COIN);
        assert_eq!(era_block_reward(5_443_200), 10 * COIN);
    }

    #[test]
    fn staker_reward_boundaries() {
        // No staker share before PoS activation.
        assert_eq!(era_staker_reward(0), 0);
        assert_eq!(era_staker_reward(100_000), 0);
        assert_eq!(era_staker_reward(POS_START_HEIGHT - 1), 0);
        // Seesaw era approximation: blockValue / 2.
        assert_eq!(era_staker_reward(POS_START_HEIGHT), 45 * COIN / 2);
        assert_eq!(era_staker_reward(300_000), 45 * COIN / 2);
        assert_eq!(era_staker_reward(1_000_000), 450_000_000 / 2);
        assert_eq!(era_staker_reward(ZC_V2_HEIGHT), 450_000_000 / 2);
        // ZC_V2 era: 5 PIV - 3 PIV MN = 2 PIV (node-verified at 2_000_000).
        assert_eq!(era_staker_reward(ZC_V2_HEIGHT + 1), 2 * COIN);
        assert_eq!(era_staker_reward(2_000_000), 2 * COIN);
        assert_eq!(era_staker_reward(V5_5_HEIGHT), 2 * COIN);
        // V5.5 era: 10 PIV - 6 PIV MN = 4 PIV (node-verified at 4_000_000).
        assert_eq!(era_staker_reward(V5_5_HEIGHT + 1), 4 * COIN);
        assert_eq!(era_staker_reward(4_000_000), 4 * COIN);
        assert_eq!(era_staker_reward(5_443_200), 4 * COIN);
    }
}
