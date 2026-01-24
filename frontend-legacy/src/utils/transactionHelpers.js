import { TX_TYPES } from './constants'

/**
 * Detect the type of a PIVX transaction based on its properties
 * @param {Object} tx - Transaction object from API
 * @returns {string} Transaction type constant
 */
export function detectTransactionType(tx) {
  if (!tx) return TX_TYPES.REGULAR

  // Check for coinstake transaction FIRST (block reward from staking/PoS)
  // Coinstake has first input spending a previous output with empty first output
  const isCoinstake = tx.vout?.[0]?.value === '0.00000000' || 
                      (tx.vout?.[0]?.value === '0' && tx.vin?.[0]?.txid)
  
  if (isCoinstake) {
    // IMPORTANT: During superblock windows, coinstake transactions can include budget payments
    // Check if this coinstake has a budget payment attached (large last output)
    if (tx.blockHeight) {
      const superblockHeight = Math.floor(tx.blockHeight / 43200) * 43200
      const blocksAfterSuperblock = tx.blockHeight - superblockHeight
      
      // Within 100 blocks of superblock, check for budget payment in coinstake
      if (blocksAfterSuperblock >= 0 && blocksAfterSuperblock < 100 && tx.vout && tx.vout.length > 2) {
        const lastOutput = tx.vout[tx.vout.length - 1]
        
        // Handle both PIV (string) and satoshi (number) formats
        let lastOutputValue = 0
        if (lastOutput.value !== undefined) {
          const valueStr = String(lastOutput.value)
          lastOutputValue = parseFloat(valueStr) / (valueStr.length > 10 ? 100000000 : 1)
        }
        
        // If last output > 50 PIV, this is a budget payment (not just a coinstake)
        if (lastOutputValue > 50) {
          return TX_TYPES.BUDGET
        }
      }
    }
    
    // Regular coinstake without budget payment
    return TX_TYPES.COINSTAKE
  }

  // Check for budget payment transaction (PIVX Core protocol rules)
  // MUST check BEFORE coinbase because budget payments have coinbase inputs!
  // Budget payments occur at superblocks and can span multiple blocks:
  // 
  // PIVX Budget Payment System:
  // 1. Superblock at height % 43200 == 0 triggers budget cycle
  // 2. If there are N passing proposals, budget payments occur in blocks:
  //    superblock_height, superblock_height+1, ..., superblock_height+(N-1)
  // 3. During budget payment blocks:
  //    - Staker gets FULL block reward (no masternode payment)
  //    - One proposal is paid per block
  // 4. Budget transactions have coinbase-like structure (no inputs)
  // 5. Number of proposals is limited by 432,000 PIV monthly budget
  //    (could be 5 proposals or 50+ proposals depending on payment amounts)
  //
  // PIVX Core Detection Logic (from wallet transaction logic):
  // - Check if transaction has coinbase input
  // - Check if LAST output value > masternode reward (~10-15 PIV)
  // - If credit > mn_reward, it's a budget payment, not MN reward
  //
  if (tx.vin?.[0]?.coinbase && tx.blockHeight) {
    const superblockHeight = Math.floor(tx.blockHeight / 43200) * 43200
    const blocksAfterSuperblock = tx.blockHeight - superblockHeight
    
    // Use generous window (100 blocks) to catch all possible budget payments
    // Theoretical max: 432,000 PIV / ~5,000 PIV per proposal = ~86 proposals
    if (blocksAfterSuperblock >= 0 && blocksAfterSuperblock < 100) {
      // PIVX Core logic: Check if last output exceeds masternode reward
      // Masternode reward is typically 10-15 PIV (varies by network version)
      // Budget payments are always > 100 PIV, so this is a safe threshold
      if (tx.vout && tx.vout.length > 0) {
        const lastOutput = tx.vout[tx.vout.length - 1]
        
        // Handle both PIV (string like "1234.56") and satoshi (number) formats
        let lastOutputValue = 0
        if (lastOutput.value !== undefined) {
          lastOutputValue = parseFloat(lastOutput.value)
        } else if (lastOutput.valueSat !== undefined) {
          // Convert satoshis to PIV (1 PIV = 100000000 satoshis)
          lastOutputValue = parseFloat(lastOutput.valueSat) / 100000000
        }
        
        // If last output > 50 PIV (well above any MN reward), it's a budget payment
        // This matches PIVX Core's logic: credit > mn_reward = BudgetPayment
        if (lastOutputValue > 50) {
          return TX_TYPES.BUDGET
        }
      }
    }
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

  // Check for coinbase transaction (block reward from mining/PoW)
  // This check comes AFTER budget check because budget payments also have coinbase inputs
  if (tx.vin?.[0]?.coinbase) {
    return TX_TYPES.COINBASE
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
