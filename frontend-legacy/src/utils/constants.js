/**
 * App-wide constants
 */

// API Configuration
export const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:3005'
export const WS_BASE_URL = import.meta.env.VITE_WS_URL || 'ws://localhost:3005'

// PIVX Constants
export const MAX_SUPPLY = 99100000 // Maximum PIV supply
export const SATOSHIS_PER_PIV = 100000000
export const BLOCK_TIME = 60 // Average block time in seconds
export const CONFIRMATIONS_REQUIRED = 6 // Confirmations for "confirmed" status

// Masternode Constants
export const MASTERNODE_COLLATERAL = 10000 // PIV required for masternode

// Governance Constants
export const MAX_MONTHLY_BUDGET = 432000 // PIV
export const GOVERNANCE_VOTE_THRESHOLD = 0.10 // 10% NET yes votes required

// Pagination
export const DEFAULT_PAGE_SIZE = 25
export const MAX_PAGE_SIZE = 100

// Cache TTL (milliseconds)
export const CACHE_TTL = {
  BLOCK: 60000,        // 1 minute
  TRANSACTION: 300000, // 5 minutes
  ADDRESS: 30000,      // 30 seconds
  CHAIN_STATE: 10000   // 10 seconds
}

// Transaction Types
export const TX_TYPES = {
  COINBASE: 'coinbase',
  COINSTAKE: 'coinstake',
  COLDSTAKE: 'coldstake',
  BUDGET: 'budget',
  SAPLING: 'sapling',
  REGULAR: 'regular'
}

// Masternode Status
export const MN_STATUS = {
  ENABLED: 'ENABLED',
  EXPIRED: 'EXPIRED',
  NEW_START_REQUIRED: 'NEW_START_REQUIRED',
  PRE_ENABLED: 'PRE_ENABLED',
  WATCHDOG_EXPIRED: 'WATCHDOG_EXPIRED',
  REMOVE: 'REMOVE'
}

// Proposal Status
export const PROPOSAL_STATUS = {
  APPROVED: 'APPROVED',
  REJECTED: 'REJECTED',
  PENDING: 'PENDING',
  UNPAID: 'UNPAID',
  INVALID: 'INVALID',
  COMPLETED: 'COMPLETED',
  INSUFFICIENT_VOTES: 'INSUFFICIENT_VOTES'
}

// Error Messages
export const ERROR_MESSAGES = {
  NETWORK_ERROR: 'Unable to connect to the blockchain node. Please try again.',
  NOT_FOUND: 'The requested resource was not found.',
  INVALID_INPUT: 'Invalid input provided.',
  TIMEOUT: 'Request timed out. Please try again.'
}

// Regex Patterns
export const PATTERNS = {
  BLOCK_HEIGHT: /^\d+$/,
  BLOCK_HASH: /^[0-9a-fA-F]{64}$/,
  TX_HASH: /^[0-9a-fA-F]{64}$/,
  ADDRESS: /^[D][a-zA-Z0-9]{33}$/,
  XPUB: /^xpub[a-zA-Z0-9]{107,108}$/
}

export default {
  API_BASE_URL,
  WS_BASE_URL,
  MAX_SUPPLY,
  SATOSHIS_PER_PIV,
  BLOCK_TIME,
  CONFIRMATIONS_REQUIRED,
  MASTERNODE_COLLATERAL,
  MAX_MONTHLY_BUDGET,
  GOVERNANCE_VOTE_THRESHOLD,
  DEFAULT_PAGE_SIZE,
  MAX_PAGE_SIZE,
  CACHE_TTL,
  TX_TYPES,
  MN_STATUS,
  PROPOSAL_STATUS,
  ERROR_MESSAGES,
  PATTERNS
}
