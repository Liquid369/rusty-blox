/* =====================================================================
   Sapling shield-flow shape detection, shared by Block/Transaction/
   AddressDetail. `sapling.value_balance` = net PIV flowing OUT of the
   shield pool, and is a PIV FLOAT on BOTH /tx and /block-detail, the one
   non-satoshi money field in otherwise sat-valued io columns (formatPiv
   ONLY, never formatSats).
   The structural s→s check runs FIRST: a pure shielded tx's value_balance
   equals its fee (positive), so the sign alone would mislabel it
   "de-shielding".
   ===================================================================== */
export const SHIELD = '◈'

// compact ledger tags (address page); PIVX SHIELD naming: s, not Zcash's z
export const SHIELD_TAG = { shielding: 't→s', 'de-shielding': 's→t', shielded: 's→s' }

// 'shielding' (t→s) | 'de-shielding' (s→t) | 'shielded' (s→s) | null.
// null covers: no sapling activity, and the degenerate vb==0-with-transparent-io
// record; never invent a direction for odd data. Callers guard coinbase/coinstake
// themselves (a stake is never reclassified).
export function shieldShape(t) {
  const sap = t?.sapling
  if (!sap || !((sap.shielded_spend_count || 0) > 0 || (sap.shielded_output_count || 0) > 0)) return null
  if (!(t.vin || []).length && !(t.vout || []).length) return 'shielded'
  const vb = sap.value_balance
  if (vb < 0) return 'shielding'
  if (vb > 0) return 'de-shielding'
  return null
}
