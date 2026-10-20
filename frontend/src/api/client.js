/* =====================================================================
   API CLIENT: one function per core-page endpoint.
   ---------------------------------------------------------------------
   LIVE by default: hits the real rusty-blox backend at the same origin (or
   VITE_API_BASE). Opt into offline MOCK fixtures with VITE_USE_MOCK=1 (dev).

   503 reindex gate: /address, /xpub, /utxo can throw a 503 (err.status).
   Pages must render a "catching up, retry" state, never an empty account.
   All money formatting lives in lib/money.js; never format here.
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
    // Callers branch on err.status; the body names which service is down.
    let msg = 'Address index is reindexing; please retry shortly'
    try {
      const b = await res.json()
      if (typeof b?.error === 'string') msg = b.detail ? `${b.error}: ${b.detail}` : b.error
    } catch { /* non-JSON 503 from a proxy */ }
    const err = new Error(msg)
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
  isMock ? Promise.resolve(mock.blockDetail(height)) : getJSON(`/block-detail/${encodeURIComponent(height)}`)

export const getTx = (txid) =>
  isMock ? Promise.resolve(mock.tx(txid)) : getJSON(`/tx/${encodeURIComponent(txid)}`)

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
  return getJSON(`/address/${encodeURIComponent(addr)}?${q}`)
}

// Blockbook-style balance history: per-bucket {time, txs, received, sent,
// sentToSelf} with satoshi STRINGS; the address page integrates received minus sent
// (BigInt) into the balance curve. 503s during reindex like /address.
export const getBalanceHistory = (addr, groupBy = 86400) =>
  isMock ? Promise.resolve(mock.balanceHistory()) : getJSON(`/balancehistory/${addr}?groupBy=${groupBy}`)

export const getUtxo = (addr) => {
  if (isMock) {
    try { return Promise.resolve(mock.utxo(addr)) } catch (e) { return Promise.reject(e) }
  }
  return getJSON(`/utxo/${encodeURIComponent(addr)}`)
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

// hourly forward-only monitor snapshots (masternode count, mempool, shield
// supply); empty until the monitor writes its first sample. hours clamped
// server-side to 1..8760; ask for the max and chart whatever exists.
export const getSnapshots = (hours = 8760) =>
  isMock ? Promise.resolve(mock.analyticsSnapshots()) : getJSON(`/analytics/snapshots?hours=${hours}`)

// --- masternodes (live RPC proxies; no 503) ---------------------------
export const getMnCount = () =>
  isMock ? Promise.resolve(mock.mnCount()) : getJSON('/mncount')

// NOTE: bare array of ~2k rows, no server pagination; virtualize client-side.
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

// Proposal name may contain spaces/parens; URL-encode it.
// --- governance history ------------------------------------------------
// Prototype: served from gov-mock.json (live node responses captured
// 2026-10-08, shaped like /api/v2/gov/*) until the gov routes ship.
// Mock-only until the gov routes ship, so a production build cannot publish
// prototype data.
const GOV_PROTOTYPE = isMock
// Tested on import.meta.env directly: Vite inlines it, so a production build
// drops the 113 KB mock chunk (the guarded isMock is not constant-folded).
const govMock = () =>
  import.meta.env.VITE_USE_MOCK === '1'
    ? import('./gov-mock.json').then((m) => m.default)
    : Promise.reject(new Error('governance history prototype is mock-only'))

export const getGovProposals = () =>
  GOV_PROTOTYPE
    ? govMock().then((m) => ({ meta: m.meta, enabled: m.enabled, downvoteInvalidLine: m.downvote_invalid_line, rows: m.proposals }))
    : getJSON('/gov/proposals?status=live')
export const getGovHistory = (hash) =>
  GOV_PROTOTYPE ? govMock().then((m) => ({ meta: m.meta, points: m.history[hash] || [] })) : getJSON(`/gov/proposal/${hash}/history?range=all`)
export const getGovBatches = (hash) =>
  GOV_PROTOTYPE ? govMock().then((m) => ({ meta: m.meta, batches: m.batches[hash] || [] })) : getJSON(`/gov/proposal/${hash}/batches`)
export const getGovChanges = (hash) =>
  GOV_PROTOTYPE ? govMock().then((m) => ({ meta: m.meta, rows: m.changes[hash] || [] })) : getJSON(`/gov/proposal/${hash}/changes?limit=1000`)
export const getGovSwings = () =>
  GOV_PROTOTYPE ? govMock().then((m) => ({ meta: m.meta, rows: m.swings })) : getJSON('/gov/swings?window=72h&min=200')

// Held vs counting votes per proposal hash (identity-checked on the backend).
export const getBudgetVoteCounts = () =>
  isMock ? Promise.resolve(mock.budgetVoteCounts()) : getJSON('/budgetvotecounts')
// Budget payments read from a superblock's blocks; no sb = the latest superblock.
export const getBudgetPayouts = (sb) =>
  isMock ? Promise.resolve(mock.budgetPayouts(sb)) : getJSON(sb ? `/budgetpayouts?sb=${sb}` : '/budgetpayouts')

// Budget votes from the node's debug.log via the sidecar; 503 when the
// explorer has no sidecar store configured.
export const getGovLog = (hours = 24) =>
  isMock ? Promise.resolve(mock.govLog(hours)) : getJSON(`/govlog?hours=${hours}`)

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
  return getJSON(`/xpub/${encodeURIComponent(xpub)}?${q}`)
}

// --- universal search classifier --------------------------------------
export const getSearch = (query) =>
  isMock ? Promise.resolve(mock.search(query)) : getJSON(`/search/${encodeURIComponent(query)}`)

// --- price ------------------------------------------------------------
// PIVX market price: { usd, eur, btc, last_updated }; all f64 NUMBERS (btc in
// sci-notation). Degrades to a 200 zero-fallback on upstream failure, so treat
// usd <= 0 as "unavailable" and keep the last good value.
export const getPrice = () =>
  isMock ? Promise.resolve(mock.price()) : getJSON('/price')

// Mainnet consensus.nBudgetCycleBlocks (43,200 blocks, ~30 days). Live callers
// pass the network's value from /status (chain.budgetCycleBlocks).
export const SUPERBLOCK_CYCLE = 43200

// Next superblock height. Mock keeps its frozen constant; live derives it from
// the chain tip so the countdown advances past each superblock instead of
// sticking on a baked-in height (e.g. tip 5,477,016 → 5,486,400 = 127×43,200).
export const nextSuperblock = (tip, cycle = SUPERBLOCK_CYCLE) =>
  isMock || !tip ? mock.nextSuperblock() : Math.ceil(tip / cycle) * cycle

// Current-era monthly treasury cap (PIV). 10 PIV/block budget accrual × 43,200
// blocks/cycle = 432,000, the real network value (matches frontend-legacy
// PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET; confirmed live: every passing /budgetinfo
// proposal's Allotted, Σ ≈ 416,904, fits under it). Mock keeps its own constant
// so the demo's deliberately-over-cap scenario is preserved.
export const monthlyBudgetCap = (live = 432000) => (isMock ? mock.monthlyBudgetCap() : live)

// Expose the 503 toggle so a prototype can demo the reindex state.
export const setAddress503 = mock.setAddress503

export default {
  getStatus, getHealth, getRecentBlocks, getBlockDetail, getTx,
  getAddress, getUtxo, getBalanceHistory, getSupply, getTransactions, getStaking, getNetwork,
  getRichlist, getWealthDistribution, getHodl, getColdstaking, getTreasury, getSnapshots,
  getMnCount, getMnList, getMempool,
  getBudgetInfo, getBudgetProjection, getFinalizedBudgets, getBudgetVotes, getBudgetVoteCounts, getBudgetPayouts, getGovLog, getXpub, getSearch, getPrice,
  nextSuperblock, monthlyBudgetCap,
  setAddress503, isMock, API_BASE
}
