/**
 * Format PIV amount with proper decimal places.
 * 
 * Backend API returns satoshi values as strings (e.g., "100000000" = 1 PIV).
 * This function converts from satoshis to PIV by dividing by 100,000,000.
 * 
 * @param {number|string} value - Amount in satoshis from backend
 * @param {number} decimals - Number of decimal places to display (default: 8)
 * @returns {string} Formatted PIV value
 */
export function formatPIV(value, decimals = 8) {
  if (value === null || value === undefined) return '0.00000000'
  
  // Convert from satoshis to PIV
  let satoshis
  if (typeof value === 'string') {
    const trimmed = value.trim()
    if (trimmed.length === 0) return '0.00000000'
    satoshis = parseFloat(trimmed)
  } else {
    satoshis = Number(value)
  }
  
  // Validate
  if (isNaN(satoshis) || !isFinite(satoshis)) return '0.00000000'
  
  // Convert satoshis to PIV (1 PIV = 100,000,000 satoshis)
  const piv = satoshis / 100000000
  return piv.toFixed(decimals)
}

/**
 * Format number with thousand separators
 * @param {number} num - Number to format
 * @returns {string} Formatted number
 */
export function formatNumber(num) {
  if (num === null || num === undefined) return '0'
  return num.toLocaleString('en-US')
}

/**
 * Format percentage with 2 decimal places
 * @param {number} num - Percentage value
 * @returns {string} Formatted percentage
 */
export function formatPercentage(num) {
  if (num === null || num === undefined) return '0.00'
  return parseFloat(num).toFixed(2)
}

/**
 * Format timestamp to human-readable date
 * @param {number} timestamp - Unix timestamp
 * @returns {string} Formatted date string
 */
export function formatDate(timestamp) {
  if (!timestamp) return 'N/A'
  const date = new Date(timestamp * 1000)
  return date.toLocaleString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  })
}

/**
 * Format timestamp to relative time (e.g., "2 hours ago")
 * @param {number} timestamp - Unix timestamp
 * @returns {string} Relative time string
 */
export function formatTimeAgo(timestamp) {
  if (!timestamp) return 'N/A'
  
  const now = Math.floor(Date.now() / 1000)
  const diff = now - timestamp
  
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  if (diff < 2592000) return `${Math.floor(diff / 86400)}d ago`
  if (diff < 31536000) return `${Math.floor(diff / 2592000)}mo ago`
  return `${Math.floor(diff / 31536000)}y ago`
}

/**
 * Format time only from timestamp
 * @param {number} timestamp - Unix timestamp
 * @returns {string} Time string
 */
export function formatTime(timestamp) {
  if (!timestamp) return 'N/A'
  const date = new Date(timestamp * 1000)
  return date.toLocaleTimeString('en-US')
}

/**
 * Truncate hash with ellipsis
 * @param {string} hash - Hash string
 * @param {number} startLen - Characters to show at start (default: 8)
 * @param {number} endLen - Characters to show at end (default: 8)
 * @returns {string} Truncated hash
 */
export function truncateHash(hash, startLen = 8, endLen = 8) {
  if (!hash || hash.length <= startLen + endLen) return hash || ''
  return `${hash.substring(0, startLen)}...${hash.substring(hash.length - endLen)}`
}

/**
 * Format duration in seconds to human-readable format
 * @param {number} seconds - Duration in seconds
 * @returns {string} Formatted duration
 */
export function formatDuration(seconds) {
  if (!seconds || seconds === 0) return '0s'
  
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  const secs = seconds % 60
  
  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${mins}m`
  if (mins > 0) return `${mins}m ${secs}s`
  return `${secs}s`
}

/**
 * Format difficulty number
 * @param {number} diff - Difficulty value
 * @returns {string} Formatted difficulty
 */
export function formatDifficulty(diff) {
  if (!diff) return '0'
  if (diff < 1000) return diff.toFixed(4)
  return diff.toExponential(2)
}

/**
 * Format file size in bytes
 * @param {number} bytes - Size in bytes
 * @returns {string} Formatted size
 */
export function formatBytes(bytes) {
  if (bytes === 0) return '0 B'
  
  const sizes = ['B', 'KB', 'MB', 'GB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  
  return `${(bytes / Math.pow(1024, i)).toFixed(2)} ${sizes[i]}`
}
