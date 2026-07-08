/* =====================================================================
   API CLIENT — one function per core-page endpoint.
   ---------------------------------------------------------------------
   LIVE by default: hits the real rusty-blox backend at the same origin (or
   VITE_API_BASE). Opt into offline MOCK fixtures with VITE_USE_MOCK=1 (dev).

   503 reindex gate: /address, /xpub, /utxo can throw a 503 (err.status).
   Pages must render a "catching up, retry" state, never an empty account.
   All money formatting lives in lib/money.js — never format here.
   ===================================================================== */

import * as mock from './mock.js'

// Base URL for the real backend. '' = same origin (production is served by the
// same rustyblox host; dev uses the vite proxy). Set VITE_API_BASE for cross-origin.
export const API_BASE =
  (typeof import.meta !== 'undefined' && import.meta.env && import.meta.env.VITE_API_BASE) || ''

// LIVE by default, so a plain `npm run build` ships the real client hitting the
// same-origin API. Opt into offline mock fixtures with VITE_USE_MOCK=1 (dev only).
export const isMock =
  typeof import.meta !== 'undefined' &&
  import.meta.env &&
  import.meta.env.VITE_USE_MOCK === '1'

async function getJSON(path) {
  const res = await fetch(`${API_BASE}/api/v2${path}`, {
    headers: { Accept: 'application/json' }
  })
  if (res.status === 503) {
    const err = new Error('Address index is reindexing; please retry shortly')
    err.status = 503
    throw err
  }
  if (!res.ok) {
    const err = new Error(`HTTP ${res.status} for ${path}`)
    err.status = res.status
    throw err
  }
  return res.json()
}

// --- core-explorer -----------------------------------------------------
export const getStatus = () =>
  isMock ? Promise.resolve(mock.status()) : getJSON('/status')

export const getHealth = () =>
  isMock ? Promise.resolve(mock.health()) : getJSON('/health')

// NOTE: /block-stats returns count+1 rows, newest-first.
export const getRecentBlocks = (count = 30) =>
  isMock ? Promise.resolve(mock.blockStats(count)) : getJSON(`/block-stats/${count}`)

// height-only; per-io values are satoshi FLOATS.
export const getBlockDetail = (height) =>
  isMock ? Promise.resolve(mock.blockDetail(height)) : getJSON(`/block-detail/${height}`)

export const getTx = (txid) =>
  isMock ? Promise.resolve(mock.tx(txid)) : getJSON(`/tx/${txid}`)

// --- address / utxo (can 503) -----------------------------------------
export const getAddress = (addr, opts = {}) => {
  if (isMock) {
    try { return Promise.resolve(mock.address(addr, opts)) } catch (e) { return Promise.reject(e) }
  }
  const q = new URLSearchParams({
    details: opts.details || 'txs',
    page: String(opts.page || 1),
    pageSize: String(opts.pageSize || 25)
  })
  return getJSON(`/address/${addr}?${q}`)
}

export const getUtxo = (addr) => {
  if (isMock) {
    try { return Promise.resolve(mock.utxo(addr)) } catch (e) { return Promise.reject(e) }
  }
  return getJSON(`/utxo/${addr}`)
}

// --- analytics (Dashboard + Analytics pages) --------------------------
export const getSupply = (range = '30d') =>
  isMock ? Promise.resolve(mock.analyticsSupply()) : getJSON(`/analytics/supply?range=${range}`)

export const getTransactions = (range = '90d') =>
  isMock ? Promise.resolve(mock.analyticsTransactions()) : getJSON(`/analytics/transactions?range=${range}`)

export const getStaking = (range = '90d') =>
  isMock ? Promise.resolve(mock.analyticsStaking()) : getJSON(`/analytics/staking?range=${range}`)

export const getNetwork = (range = '90d') =>
  isMock ? Promise.resolve(mock.analyticsNetwork()) : getJSON(`/analytics/network?range=${range}`)

export const getRichlist = (limit = 100) =>
  isMock ? Promise.resolve(mock.analyticsRichlist(limit)) : getJSON(`/analytics/richlist?limit=${limit}`)

export const getWealthDistribution = () =>
  isMock ? Promise.resolve(mock.analyticsWealthDistribution()) : getJSON('/analytics/wealth-distribution')

export const getHodl = () =>
  isMock ? Promise.resolve(mock.analyticsHodl()) : getJSON('/analytics/hodl')

export const getColdstaking = (range = '90d') =>
  isMock ? Promise.resolve(mock.analyticsColdstaking()) : getJSON(`/analytics/coldstaking?range=${range}`)

export const getTreasury = () =>
  isMock ? Promise.resolve(mock.analyticsTreasury()) : getJSON('/analytics/treasury')

// --- masternodes (live RPC proxies; no 503) ---------------------------
export const getMnCount = () =>
  isMock ? Promise.resolve(mock.mnCount()) : getJSON('/mncount')

// NOTE: bare array of ~2k rows, no server pagination — virtualize client-side.
export const getMnList = () =>
  isMock ? Promise.resolve(mock.mnList()) : getJSON('/mnlist')

// --- mempool ----------------------------------------------------------
export const getMempool = () =>
  isMock ? Promise.resolve(mock.mempool()) : getJSON('/mempool')

// --- governance -------------------------------------------------------
export const getBudgetInfo = () =>
  isMock ? Promise.resolve(mock.budgetInfo()) : getJSON('/budgetinfo')

export const getBudgetProjection = () =>
  isMock ? Promise.resolve(mock.budgetProjection()) : getJSON('/budgetprojection')

// Finalized budgets the node tracks (mnfinalbudget show): object keyed "Name (hash)"
// with FeeTX/BlockStart/BlockEnd/Proposals/VoteCount/Status. Used to resolve a
// budget-finalization collateral tx to its budget (FeeTX == txid).
export const getFinalizedBudgets = () =>
  isMock ? Promise.resolve(mock.finalizedBudgets()) : getJSON('/finalizedbudgets')

// Proposal name may contain spaces/parens — URL-encode it.
export const getBudgetVotes = (name) =>
  isMock ? Promise.resolve(mock.budgetVotes(name)) : getJSON(`/budgetvotes/${encodeURIComponent(name)}`)

// --- xpub (can 503 like /address) -------------------------------------
export const getXpub = (xpub, opts = {}) => {
  if (isMock) {
    try { return Promise.resolve(mock.xpub(xpub, opts)) } catch (e) { return Promise.reject(e) }
  }
  const q = new URLSearchParams({
    details: opts.details || 'tokens',
    page: String(opts.page || 1),
    pageSize: String(opts.pageSize || 25)
  })
  return getJSON(`/xpub/${xpub}?${q}`)
}

// --- universal search classifier --------------------------------------
export const getSearch = (query) =>
  isMock ? Promise.resolve(mock.search(query)) : getJSON(`/search/${encodeURIComponent(query)}`)

// --- price ------------------------------------------------------------
// PIVX market price: { usd, eur, btc, last_updated } — all f64 NUMBERS (btc in
// sci-notation). Degrades to a 200 zero-fallback on upstream failure, so treat
// usd <= 0 as "unavailable" and keep the last good value.
export const getPrice = () =>
  isMock ? Promise.resolve(mock.price()) : getJSON('/price')

// PIVX budget cycle = consensus.nBudgetCycleBlocks (43,200 blocks, ~30 days).
export const SUPERBLOCK_CYCLE = 43200

// Next superblock height. Mock keeps its frozen constant; live derives it from
// the chain tip so the countdown advances past each superblock instead of
// sticking on a baked-in height (e.g. tip 5,477,016 → 5,486,400 = 127×43,200).
export const nextSuperblock = (tip) =>
  isMock || !tip ? mock.nextSuperblock() : Math.ceil(tip / SUPERBLOCK_CYCLE) * SUPERBLOCK_CYCLE

// Current-era monthly treasury cap (PIV). 10 PIV/block budget accrual × 43,200
// blocks/cycle = 432,000 — the real network value (matches frontend-legacy
// PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET; confirmed live: every passing /budgetinfo
// proposal's Allotted, Σ ≈ 416,904, fits under it). Mock keeps its own constant
// so the demo's deliberately-over-cap scenario is preserved.
export const monthlyBudgetCap = () => (isMock ? mock.monthlyBudgetCap() : 432000)

// Expose the 503 toggle so a prototype can demo the reindex state.
export const setAddress503 = mock.setAddress503

export default {
  getStatus, getHealth, getRecentBlocks, getBlockDetail, getTx,
  getAddress, getUtxo, getSupply, getTransactions, getStaking, getNetwork,
  getRichlist, getWealthDistribution, getHodl, getColdstaking, getTreasury,
  getMnCount, getMnList, getMempool,
  getBudgetInfo, getBudgetProjection, getFinalizedBudgets, getBudgetVotes, getXpub, getSearch, getPrice,
  nextSuperblock, monthlyBudgetCap,
  setAddress503, isMock, API_BASE
}
