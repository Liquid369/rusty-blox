import { TX_TYPES } from './constants'

/**
 * Detect the type of a PIVX transaction based on its properties
 * @param {Object} tx - Transaction object from API
 * @returns {string} Transaction type constant
 */
export function detectTransactionType(tx) {
  if (!tx) return TX_TYPES.REGULAR

  // Check for coinbase transaction (block reward from mining/PoW)
  if (tx.vin?.[0]?.coinbase) {
    return TX_TYPES.COINBASE
  }

  // Check for coinstake transaction (block reward from staking/PoS)
  // Coinstake has first input spending a previous output with empty first output
  if (tx.vout?.[0]?.value === '0.00000000' && 
      tx.vout?.[0]?.type === 'nonstandard' &&
      tx.vin?.[0]?.txid) {
    return TX_TYPES.COINSTAKE
  }

  // Check for shielded (Sapling) transaction
  // These have shielded spends or outputs
  if (tx.vShieldedSpend?.length > 0 || tx.vShieldedOutput?.length > 0 ||
      tx.saplingData || tx.shieldedSpends || tx.shieldedOutputs) {
    return TX_TYPES.SAPLING
  }

  // Check for cold staking transaction
  // Cold stake outputs have specific script type 'coldstake' or 'coldstaking'
  const hasColdStakeOutput = tx.vout?.some(output => 
    output.type === 'coldstake' || 
    output.type === 'coldstaking' ||
    output.scriptPubKey?.type === 'coldstake'
  )
  if (hasColdStakeOutput) {
    return TX_TYPES.COLDSTAKE
  }

  // Check for budget payment transaction
  // Budget payments typically have specific patterns or can be detected
  // from the outputs going to specific governance addresses
  // This is a simplified check - might need refinement based on actual data
  const hasBudgetOutput = tx.vout?.some(output => 
    output.type === 'zerocoinmint' || // Legacy
    (parseFloat(output.value) > 1000 && tx.vout.length === 1) // Large single output
  )
  if (hasBudgetOutput && !tx.vin?.[0]?.coinbase && tx.vout?.length <= 2) {
    return TX_TYPES.BUDGET
  }

  // Default to regular transaction
  return TX_TYPES.REGULAR
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
    [TX_TYPES.COLDSTAKE]: 'Cold Stake',
    [TX_TYPES.BUDGET]: 'Budget',
    [TX_TYPES.SAPLING]: 'Shielded',
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
    [TX_TYPES.SAPLING]: 'default',
    [TX_TYPES.REGULAR]: 'default'
  }
  return variants[type] || 'default'
}
