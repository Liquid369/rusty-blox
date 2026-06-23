import { TX_TYPES } from './constants'

/**
 * Transaction classification helpers.
 *
 * Shapes verified against the live /api/v2/tx endpoint:
 * - Coinbase:  vin[0] has no `txid` (only the coinbase script `hex`), valueIn "0"
 * - Coinstake: vout[0].value === "0" with no addresses AND vin[0] spends a real txid
 * - Shield:    version >= 3 (Sapling). Transparent vin/vout may be empty; some
 *              backends also expose sapling markers (valueBalance / vShieldedSpend)
 * - Cold-Stake (P2CS): output addresses array = [staker (S...), owner (D...)]
 * - Everything else is a transparent transaction
 */

const SATS_PER_PIV = 100000000

/** Parse a satoshi string/number into a Number (display math only). */
export function toSats(value) {
  if (value === null || value === undefined) return 0
  const n = typeof value === 'string' ? parseFloat(value.trim()) : Number(value)
  return Number.isFinite(n) ? n : 0
}

/** True if the input is a coinbase-style input (no previous txid). */
export function isCoinbaseInput(input) {
  if (!input) return false
  if (input.coinbase) return true
  return !input.txid && !input.isAddress
}

/** True if the transaction has a coinbase input. */
export function isCoinbaseTx(tx) {
  return !!tx?.vin?.length && isCoinbaseInput(tx.vin[0])
}

/**
 * True if the transaction is a coinstake (PoS block reward):
 * first output is the empty marker output (value "0", no addresses)
 * and the first input spends a real previous output.
 */
export function isCoinstakeTx(tx) {
  const first = tx?.vout?.[0]
  if (!first) return false
  const zeroValue = first.value === '0' || first.value === '0.00000000'
  const noAddresses = !first.addresses || first.addresses.length === 0
  return zeroValue && noAddresses && !!tx.vin?.[0]?.txid
}

/** True if the transaction carries Sapling (shield) data. */
export function isShieldTx(tx) {
  if (!tx) return false
  if (tx.vShieldedSpend?.length > 0 || tx.vShieldedOutput?.length > 0) return true
  if (tx.valueBalance !== undefined && tx.valueBalance !== null && tx.valueBalance !== 0 && tx.valueBalance !== '0') return true
  if (tx.sapling || tx.saplingData) return true
  // Live API marker: Sapling transactions are serialized with version >= 3
  return (tx.version || 0) >= 3
}

/**
 * Signed net Sapling value the transaction moves. Sourced from either the
 * block-detail shape (`tx.sapling.value_balance`, PIV float) or the live tx
 * endpoint (`tx.valueBalance`). Only the SIGN is used for direction, so the
 * unit/scale is irrelevant.
 *
 * Sign convention (matches the backend — block_detail.rs / sapling_validation.rs):
 *   < 0  value entered the shielded pool  (transparent -> shielded) = shielding
 *   > 0  value left the shielded pool      (shielded -> transparent) = deshielding
 *   = 0  value stayed inside the pool       (shielded -> shielded)    = pure shield
 */
export function getShieldValueBalance(tx) {
  const sap = tx?.sapling
  if (sap && sap.value_balance !== undefined && sap.value_balance !== null) {
    return toSats(sap.value_balance)
  }
  if (tx?.valueBalance !== undefined && tx.valueBalance !== null) {
    return toSats(tx.valueBalance)
  }
  return 0
}

/**
 * Direction of a shielded (Sapling) transaction, by the value-balance sign:
 *   'shielding'   transparent -> shielded (value_balance < 0)
 *   'deshielding' shielded -> transparent (value_balance > 0)
 *   'shield'      fully shielded z->z      (value_balance == 0, or unknown)
 */
export function getShieldDirection(tx) {
  const vb = getShieldValueBalance(tx)
  if (vb < 0) return 'shielding'
  if (vb > 0) return 'deshielding'
  return 'shield'
}

/** True if a detected TX_TYPES value is any shielded variant. */
export function isShieldType(type) {
  return type === TX_TYPES.SAPLING ||
         type === TX_TYPES.SHIELDING ||
         type === TX_TYPES.DESHIELDING
}

/**
 * True if an output is a cold-staking (P2CS) output:
 * addresses = [staker (S...), owner (D...)].
 */
export function isColdStakeOutput(output) {
  const addrs = output?.addresses
  if (!Array.isArray(addrs) || addrs.length < 2) return false
  const hasStaker = addrs.some(a => typeof a === 'string' && a.startsWith('S'))
  const hasOwner = addrs.some(a => typeof a === 'string' && a.startsWith('D'))
  return hasStaker && hasOwner
}

/** True if any output (or input being spent) is a P2CS script. */
export function hasColdStakeOutput(tx) {
  return !!tx?.vout?.some(isColdStakeOutput)
}

/**
 * Map an input/output addresses array to labeled entries.
 * P2CS scripts carry [staker (S...), owner (D...)] - both are returned
 * with their role so views can render and link each one.
 * @returns {Array<{address: string, role: string|null}>}
 */
export function getAddressRoles(io) {
  const addrs = io?.addresses
  if (!Array.isArray(addrs)) return []
  const coldStake = isColdStakeOutput(io)
  return addrs.map(address => {
    let role = null
    if (coldStake) {
      role = address.startsWith('S') ? 'Staker' : 'Owner'
    }
    return { address, role }
  })
}

/** Detect a budget (superblock) payment attached to a coinbase/coinstake. */
function isBudgetPayment(tx) {
  if (!tx.blockHeight || !tx.vout?.length) return false
  const superblockHeight = Math.floor(tx.blockHeight / 43200) * 43200
  const blocksAfterSuperblock = tx.blockHeight - superblockHeight
  if (blocksAfterSuperblock < 0 || blocksAfterSuperblock >= 100) return false
  // Budget payments place a large payout (well above any MN reward) as the last output
  const lastOutput = tx.vout[tx.vout.length - 1]
  return toSats(lastOutput?.value) / SATS_PER_PIV > 50
}

/**
 * Detect the type of a PIVX transaction based on its properties.
 * @param {Object} tx - Transaction object from /api/v2/tx
 * @returns {string} Transaction type constant (TX_TYPES)
 */
export function detectTransactionType(tx) {
  if (!tx) return TX_TYPES.REGULAR

  // Coinstake (PoS block reward) - may carry a budget payment in superblock windows
  if (isCoinstakeTx(tx)) {
    if (tx.vout.length > 2 && isBudgetPayment(tx)) return TX_TYPES.BUDGET
    return TX_TYPES.COINSTAKE
  }

  // Coinbase (PoW reward / budget payout blocks)
  if (isCoinbaseTx(tx)) {
    if (isBudgetPayment(tx)) return TX_TYPES.BUDGET
    return TX_TYPES.COINBASE
  }

  // Shield (Sapling) — classify direction by the signed value balance
  if (isShieldTx(tx)) {
    switch (getShieldDirection(tx)) {
      case 'shielding': return TX_TYPES.SHIELDING
      case 'deshielding': return TX_TYPES.DESHIELDING
      default: return TX_TYPES.SAPLING
    }
  }

  // Cold-staking delegation (P2CS output)
  if (hasColdStakeOutput(tx)) {
    return TX_TYPES.COLDSTAKE
  }

  return TX_TYPES.REGULAR
}

/**
 * Fee rate in satoshi per byte, or null when not applicable
 * (coinbase/coinstake pay no fees).
 * @param {Object} tx - Transaction object from /api/v2/tx
 * @returns {number|null}
 */
export function getFeeRate(tx) {
  const fees = toSats(tx?.fees)
  const size = tx?.size || tx?.vsize
  if (!fees || !size) return null
  return fees / size
}

/**
 * Get a human-readable label for a transaction type
 * @param {string} type - Transaction type constant
 * @returns {string} Display label
 */
export function getTransactionTypeLabel(type) {
  const labels = {
    [TX_TYPES.COINBASE]: 'Coinbase',
    [TX_TYPES.COINSTAKE]: 'Coinstake',
    [TX_TYPES.COLDSTAKE]: 'Cold-Stake',
    [TX_TYPES.BUDGET]: 'Budget',
    [TX_TYPES.SAPLING]: 'Shield',
    [TX_TYPES.SHIELDING]: 'Shielding',
    [TX_TYPES.DESHIELDING]: 'Deshielding',
    [TX_TYPES.REGULAR]: 'Transparent'
  }
  return labels[type] || 'Transaction'
}

/**
 * Get the badge variant color for a transaction type
 * @param {string} type - Transaction type constant
 * @returns {string} Badge variant name
 */
export function getTransactionTypeBadgeVariant(type) {
  const variants = {
    [TX_TYPES.COINBASE]: 'info',
    [TX_TYPES.COINSTAKE]: 'success',
    [TX_TYPES.COLDSTAKE]: 'accent',
    [TX_TYPES.BUDGET]: 'warning',
    [TX_TYPES.SAPLING]: 'info',
    [TX_TYPES.SHIELDING]: 'accent',
    [TX_TYPES.DESHIELDING]: 'warning',
    [TX_TYPES.REGULAR]: 'default'
  }
  return variants[type] || 'default'
}
