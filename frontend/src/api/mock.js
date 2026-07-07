/* =====================================================================
   MOCK FIXTURES — realistic offline data for every core-page endpoint.
   ---------------------------------------------------------------------
   Shapes + sample values mirror API-INVENTORY.md (probed against the
   live https://explorer.pivx.org). UNITS are preserved exactly so the
   money.js helpers exercise the real footguns:
     - /tx, /address, /utxo, richlist.balance, transactions.avg_value
       -> satoshi STRINGS
     - /block-detail per-io value -> satoshi FLOATS
     - /block-detail aggregates + most analytics -> PIV
   Series are long enough to feed real charts (~120 daily rows, top-100
   richlist, full HODL bands, cumulative coldstaking, etc).
   The client returns this by default (isMock=true).
   ===================================================================== */

// --- deterministic PRNG so fixtures are stable across reloads ----------
function mulberry32(seed) {
  let a = seed >>> 0
  return function () {
    a |= 0; a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const rng = mulberry32(0x42_424242)
const rand = (lo, hi) => lo + rng() * (hi - lo)
const randInt = (lo, hi) => Math.floor(rand(lo, hi + 1))

const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
function fakeAddr(prefix = 'D') {
  let s = prefix
  for (let i = 0; i < 33; i++) s += B58[randInt(0, B58.length - 1)]
  return s
}
function fakeHash() {
  const hx = '0123456789abcdef'
  let s = ''
  for (let i = 0; i < 64; i++) s += hx[randInt(0, 15)]
  return s
}

// --- chain anchors (match the inventory's probe tip ~5,475,975) --------
export const TIP_HEIGHT = 5475975
export const TIP_TIME = 1782719055
const SAMPLE_ADDR = 'DU8gPC5mh4KxWJARQRxoESFark2jAguBr5' // richlist #1 in inventory

// Last analytics point = yesterday (drop_incomplete_trailing_days). Walk
// back N days from this base. Base chosen as 2026-06-28 UTC.
const LAST_DAY = Date.UTC(2026, 5, 28) / 1000
function dayString(daysAgo) {
  const d = new Date((LAST_DAY - daysAgo * 86400) * 1000)
  return d.toISOString().slice(0, 10)
}

const SERIES_LEN = 120

// =====================================================================
// core-explorer
// =====================================================================

export function status() {
  return {
    height: TIP_HEIGHT,
    hash: 'a99bbd03' + fakeHash().slice(8),
    synced: true,
    sync_percentage: 100.0,
    network_height: TIP_HEIGHT
  }
}

export function health() {
  return {
    status: 'healthy',
    database_ok: true,
    address_index_complete: true,
    total_transactions: 134913180,
    valid_transactions: 134913180,
    orphaned_transactions: 0,
    indexed_addresses: 1698219,
    warnings: []
  }
}

// /block-stats/{count} — NEWEST-FIRST, returns count+1 rows (off-by-one).
export function blockStats(count = 30) {
  const rows = []
  let diff = 12984.71
  for (let i = 0; i <= count; i++) {
    const height = TIP_HEIGHT - i
    diff += rand(-300, 300)
    rows.push({
      height,
      hash: fakeHash(),
      time: TIP_TIME - i * randInt(45, 80),
      tx_count: randInt(1, 6),
      size: 80, // header bytes only, per inventory
      difficulty: Number(Math.max(8000, diff).toFixed(5))
    })
  }
  return rows // newest-first; consumer reverses for L->R time axes
}

// /block/{heightOrHash} — header + ordered txid list only.
export function block(heightOrHash) {
  const height = /^\d+$/.test(String(heightOrHash))
    ? Number(heightOrHash)
    : 5475000
  const txids = Array.from({ length: randInt(2, 5) }, () => fakeHash())
  return {
    hash: fakeHash(),
    height,
    version: 11,
    merkleroot: fakeHash(),
    time: TIP_TIME - (TIP_HEIGHT - height) * 60,
    nonce: 0, // PoS
    bits: '1b04a66f',
    difficulty: 14093.15234,
    tx: txids,
    previousblockhash: fakeHash()
  }
}

// /block-detail/{height} — full per-tx detail.
// per-io value = satoshi FLOAT; tx-level aggregates = PIV float.
export function blockDetail(height) {
  const h = Number(height) || 5475000
  const staker = SAMPLE_ADDR
  const payTo = fakeAddr()
  const payFrom = fakeAddr()
  return {
    height: h,
    hash: fakeHash(),
    confirmations: TIP_HEIGHT - h + 1,
    size: 921,
    version: 11,
    merkleroot: fakeHash(),
    time: TIP_TIME - (TIP_HEIGHT - h) * 60,
    nonce: 0,
    bits: '1b04a66f',
    difficulty: 14093.15234,
    previousblockhash: fakeHash(),
    nextblockhash: fakeHash(),
    reward: 10.0, // PIV
    tx: [
      {
        txid: fakeHash(),
        version: 1,
        size: 90,
        locktime: 0,
        vin: [{ coinbase: '03b88a5300', txid: null, vout: null, address: null, addresses: null, value: null, type: null }],
        vout: [{ n: 0, value: 0.0, addresses: [], spent: false, type: 'nonstandard' }],
        value_in: 0.0,
        value_out: 0.0,
        fees: 0.0,
        tx_type: 'coinbase',
        reward: 0.0
      },
      {
        txid: fakeHash(),
        version: 1,
        size: 235,
        locktime: 0,
        vin: [{ txid: fakeHash(), vout: 1, address: staker, addresses: [staker], value: 54430645131.0, coinbase: null, type: 'pubkeyhash' }],
        vout: [
          { n: 0, value: 0.0, addresses: [], spent: false, type: 'nonstandard' },
          { n: 1, value: 54830645131.0, addresses: [staker], spent: false, type: 'pubkeyhash' },
          { n: 2, value: 600000000.0, addresses: [fakeAddr()], spent: false, type: 'pubkeyhash' }
        ],
        value_in: 544.30645131, // PIV
        value_out: 554.30645131,
        fees: 0.0,
        tx_type: 'coinstake',
        reward: 10.0
      },
      {
        // a normal payment tx (multiple vin/vout) for the value-flow view
        txid: fakeHash(),
        version: 1,
        size: 373,
        locktime: 0,
        vin: [
          { txid: fakeHash(), vout: 0, address: payFrom, addresses: [payFrom], value: 320000000.0, coinbase: null, type: 'pubkeyhash' },
          { txid: fakeHash(), vout: 1, address: payFrom, addresses: [payFrom], value: 180000000.0, coinbase: null, type: 'pubkeyhash' }
        ],
        vout: [
          { n: 0, value: 450000000.0, addresses: [payTo], spent: false, type: 'pubkeyhash' },
          { n: 1, value: 49977800.0, addresses: [payFrom], spent: false, type: 'pubkeyhash' }
        ],
        value_in: 5.0,
        value_out: 4.999778,
        fees: 0.000222,
        reward: undefined
      }
    ]
  }
}

// /tx/{txid} — Blockbook camelCase. ALL money = satoshi STRINGS.
export function tx(txid) {
  const from = SAMPLE_ADDR
  const to = fakeAddr()
  const change = fakeAddr()
  return {
    txid: txid || fakeHash(),
    version: 1,
    lockTime: 0,
    vin: [
      { txid: fakeHash(), vout: 0, sequence: 4294967295, n: 0, addresses: [from], isAddress: true, value: '32000000000' },
      { txid: fakeHash(), vout: 1, sequence: 4294967295, n: 1, addresses: [from], isAddress: true, value: '23430645131' }
    ],
    vout: [
      { value: '45000000000', n: 0, hex: '76a914...88ac', addresses: [to], isAddress: true, spent: false },
      { value: '10000000000', n: 1, hex: '76a914...88ac', addresses: [change], isAddress: true, spent: true },
      { value: '430623131', n: 2, hex: '76a914...88ac', addresses: [from], isAddress: true, spent: null }
    ],
    blockHash: fakeHash(),
    blockHeight: 5475000,
    confirmations: 978,
    blockTime: 1782658935,
    size: 373,
    vsize: 373,
    value: '55430623131',
    valueIn: '55430645131',
    fees: '22000',
    hex: '0100000001...'
  }
}

// =====================================================================
// address-xpub-utxo  (money = satoshi STRINGS; 503 togglable)
// =====================================================================

// Flip to true to exercise the reindex 503 state on the address page.
export let ADDRESS_503 = false
export function setAddress503(v) { ADDRESS_503 = !!v }

export function address(addr, { details = 'txs', page = 1, pageSize = 25 } = {}) {
  if (ADDRESS_503) {
    const err = new Error('Address index is reindexing; please retry shortly')
    err.status = 503
    err.body = { error: { message: 'Address index is reindexing; please retry shortly' } }
    throw err
  }
  const txCount = 19
  const txids = Array.from({ length: txCount }, () => fakeHash())
  const transactions =
    details === 'txs'
      ? txids.slice(0, pageSize).map((id, i) => buildAddrTx(id, addr || SAMPLE_ADDR, i))
      : undefined
  return {
    page,
    totalPages: Math.ceil(txCount / pageSize),
    itemsOnPage: Math.min(pageSize, txCount),
    address: addr || SAMPLE_ADDR,
    balance: '3482633087462720', // 34,826,330.87 PIV
    totalReceived: '3700342587460450',
    totalSent: '217709499997730',
    unconfirmedBalance: '0',
    unconfirmedTxs: 0,
    txs: txCount,
    txids: details === 'txids' || details === 'basic' ? txids : undefined,
    transactions
  }
}

function buildAddrTx(id, addr, i) {
  const other = fakeAddr()
  const inbound = i % 2 === 0
  return {
    txid: id,
    version: 1,
    lockTime: 0,
    blockHash: fakeHash(),
    blockHeight: TIP_HEIGHT - i * 1287,
    confirmations: 1 + i * 1287,
    blockTime: TIP_TIME - i * 77000,
    size: 235,
    vsize: 235,
    value: String(randInt(10, 9000) * 100000000),
    valueIn: String(randInt(10, 9000) * 100000000),
    fees: String(randInt(0, 50000)),
    hex: '0100...',
    vin: [{ txid: fakeHash(), vout: 0, n: 0, addresses: [inbound ? other : addr], isAddress: true, value: String(randInt(10, 9000) * 100000000) }],
    vout: [{ value: String(randInt(10, 9000) * 100000000), n: 0, addresses: [inbound ? addr : other], isAddress: true, spent: i % 3 === 0 }]
  }
}

// /utxo/{address} — BARE ARRAY. value = satoshi STRING.
export function utxo(addr) {
  if (ADDRESS_503) {
    const err = new Error('Address index is reindexing; please retry shortly')
    err.status = 503
    throw err
  }
  const out = []
  let confs = 2432
  for (let i = 0; i < 24; i++) {
    confs += randInt(120, 2200)
    const isStake = rng() < 0.6
    out.push({
      txid: fakeHash(),
      vout: randInt(0, 2),
      value: String(randInt(1, 1500000) * 100000), // up to ~1.5M PIV, wide spread
      confirmations: confs,
      height: TIP_HEIGHT - confs,
      coinbase: false,
      coinstake: isStake,
      spendable: true
    })
  }
  // sorted by confirmations ascending (newest first), per inventory
  return out.sort((a, b) => a.confirmations - b.confirmations)
}

// =====================================================================
// analytics-suite
// =====================================================================

// supply — PIV STRINGS; historical always empty.
export function analyticsSupply() {
  return {
    current: {
      total_supply: '104321713.49597794',
      transparent_supply: '103288595.29071736',
      shielded_supply: '1033118.20526058',
      shield_adoption_percentage: 0.990
    },
    historical: []
  }
}

// transactions — richest daily series. avg_value = SATOSHIS string;
// volume/avg_fee = PIV strings; rates = f64.
export function analyticsTransactions() {
  const out = []
  for (let i = SERIES_LEN - 1; i >= 0; i--) {
    const stake = randInt(1380, 1440)
    const payment = randInt(60, 260)
    const coinbase = stake
    const count = stake + payment + coinbase
    const volumePiv = rand(600000, 1600000)
    const avgValueSat = Math.floor((volumePiv / payment) * 1e8)
    out.push({
      date: dayString(i),
      count,
      volume: volumePiv.toFixed(8),
      payment_count: payment,
      stake_count: stake,
      coinbase_count: coinbase,
      avg_value: String(avgValueSat), // SATOSHIS
      avg_fee: rand(0.0001, 0.01).toFixed(8), // PIV
      avg_fee_per_byte: Number(rand(120, 240).toFixed(1)),
      active_addresses: randInt(1700, 2600),
      new_addresses: randInt(0, 40),
      sapling_txs: randInt(0, 25),
      coldstake_txs: randInt(380, 540),
      coin_days_destroyed: Number(rand(4e7, 1.2e8).toFixed(2))
    })
  }
  return out
}

// staking — daily economics. PIV strings + f64 rates.
export function analyticsStaking() {
  const out = []
  for (let i = SERIES_LEN - 1; i >= 0; i--) {
    const apy = rand(7.2, 8.6)
    out.push({
      date: dayString(i),
      participation_rate: Number(rand(22, 26).toFixed(2)),
      total_staked: rand(25500000, 27200000).toFixed(8),
      active_stakers: randInt(260, 330),
      rewards_distributed: rand(13900, 14200).toFixed(8),
      avg_block_time: Number(rand(59.5, 62.5).toFixed(1)),
      avg_stake_size: rand(1200, 1500).toFixed(8),
      apy_estimate: Number(apy.toFixed(2)),
      gross_yield_estimate: Number((apy * 2.5).toFixed(2)),
      top10_dominance: Number(rand(42, 52).toFixed(1))
    })
  }
  return out
}

// network — daily health. difficulty = 2dp STRING; rest f64/u64.
export function analyticsNetwork() {
  const out = []
  let diff = 16000
  for (let i = SERIES_LEN - 1; i >= 0; i--) {
    diff += rand(-400, 500)
    out.push({
      date: dayString(i),
      difficulty: Math.max(9000, diff).toFixed(2),
      orphan_rate: Number(rand(0.5, 4.5).toFixed(2)),
      blocks_per_day: randInt(1390, 1440),
      avg_block_size: randInt(450, 900),
      interval_p95_secs: randInt(120, 200),
      interval_max_secs: randInt(260, 480)
    })
  }
  return out
}

// coldstaking — created/spent/net_cumulative PIV strings; cumulative grows.
export function analyticsColdstaking() {
  const out = []
  let net = 5800000
  const rows = []
  for (let i = SERIES_LEN - 1; i >= 0; i--) {
    const created = rand(150000, 280000)
    const spent = rand(140000, 260000)
    net += created - spent
    rows.push({ daysAgo: i, created, spent, net })
  }
  // rows are newest..oldest in push order? we pushed i descending => oldest first already
  for (const r of rows) {
    out.push({
      date: dayString(r.daysAgo),
      created: r.created.toFixed(8),
      spent: r.spent.toFixed(8),
      net_cumulative: r.net.toFixed(8)
    })
  }
  return out
}

// hodl — point-in-time age bands (PIV strings).
export function analyticsHodl() {
  const total = 103231371.97
  const pct = { '<1m': 30.6, '1-3m': 33.1, '3-6m': 3.4, '6-12m': 6.2, '1-2y': 4.1, '>2y': 22.7 }
  const bands = Object.entries(pct).map(([band, percentage]) => ({
    band,
    value: ((percentage / 100) * total).toFixed(8),
    percentage
  }))
  return { bands, total: total.toFixed(8) }
}

// richlist — top 100. balance = SATOSHIS string.
export function analyticsRichlist(limit = 100) {
  const out = []
  // top holder ~33.74% then a decaying distribution
  let pct = 33.74
  for (let rank = 1; rank <= limit; rank++) {
    const balancePiv = (pct / 100) * 103231371.97
    out.push({
      rank,
      address: rank === 1 ? SAMPLE_ADDR : fakeAddr(),
      balance: String(Math.floor(balancePiv * 1e8)), // SATOSHIS
      percentage: Number(pct.toFixed(4)),
      txCount: randInt(5, 4200)
    })
    pct = Math.max(0.02, pct * rand(0.78, 0.93))
  }
  return out
}

// wealth-distribution — companion: Gini/Nakamoto + 7-bucket histogram.
export function analyticsWealthDistribution() {
  return {
    top_10: 61.2,
    top_50: 79.4,
    top_100: 86.1,
    top_1000: 96.8,
    histogram: [
      { range: '0–1', count: 1402310, percentage: 82.6 },
      { range: '1–10', count: 198440, percentage: 11.7 },
      { range: '10–100', count: 71220, percentage: 4.2 },
      { range: '100–1k', count: 19880, percentage: 1.17 },
      { range: '1k–10k', count: 4910, percentage: 0.29 },
      { range: '10k–100k', count: 940, percentage: 0.055 },
      { range: '>100k', count: 119, percentage: 0.007 }
    ],
    gini: 0.984,
    nakamoto_coefficient: 33
  }
}

// treasury — append-only payout history (PIV strings). Early outliers.
export function analyticsTreasury() {
  const out = [
    { height: 86400, date: '2016-04-03', total_paid: '1000000.00000000', n_outputs: 2 },
    { height: 129601, date: '2016-05-03', total_paid: '500000.00000000', n_outputs: 2 },
    { height: 648001, date: '2017-05-18', total_paid: '4999.99537778', n_outputs: 3 }
  ]
  let height = 700000
  let date = Date.UTC(2017, 6, 1) / 1000
  for (let i = 0; i < 60; i++) {
    height += 43200
    date += 43200 * 60
    out.push({
      height,
      date: new Date(date * 1000).toISOString().slice(0, 10),
      total_paid: rand(28000, 64000).toFixed(8),
      n_outputs: randInt(3, 14)
    })
  }
  return out
}

// =====================================================================
// masternodes  (live RPC proxies; counts are plain — no money fields)
// =====================================================================

// /mncount — ipv4+ipv6+onion == total; inqueue <= enabled.
export function mnCount() {
  return { total: 2111, stable: 2107, enabled: 2111, inqueue: 2097, ipv4: 551, ipv6: 1160, onion: 400 }
}

// /mnlist — API returns the whole ~2k set as a bare array. We mock 22 mixed
// rows. CACHED so a row's collateral outpoint (txhash) is stable across
// navigations — MasternodeDetail re-fetches the list and .find()s by id.
const MN_STATUSES = [
  'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED',
  'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED', 'ENABLED',
  'PRE_ENABLED', 'PRE_ENABLED', 'EXPIRED', 'MISSING', 'NEW_START_REQUIRED', 'ENABLED',
]
const MN_NETS = ['ipv4', 'ipv6', 'onion']
let _mnList = null
export function mnList() {
  if (_mnList) return _mnList
  const now = TIP_TIME
  _mnList = MN_STATUSES.map((status, i) => {
    const neverPaid = status === 'PRE_ENABLED' || status === 'NEW_START_REQUIRED'
    return {
      rank: i,
      type: 'legacy',
      network: MN_NETS[i % 3],
      txhash: fakeHash(),
      outidx: randInt(0, 2),
      pubkey: fakeAddr('D'),
      status,
      addr: fakeAddr('D'),
      version: 70927,
      lastseen: now - randInt(30, 5400),                 // last seen ~last 90m
      activetime: randInt(3 * 86400, 1500 * 86400),      // 3d .. ~4y uptime (duration)
      lastpaid: neverPaid ? 0.0 : now - randInt(1800, 18 * 86400), // unix sec
    }
  })
  return _mnList
}

// =====================================================================
// mempool  (per-tx size/fee are ALWAYS null in the real API)
// =====================================================================

// /mempool — bytes/usage and per-tx size/fee are unavailable (null) by design.
export function mempool() {
  const txs = Array.from({ length: 8 }, () => ({
    txid: fakeHash(), size: null, fee: null, time: TIP_TIME - randInt(5, 900),
  }))
  txs.sort((a, b) => b.time - a.time) // API order is nondeterministic; sort client-side
  return { size: txs.length, bytes: 0, usage: null, transactions: txs }
}

// =====================================================================
// governance  (budgetinfo/projection amounts are PIV f64 numbers; Title-Case)
// =====================================================================

const MN_TOTAL = 2111
const NET_THRESHOLD = 0.1 * MN_TOTAL // funding gate: (Yeas - Nays) must exceed this
const SUPERBLOCK_HEIGHT = 5486400    // next superblock (≈ ceil(tip/43200)*43200)
const MONTHLY_BUDGET_CAP = 432000    // current-era treasury cap (PIV/cycle); demand intentionally exceeds it

function prop(Name, URL, Yeas, Nays, Abstains, Monthly, totalCount, remaining) {
  const start = SUPERBLOCK_HEIGHT - (totalCount - remaining) * 43200
  return {
    Name, URL,
    Hash: fakeHash(), FeeHash: fakeHash(),
    BlockStart: start, BlockEnd: start + totalCount * 43200,
    TotalPaymentCount: totalCount, RemainingPaymentCount: remaining,
    PaymentAddress: fakeAddr('D'),
    Ratio: Yeas + Nays > 0 ? Yeas / (Yeas + Nays) : 0,
    Yeas, Nays, Abstains,
    TotalPayment: Monthly * totalCount, MonthlyPayment: Monthly,
    IsEstablished: true, IsValid: true, Allotted: 0.0, // budgetinfo.Allotted is 0; see /budgetprojection
  }
}

// Built once at import => Hash/PaymentAddress stable across calls.
// Monthly demand (Σ 630k) intentionally exceeds the 432k cap so the Governance
// Budget Simulator has something to cut — the passing subset (490k) alone is
// already over-cap, so even "reset to actual" shows a cap-limited payout.
const BUDGET_PROPOSALS = [
  prop('PIVX-Labs-Core-Dev', 'https://forum.pivx.org/proposal/labs-core-dev', 1510, 22, 0, 120000, 6, 4),
  prop('Core-Maintenance-Q3-2026', 'https://forum.pivx.org/proposal/core-maint-q3', 1240, 60, 12, 90000, 3, 2),
  prop('Marketing-Global-Reach', 'https://forum.pivx.org/proposal/marketing-global', 980, 140, 30, 80000, 4, 3),
  prop('Wallet-Mobile-Team', 'https://forum.pivx.org/proposal/wallet-mobile', 900, 180, 18, 70000, 8, 6),
  prop('Translation-Team-2026', 'https://forum.pivx.org/proposal/translation-2026', 760, 90, 8, 30000, 12, 9),
  prop('Security-Audit-2026', 'https://forum.pivx.org/proposal/security-audit', 700, 220, 24, 60000, 4, 4),
  prop('Community-Events-2026', 'https://forum.pivx.org/proposal/community-events', 540, 250, 40, 40000, 6, 5),
  prop('Exchange-Listing-Fund', 'https://forum.pivx.org/proposal/exchange-listing', 410, 280, 15, 90000, 2, 2),
  prop('Influencer-Campaign-2026', 'https://forum.pivx.org/proposal/influencer-2026', 300, 290, 22, 50000, 3, 3),
]

export function proposalPasses(p) { return (p.Yeas - p.Nays) > NET_THRESHOLD }
export const mnTotal = () => MN_TOTAL
export const nextSuperblock = () => SUPERBLOCK_HEIGHT
export const monthlyBudgetCap = () => MONTHLY_BUDGET_CAP

// /budgetinfo — all proposals (Allotted = 0 here, per the real RPC).
export function budgetInfo() { return BUDGET_PROPOSALS.map((p) => ({ ...p })) }

export function price() {
  return { usd: 0.2143, eur: 0.1985, btc: 0.00000214, last_updated: 1782700000 }
}

// ponytail: empty — the mock demo txs aren't budget collaterals, so this is
// never matched. Add a fixture if a mock budget-finalization tx is introduced.
export function finalizedBudgets() { return {} }

// /budgetprojection — only the funded subset, priority-ordered, with the real
// Allotted populated and a cumulative TotalBudgetAllotted running total.
export function budgetProjection() {
  const ranked = BUDGET_PROPOSALS
    .filter(proposalPasses)
    .sort((a, b) => (b.Yeas - b.Nays) - (a.Yeas - a.Nays))
  // Real PIVX rule: fund each ranked proposal only if it still FITS under the
  // per-cycle cap (greedy-skip); an overflowing proposal is deferred, not paid.
  // So TotalBudgetAllotted never exceeds MONTHLY_BUDGET_CAP (432,000).
  let cum = 0
  const out = []
  for (const p of ranked) {
    if (cum + p.MonthlyPayment > MONTHLY_BUDGET_CAP) continue
    cum += p.MonthlyPayment
    out.push({ ...p, Allotted: p.MonthlyPayment, TotalBudgetAllotted: cum })
  }
  return out
}

// /budgetvotes/{name} — raw RPC array; length != Yeas (includes NO/ABSTAIN +
// invalid/superseded). Unknown name -> [] (a 200, not an error).
export function budgetVotes(name) {
  const p = BUDGET_PROPOSALS.find((x) => x.Name === name)
  if (!p) return []
  const votes = []
  const windowStart = TIP_TIME - 28 * 86400
  const pushN = (n, vote, valid) => {
    for (let i = 0; i < n; i++) {
      votes.push({
        Vote: vote, fValid: valid,
        mnId: `${fakeHash()}-${randInt(0, 2)}`,
        nHash: fakeHash(),
        nTime: windowStart + randInt(0, 28 * 86400),
      })
    }
  }
  pushN(p.Yeas, 'YES', true)
  pushN(p.Nays, 'NO', true)
  pushN(p.Abstains, 'ABSTAIN', true)
  pushN(Math.round(p.Yeas * 0.05), 'YES', false) // superseded / invalid votes
  return votes.sort((a, b) => a.nTime - b.nTime)
}

// =====================================================================
// xpub  (HD account aggregate; money = satoshi STRINGS, like /address)
// =====================================================================

export function xpub(xpubStr, { details = 'tokens', page = 1, pageSize = 25 } = {}) {
  if (ADDRESS_503) {
    const err = new Error('Address index is reindexing; please retry shortly')
    err.status = 503
    throw err
  }
  const derived = 14
  let totalBal = 0n, totalRecv = 0n, totalSent = 0n
  const tokens = []
  for (let i = 0; i < derived; i++) {
    const chain = i % 4 === 3 ? 1 : 0 // path chain index: 0 = receive, 1 = change
    const recv = BigInt(randInt(50, 90000)) * 100000000n
    const sent = BigInt(randInt(0, Number(recv / 100000000n))) * 100000000n
    const bal = recv - sent
    totalRecv += recv; totalSent += sent; totalBal += bal
    tokens.push({
      type: 'XPUBAddress',
      name: fakeAddr('D'),
      path: `m/44'/119'/0'/${chain}/${i}`,
      transfers: randInt(1, 60),
      decimals: 8,
      balance: bal.toString(),
      totalReceived: recv.toString(),
      totalSent: sent.toString(),
    })
  }
  const txCount = tokens.reduce((s, t) => s + t.transfers, 0) // transfers, not unique txs
  const txids = Array.from({ length: Math.min(40, txCount) }, () => fakeHash())
  const transactions = txids
    .slice(0, pageSize)
    .map((id, i) => buildAddrTx(id, tokens[i % tokens.length].name, i))
  // Honor `details` like the real /xpub: tokens XOR transactions per mode (and
  // never txids in tokens/txs modes), so the mock can't mask a view that reads a
  // field its detail mode doesn't actually return.
  const out = {
    page,
    totalPages: Math.ceil(txids.length / pageSize),
    itemsOnPage: Math.min(pageSize, txids.length),
    address: xpubStr,
    balance: totalBal.toString(),
    totalReceived: totalRecv.toString(),
    totalSent: totalSent.toString(),
    unconfirmedBalance: '0',
    unconfirmedTxs: 0,
    txs: txCount,
    usedTokens: derived,
    totalTokens: derived,
  }
  if (details === 'tokens' || details === 'tokenBalances') out.tokens = tokens
  if (details === 'txs') out.transactions = transactions
  if (details === 'txids') out.txids = txids
  return out
}

// =====================================================================
// search — internally-tagged classification (drives a redirect / result card)
// =====================================================================

export function search(query) {
  const s = String(query || '').trim()
  if (/^\d+$/.test(s)) return { type: 'Block', height: Number(s), hash: fakeHash() }
  if (/^[0-9a-fA-F]{64}$/.test(s)) return { type: 'Transaction', txid: s, block_height: randInt(5000000, TIP_HEIGHT) }
  if (/^xpub/i.test(s) && s.length >= 100 && s.length <= 120) return { type: 'XPub', xpub: s }
  if (/^[DS67E]/.test(s) && s.length >= 26 && s.length <= 40) return { type: 'Address', address: s, balance: null }
  return { type: 'NotFound', query: s }
}
