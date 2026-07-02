/* =====================================================================
   COINSTAKE INPUT RECOVERY
   ---------------------------------------------------------------------
   The backend resolves HOT (P2PKH) coinstake inputs but leaves COLD-STAKE
   (P2CS) inputs BLANK on BOTH /tx and /block-detail:
     vin = { txid, vout, address: null, addresses: null, value: null }
   so the staker that minted the block shows as "—" on the input side.

   Recovery is exact, no extra fetch, by two PIVX consensus facts:
   1. A coinstake returns the staked coins to the SAME P2CS script it spent,
      so the input's [staker(S), owner(D)] == the staked output's addresses.
   2. out = in + minted, and a coinstake's minted amount IS the block reward,
      so for a single-input coinstake: consumed value = value_out − reward.

   /tx has no tx_type, so coinstakes are detected STRUCTURALLY (the empty
   marker output) — one code path keeps both detail pages consistent.
   ===================================================================== */

// A PoS coinstake's first output is an empty 0-value marker, and its first
// input spends a real prevout (not a coinbase). Unique to coinstakes.
export function isCoinstakeTx(tx) {
  const o = tx && tx.vout && tx.vout[0]
  const i = tx && tx.vin && tx.vin[0]
  return !!(o && Number(o.value) === 0 && !(o.addresses || []).length &&
            i && i.txid && !i.coinbase)
}

// The staked pay-back output: funded + addressed, preferring the P2CS
// [staker, owner] output over a single-address (hot) one.
export function stakedOutput(tx) {
  const funded = ((tx && tx.vout) || []).filter(
    (o) => (o.addresses || []).length && Number(o.value) > 0)
  return funded.find((o) => (o.addresses || []).length >= 2) || funded[0] || null
}

// True when this vin is a cold-stake input the backend left unresolved.
export function isUnresolvedColdVin(vin) {
  return !!(vin && vin.txid && !(vin.addresses || []).length && vin.value == null)
}

// Addresses to display for an unresolved coinstake input ([staker, owner]),
// or null when the vin is already resolved / this isn't a coinstake.
export function coinstakeInputAddresses(tx, vin) {
  if (!isUnresolvedColdVin(vin) || !isCoinstakeTx(tx)) return null
  const so = stakedOutput(tx)
  return (so && so.addresses) || null
}

// Consumed stake (original value) for a SINGLE-input coinstake, in satoshi:
//   input = value_out − reward
// `reward` is the FULL minted block reward — split between the staker and the
// masternode. value_out already includes the masternode output, so subtracting
// the whole reward nets back to the staker's ORIGINAL stake (== stakerOutput −
// stakerReward). Multi-input coinstakes can't be split per-vin -> null.
export function coinstakeInputValueSat(totalOutSat, rewardSat, vinCount) {
  if (vinCount !== 1 || rewardSat == null || totalOutSat == null) return null
  // BigInt-exact: totalOutSat may be a /tx satoshi STRING above 2^53, so never
  // route it through Number(). Returns a satoshi STRING (formatSats-safe) or null.
  try {
    const v = BigInt(totalOutSat) - BigInt(Math.round(Number(rewardSat)))
    return v >= 0n ? v.toString() : null
  } catch {
    return null
  }
}
