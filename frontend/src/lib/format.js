/* =====================================================================
   NON-MONEY FORMATTING HELPERS
   ---------------------------------------------------------------------
   Time, hashes, counts, percentages, difficulty. For anything that is a
   coin AMOUNT, use lib/money.js instead — never format money here.
   ===================================================================== */

/**
 * Relative time from a unix-SECONDS timestamp (the API uses seconds).
 * @param {number|string} tsSeconds
 * @returns {string} e.g. "5m ago", "3d ago"
 */
export function timeAgo(tsSeconds) {
  const ts = Number(tsSeconds)
  if (!ts || !Number.isFinite(ts)) return '—'
  const now = Math.floor(Date.now() / 1000)
  const diff = now - ts
  if (diff < 0) return 'just now'
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  if (diff < 2592000) return `${Math.floor(diff / 86400)}d ago`
  if (diff < 31536000) return `${Math.floor(diff / 2592000)}mo ago`
  return `${Math.floor(diff / 31536000)}y ago`
}

/**
 * Absolute UTC-ish local datetime from unix SECONDS.
 * @param {number|string} tsSeconds
 * @returns {string}
 */
export function formatDateTime(tsSeconds) {
  const ts = Number(tsSeconds)
  if (!ts || !Number.isFinite(ts)) return '—'
  return new Date(ts * 1000).toLocaleString('en-US', {
    year: 'numeric', month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit', second: '2-digit'
  })
}

/**
 * Truncate a hash/txid/address for compact display.
 * @param {string} hash
 * @param {number} [head=10]
 * @param {number} [tail=8]
 * @returns {string}
 */
export function truncateHash(hash, head = 10, tail = 8) {
  if (!hash) return ''
  if (hash.length <= head + tail + 1) return hash
  return `${hash.slice(0, head)}…${hash.slice(-tail)}`
}

/**
 * Compact a large count to K / M / B (e.g. 134913180 -> "134.9M").
 * For COUNTS, not coin amounts.
 * @param {number|string} n
 * @param {number} [decimals=1]
 * @returns {string}
 */
export function compactNumber(n, decimals = 1) {
  const v = Number(n)
  if (!Number.isFinite(v)) return '0'
  const abs = Math.abs(v)
  if (abs < 1000) return String(v)
  const units = [
    { t: 1e9, s: 'B' },
    { t: 1e6, s: 'M' },
    { t: 1e3, s: 'K' }
  ]
  for (const u of units) {
    if (abs >= u.t) {
      return `${(v / u.t).toFixed(decimals).replace(/\.0+$/, '')}${u.s}`
    }
  }
  return String(v)
}

/**
 * Plain thousands-grouped integer (e.g. 1698219 -> "1,698,219").
 * @param {number|string} n
 * @returns {string}
 */
export function formatCount(n) {
  const v = Number(n)
  if (!Number.isFinite(v)) return '0'
  return v.toLocaleString('en-US')
}

/**
 * True when a tx/utxo is not in a block (unconfirmed). The API reports a
 * non-positive height — blockHeight -1 (tx) or height 0 (utxo) — with 0
 * confirmations. This covers BOTH still-pending (in mempool) and dropped/
 * conflicted (evicted, will never confirm) txs; we can't tell them apart from
 * the address index alone, so render a neutral "UNCONFIRMED" marker (never a
 * raw "-1" or a /block/-1 link), not "MEMPOOL".
 * @param {number|string} height
 * @returns {boolean}
 */
export const isUnconfirmedHeight = (height) => !(Number(height) > 0)

/**
 * Format a percentage (API rates are already 0–100 f64).
 * @param {number|string} v
 * @param {number} [decimals=2]
 * @returns {string} e.g. "33.74%"
 */
export function percent(v, decimals = 2) {
  const n = Number(v)
  if (!Number.isFinite(n)) return '0%'
  return `${n.toFixed(decimals)}%`
}

/**
 * Format difficulty (a derived decimal, NOT satoshis).
 * @param {number|string} diff
 * @returns {string}
 */
export function formatDifficulty(diff) {
  const n = Number(diff)
  if (!Number.isFinite(n) || n === 0) return '0'
  if (n < 1000) return n.toFixed(2)
  if (n < 1e6) return n.toLocaleString('en-US', { maximumFractionDigits: 0 })
  return n.toExponential(2)
}

/**
 * Humanize a DURATION in seconds (e.g. masternode activetime). Distinct
 * from timeAgo — this is a span, not a point in time.
 * @param {number|string} seconds
 * @returns {string}
 */
export function formatDuration(seconds) {
  const s = Number(seconds)
  if (!s || !Number.isFinite(s)) return '0s'
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 365) return `${(d / 365).toFixed(1)}y`
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m`
  return `${Math.floor(s)}s`
}

/**
 * Return `url` only when it uses an http(s) scheme; otherwise ''. Guards an
 * attacker-controlled field (e.g. a PIVX proposal URL — Core validates it by
 * LENGTH only, not scheme) from reaching an <a :href> sink as javascript:/data:.
 * @param {string} url
 * @returns {string}
 */
export function safeUrl(url) {
  return /^https?:\/\//i.test(String(url ?? '')) ? String(url) : ''
}

/**
 * Escape HTML metacharacters for safe interpolation into an HTML string. ECharts
 * DOM tooltips parse their formatter's return value as HTML, so any free-form
 * field (a proposal Name) must be escaped before `${...}` interpolation.
 * @param {string|number} s
 * @returns {string}
 */
export function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => (
    { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
  ))
}
