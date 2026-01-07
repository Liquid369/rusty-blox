/**
 * PIVX Governance Status Logic - Core-Faithful Implementation
 * 
 * This module implements PIVX Core's governance rules for proposal status determination.
 * Based on PIVX Core's budget system and the actual data from getbudgetinfo RPC.
 * 
 * PIVX GOVERNANCE RULES (from Core):
 * =================================
 * 
 * 1. PASSING THRESHOLD:
 *    - Requires net votes >= 10% of enabled masternodes
 *    - Net votes = (Yeas - Nays)
 *    - With 2096 MNs: threshold = ceil(2096 * 0.10) = 210 votes
 * 
 * 2. BUDGET CYCLE:
 *    - Superblock pays out once per month (~43,200 blocks)
 *    - Maximum monthly budget: 432,000 PIV
 *    - Proposals are sorted by net votes (descending)
 *    - Top proposals funded until budget exhausted
 * 
 * 3. PROPOSAL LIFECYCLE:
 *    - BlockStart: when voting begins
 *    - BlockEnd: when proposal expires (payments complete)
 *    - TotalPaymentCount: total number of payment cycles
 *    - RemainingPaymentCount: payments still owed
 * 
 * 4. VALIDITY:
 *    - IsValid: proposal meets basic criteria (fee paid, format correct)
 *    - Can lose validity if modified or fee transaction reorged
 * 
 * STATUS DEFINITIONS:
 * ==================
 * 
 * ACTIVE - Proposal is in its voting/payment window
 *   - Current height < BlockEnd
 *   - RemainingPaymentCount may be > 0
 * 
 * PASSING - Proposal meets voting threshold AND will be funded
 *   - IsValid = true
 *   - Net votes >= 10% threshold
 *   - NOT completed (current height < BlockEnd)
 *   - Fits within 432,000 PIV budget (by vote rank)
 * 
 * PASSING_UNFUNDED - Proposal meets voting threshold but WON'T be funded
 *   - IsValid = true
 *   - Net votes >= 10% threshold
 *   - NOT completed
 *   - Does NOT fit within budget (lower-ranked by votes)
 * 
 * FAILING - Proposal does not meet voting threshold
 *   - IsValid = true
 *   - Net votes < 10% threshold
 *   - NOT completed
 * 
 * COMPLETED - Proposal has finished its lifecycle
 *   - Current height >= BlockEnd
 *   - All payments made (RemainingPaymentCount should be 0)
 * 
 * INVALID - Proposal marked invalid by network
 *   - IsValid = false
 *   - Fee transaction reorged, malformed, or other issue
 */

// PIVX Constants
export const PIVX_GOVERNANCE = {
  // Maximum monthly budget allocated to proposals (in PIV)
  MAX_MONTHLY_BUDGET: 432000,
  
  // Passing threshold: 10% of enabled masternodes
  PASSING_THRESHOLD_PERCENT: 0.10,
  
  // Approximate blocks per month (30 days * 1440 min/day * 60 sec/min / 60 sec/block)
  BLOCKS_PER_MONTH: 43200,
}

/**
 * Proposal Status Enum
 */
export const ProposalStatus = {
  ACTIVE: 'active',                    // In voting window
  PASSING: 'passing',                  // Passing votes + funded
  PASSING_UNFUNDED: 'passing_unfunded', // Passing votes but not funded
  FAILING: 'failing',                  // Not meeting vote threshold
  COMPLETED: 'completed',              // Lifecycle finished
  INVALID: 'invalid',                  // Marked invalid by network
}

/**
 * Calculate passing threshold based on masternode count
 * @param {number} enabledMasternodes - Number of enabled masternodes
 * @returns {number} Minimum net votes required to pass
 */
export function calculatePassingThreshold(enabledMasternodes) {
  if (!enabledMasternodes || enabledMasternodes === 0) return 0
  return Math.ceil(enabledMasternodes * PIVX_GOVERNANCE.PASSING_THRESHOLD_PERCENT)
}

/**
 * Check if proposal's voting period has ended
 * @param {Object} proposal - Proposal object
 * @param {number} currentHeight - Current blockchain height
 * @returns {boolean} True if proposal lifecycle is complete
 */
export function isProposalCompleted(proposal, currentHeight) {
  if (!currentHeight || !proposal.BlockEnd) return false
  return currentHeight >= proposal.BlockEnd
}

/**
 * Check if proposal meets the voting threshold
 * @param {Object} proposal - Proposal object
 * @param {number} passingThreshold - Minimum net votes required
 * @returns {boolean} True if proposal meets vote threshold
 */
export function meetsVotingThreshold(proposal, passingThreshold) {
  const netVotes = proposal.Yeas - proposal.Nays
  return netVotes >= passingThreshold
}

/**
 * Calculate which proposals will be funded based on budget constraints
 * 
 * PIVX Core algorithm:
 * 1. Filter to proposals meeting voting threshold
 * 2. Sort by net votes (descending) - highest votes win
 * 3. Allocate budget in order until 432,000 PIV cap reached
 * 
 * @param {Array} proposals - All proposals
 * @param {number} passingThreshold - Minimum net votes required
 * @param {number} currentHeight - Current blockchain height
 * @returns {Object} { funded: Array, unfunded: Array }
 */
export function calculateFundedProposals(proposals, passingThreshold, currentHeight) {
  // Filter to active proposals that meet voting threshold
  const eligible = proposals.filter(p => 
    p.IsValid && 
    !isProposalCompleted(p, currentHeight) &&
    meetsVotingThreshold(p, passingThreshold)
  )
  
  // Sort by net votes (descending) - PIVX Core priority
  const sorted = [...eligible].sort((a, b) => {
    const aVotes = a.Yeas - a.Nays
    const bVotes = b.Yeas - b.Nays
    return bVotes - aVotes
  })
  
  // Allocate budget until cap reached
  const funded = []
  const unfunded = []
  let allocatedBudget = 0
  
  for (const proposal of sorted) {
    const amount = proposal.MonthlyPayment || 0
    
    if (allocatedBudget + amount <= PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET) {
      funded.push(proposal)
      allocatedBudget += amount
    } else {
      unfunded.push(proposal)
    }
  }
  
  return { funded, unfunded }
}

/**
 * Determine the status of a proposal based on PIVX Core rules
 * 
 * @param {Object} proposal - Proposal object with PIVX Core fields
 * @param {number} currentHeight - Current blockchain height  
 * @param {number} passingThreshold - Minimum net votes required (10% of MNs)
 * @param {boolean} isFunded - Whether proposal fits within budget allocation
 * @returns {string} ProposalStatus enum value
 */
export function getProposalStatus(proposal, currentHeight, passingThreshold, isFunded) {
  // Check validity first
  if (!proposal.IsValid) {
    return ProposalStatus.INVALID
  }
  
  // Check if completed
  if (isProposalCompleted(proposal, currentHeight)) {
    return ProposalStatus.COMPLETED
  }
  
  // Check voting threshold
  const meetsThreshold = meetsVotingThreshold(proposal, passingThreshold)
  
  if (!meetsThreshold) {
    return ProposalStatus.FAILING
  }
  
  // Meets threshold - check funding
  if (isFunded) {
    return ProposalStatus.PASSING
  } else {
    return ProposalStatus.PASSING_UNFUNDED
  }
}

/**
 * Get human-readable status label
 * @param {string} status - ProposalStatus enum value
 * @returns {string} Display label
 */
export function getStatusLabel(status) {
  const labels = {
    [ProposalStatus.ACTIVE]: 'Active',
    [ProposalStatus.PASSING]: 'Passing',
    [ProposalStatus.PASSING_UNFUNDED]: 'Passing (Unfunded)',
    [ProposalStatus.FAILING]: 'Failing',
    [ProposalStatus.COMPLETED]: 'Completed',
    [ProposalStatus.INVALID]: 'Invalid',
  }
  return labels[status] || 'Unknown'
}

/**
 * Get badge variant for status
 * @param {string} status - ProposalStatus enum value
 * @returns {string} Badge variant (success, warning, danger, secondary)
 */
export function getStatusVariant(status) {
  const variants = {
    [ProposalStatus.ACTIVE]: 'info',
    [ProposalStatus.PASSING]: 'success',
    [ProposalStatus.PASSING_UNFUNDED]: 'warning',
    [ProposalStatus.FAILING]: 'danger',
    [ProposalStatus.COMPLETED]: 'secondary',
    [ProposalStatus.INVALID]: 'danger',
  }
  return variants[status] || 'secondary'
}

/**
 * Get explanation tooltip for status
 * @param {string} status - ProposalStatus enum value
 * @param {Object} proposal - Proposal object (for contextual info)
 * @returns {string} Explanation text
 */
export function getStatusExplanation(status, proposal) {
  const netVotes = proposal.Yeas - proposal.Nays
  
  const explanations = {
    [ProposalStatus.ACTIVE]: 'Proposal is currently in its voting period',
    [ProposalStatus.PASSING]: `Passing with ${netVotes} net votes and will be funded in the next superblock`,
    [ProposalStatus.PASSING_UNFUNDED]: `Passing with ${netVotes} net votes but will not be funded due to budget constraints (ranked lower than funded proposals)`,
    [ProposalStatus.FAILING]: `Not passing - needs more votes (currently ${netVotes} net votes)`,
    [ProposalStatus.COMPLETED]: 'Proposal has completed its payment cycle',
    [ProposalStatus.INVALID]: 'Proposal is invalid (fee transaction issue or network rejection)',
  }
  return explanations[status] || ''
}

/**
 * Calculate comprehensive governance statistics
 * @param {Array} proposals - All proposals
 * @param {number} currentHeight - Current blockchain height
 * @param {number} enabledMasternodes - Number of enabled masternodes
 * @returns {Object} Statistics object
 */
export function calculateGovernanceStats(proposals, currentHeight, enabledMasternodes) {
  const passingThreshold = calculatePassingThreshold(enabledMasternodes)
  const { funded, unfunded } = calculateFundedProposals(proposals, passingThreshold, currentHeight)
  
  const activeProposals = proposals.filter(p => 
    p.IsValid && !isProposalCompleted(p, currentHeight)
  )
  
  const completedProposals = proposals.filter(p => 
    isProposalCompleted(p, currentHeight)
  )
  
  const failingProposals = activeProposals.filter(p => 
    !meetsVotingThreshold(p, passingThreshold)
  )
  
  const invalidProposals = proposals.filter(p => !p.IsValid)
  
  const allocatedBudget = funded.reduce((sum, p) => sum + (p.MonthlyPayment || 0), 0)
  const remainingBudget = PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET - allocatedBudget
  const budgetUtilization = (allocatedBudget / PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET) * 100
  
  return {
    total: proposals.length,
    active: activeProposals.length,
    passing: funded.length,
    passingUnfunded: unfunded.length,
    failing: failingProposals.length,
    completed: completedProposals.length,
    invalid: invalidProposals.length,
    
    budget: {
      max: PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET,
      allocated: allocatedBudget,
      remaining: remainingBudget,
      utilization: budgetUtilization,
    },
    
    voting: {
      threshold: passingThreshold,
      enabledMasternodes,
      thresholdPercent: PIVX_GOVERNANCE.PASSING_THRESHOLD_PERCENT * 100,
    },
    
    fundedProposals: funded,
    unfundedProposals: unfunded,
  }
}
