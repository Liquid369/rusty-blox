/* =====================================================================
   MONEY FORMATTING — THE SINGLE MOST IMPORTANT FILE IN THE FOUNDATION
   ---------------------------------------------------------------------
   Money units in the rusty-blox API are NOT global. The SAME concept
   ("a value") arrives as a satoshi STRING on one endpoint, a satoshi
   FLOAT on another, and a PIV decimal on a third. Picking the wrong
   helper silently renders amounts off by 1e8 (or loses precision on
   values above 2^53). EVERY prototype MUST format money through this
   file. Do not parseInt() a satoshi string anywhere.

   ----------------------------------------------------------------------
   WHICH HELPER FOR WHICH ENDPOINT (from API-INVENTORY.md):

   formatSats()  — value is SATOSHIS (string or float). Divides by 1e8.
     - /address, /xpub, /utxo : every value/balance/totalReceived/
       totalSent/fees  ............................. satoshi STRING
     - /tx : value, valueIn, fees, vin/vout value ... satoshi STRING
             (EXCEPT sapling.value_balance -> PIV float, use formatPiv)
     - /block-detail : per-input / per-output value . satoshi FLOAT
     - /analytics/richlist : balance ................ satoshi STRING
     - /analytics/transactions : avg_value .......... satoshi STRING
            (note: avg_fee on the SAME object is PIV -> formatPiv)

   formatPiv()   — value is already PIV (decimal string or f64 number).
     - /block-detail : value_in, value_out, fees, reward,
                       sapling.value_balance ......... PIV float
     - /tx : sapling.value_balance .................. PIV float
     - /analytics/supply : *_supply ................. PIV string
     - /analytics/transactions : volume, avg_fee .... PIV string
     - /analytics/staking : total_staked, rewards_distributed,
                            avg_stake_size ........... PIV string
     - /analytics/hodl : bands[].value, total ....... PIV string
     - /analytics/coldstaking : created, spent,
                                net_cumulative ....... PIV string
     - /analytics/treasury : total_paid ............. PIV string
     - /analytics/snapshots : *_supply_piv .......... PIV f64 number
     - /moneysupply, /budgetinfo, /budgetprojection . PIV f64 number

   formatFiat()    — ordinary fiat number (USD/EUR from /price, f64).
     Guards the documented 0.0 "price unavailable" upstream fallback.

   NOT money: difficulty, percentages/rates, counts. Use lib/format.js.
   ===================================================================== */

const SAT_PER_PIV = 100000000n // 1e8

/**
 * Group the integer part of a numeric string with thousands separators,
 * preserving the fractional part EXACTLY (no rounding).
 * "381780.99997730" -> "381,780.99997730"
 * @param {string|number} numStr
 * @returns {string}
 */
export function groupThousands(numStr) {
  const str = String(numStr)
  const neg = str.startsWith('-')
  const body = neg ? str.slice(1) : str
  const dot = body.indexOf('.')
  const intPart = dot === -1 ? body : body.slice(0, dot)
  const decPart = dot === -1 ? '' : body.slice(dot)
  const grouped = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return `${neg ? '-' : ''}${grouped}${decPart}`
}

/**
 * Format a SATOSHI amount as PIV. BigInt-safe for integer satoshi strings
 * (values regularly exceed Number.MAX_SAFE_INTEGER, e.g.
 * "3482633087462720" = 34,826,330.8746272 PIV — parseInt would corrupt it).
 *
 * Accepts:
 *  - integer satoshi STRING ("54830645131")  -> exact BigInt division
 *  - satoshi FLOAT (54830645131.0, block-detail per-io) -> /1e8
 *  - number/string with a decimal point or exponent -> float path
 *
 * @param {string|number|bigint|null|undefined} sat
 * @param {object} [opts]
 * @param {number} [opts.decimals=8]  fractional digits to show
 * @param {boolean} [opts.group=true] thousands separators
 * @returns {string} PIV amount (no unit suffix)
 */
export function formatSats(sat, opts = {}) {
  const { decimals = 8, group = true } = opts
  if (sat === null || sat === undefined) return zero(decimals, group)

  if (typeof sat === 'bigint') {
    return fromBigIntSats(sat, decimals, group)
  }

  if (typeof sat === 'number') {
    if (!Number.isFinite(sat)) return zero(decimals, group)
    return fromFloat(sat / 1e8, decimals, group)
  }

  const s = String(sat).trim()
  if (s === '') return zero(decimals, group)

  // Floats / exponents (e.g. block-detail "54830645131.0") -> Number path.
  if (/[.eE]/.test(s)) {
    const n = Number(s)
    if (!Number.isFinite(n)) return zero(decimals, group)
    return fromFloat(n / 1e8, decimals, group)
  }

  // Pure integer satoshi string -> exact BigInt conversion.
  if (/^-?\d+$/.test(s)) {
    return fromBigIntSats(BigInt(s), decimals, group)
  }

  // Unrecognized -> safe zero rather than NaN.
  return zero(decimals, group)
}

function fromBigIntSats(sats, decimals, group) {
  const neg = sats < 0n
  const abs = neg ? -sats : sats
  const intPart = abs / SAT_PER_PIV
  const frac = abs % SAT_PER_PIV
  // frac is satoshis remainder (8 digits); slice to requested decimals.
  let fracStr = frac.toString().padStart(8, '0')
  if (decimals <= 0) {
    fracStr = ''
  } else if (decimals < 8) {
    fracStr = fracStr.slice(0, decimals)
  } else if (decimals > 8) {
    fracStr = fracStr.padEnd(decimals, '0')
  }
  const intStr = group ? groupThousands(intPart.toString()) : intPart.toString()
  const body = fracStr ? `${intStr}.${fracStr}` : intStr
  return neg ? `-${body}` : body
}

function fromFloat(piv, decimals, group) {
  const fixed = piv.toFixed(decimals)
  return group ? groupThousands(fixed) : fixed
}

function zero(decimals, group) {
  return fromFloat(0, decimals, group)
}

/**
 * Format an amount that is ALREADY in PIV (decimal string or f64 number).
 * Use for every endpoint whose field is documented as PIV (see the table
 * at the top of this file). Does NOT divide by 1e8.
 *
 * @param {string|number|null|undefined} piv
 * @param {object} [opts]
 * @param {number} [opts.decimals=8]
 * @param {boolean} [opts.group=true]
 * @returns {string}
 */
export function formatPiv(piv, opts = {}) {
  const { decimals = 8, group = true } = opts
  if (piv === null || piv === undefined) return zero(decimals, group)
  const n = typeof piv === 'number' ? piv : parseFloat(String(piv).trim())
  if (!Number.isFinite(n)) return zero(decimals, group)
  return fromFloat(n, decimals, group)
}

/**
 * Convenience: format an ordinary fiat number (USD/EUR from /price).
 * Guards the 0.0 "price unavailable" fallback documented in the inventory.
 * @param {number|string} v
 * @param {string} [symbol='$']
 * @param {number} [decimals=4]
 * @returns {string}
 */
export function formatFiat(v, symbol = '$', decimals = 4) {
  const n = typeof v === 'number' ? v : parseFloat(String(v ?? ''))
  if (!Number.isFinite(n) || n === 0) return 'unavailable'
  return `${symbol}${groupThousands(n.toFixed(decimals))}`
}
