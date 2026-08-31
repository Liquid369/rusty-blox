<script setup>
/* =====================================================================
   API REFERENCE — public docs for the /api/v2 surface: a Blockbook v2
   drop-in plus PIVX extensions, rendered from a static descriptor table
   so it stays maintainable next to the code. The money-units section
   mirrors lib/money.js: it is the single most important thing an
   integrator needs to read.
   ===================================================================== */
import Copyable from '../components/Copyable.vue'
import HudPanel from '../components/HudPanel.vue'

const BASE = `${location.protocol}//${location.host}/api/v2`
const WS = `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/websocket`

const GROUPS = [
  {
    title: 'BLOCKBOOK CORE',
    id: 'drop-in surface · wallets point here unchanged',
    endpoints: [
      { m: 'GET', p: '', d: 'Status envelope {blockbook:{bestHeight,inSync,...},backend:{...}}. Also at /api.', ttl: '15s backend' },
      { m: 'GET', p: '/status', d: 'Compact chain tip (PIVX extension; the envelope above is the Blockbook one).', ttl: '5s' },
      { m: 'GET', p: '/block/{height|hash}', q: 'page', d: 'Paginated block with FULL tx objects (1000/page), camelCase links, string nonce/difficulty.', ttl: '60-300s' },
      { m: 'GET', p: '/block-index/{height}', d: '{blockHash} at a height.', ttl: '300s' },
      { m: 'GET', p: '/tx/{txid}', d: 'Full transaction; mempool fallback for unconfirmed (blockHeight -1); confirmations recomputed live on every hit.', ttl: '300s body' },
      { m: 'GET', p: '/tx-specific/{txid}', d: "The node's verbose getrawtransaction, untouched.", ttl: '60s' },
      { m: 'GET', p: '/address/{addr}', q: 'details=basic|txids|txs|txslight · page · pageSize(≤1000) · from · to', d: 'Balance, lifetime totals, paginated ledger newest-first, LIVE unconfirmed balance/txs from the mempool. txslight = full vin/vout without raw hex + sapling proofs.', ttl: '30s + live overlay' },
      { m: 'GET', p: '/xpub/{xpub}', q: 'details=tokens|txids|txs|txslight · gap · tokens=used|derived|nonzero · page', d: 'BIP32 account scan: tokens, aggregate ledger, live unconfirmed.', ttl: '300s + live overlay' },
      { m: 'GET', p: '/utxo/{addr}', q: 'confirmed=true', d: 'Unspent outputs incl. 0-conf mempool UTXOs; outputs spent by pending txs are hidden. confirmed=true = confirmed view only.', ttl: '30s + live overlay' },
      { m: 'GET', p: '/balancehistory/{addr}', q: 'groupBy(secs) · from · to · fiatcurrency', d: 'Buckets {time, txs, received, sent, sentToSelf, rates}. Sum received-sent to reconstruct the balance curve; it lands exactly on the address balance.', ttl: '300s' },
      { m: 'GET', p: '/estimatefee/{blocks}', d: '{"result":"<PIV/kB>"}, floored at the 0.0001 relay minimum.', ttl: '15s' },
      { m: 'GET', p: '/sendtx/{hex}', d: 'Broadcast a raw transaction. {"result":"<txid>"} on success.', ttl: 'none' },
      { m: 'POST', p: '/sendtx', d: 'Broadcast; raw hex as the request body.', ttl: 'none' },
      { m: 'GET', p: '/tickers', q: 'currency · timestamp', d: 'Fiat rates {ts, rates:{usd,eur,btc}}; nearest ticker to the timestamp.', ttl: '60s' },
      { m: 'GET', p: '/tickers-list', q: 'timestamp', d: 'Available fiat currencies at a timestamp.', ttl: '60s' },
    ],
  },
  {
    title: 'WEBSOCKET',
    id: 'blockbook JSON protocol · one socket, subscriptions + queries',
    endpoints: [
      { m: 'WS', p: '/websocket', d: 'Requests {id, method, params} -> {id, data}. Methods: getInfo, getBlockHash, getAccountInfo, getAccountUtxo, getTransaction, getTransactionSpecific, getBalanceHistory, getCurrentFiatRates, getFiatRatesForTimestamps, getFiatRatesTickersList, estimateFee, sendTransaction, subscribeNewBlock, subscribeNewTransaction, subscribeAddresses, unsubscribe*, ping.', ttl: '' },
      { m: 'WS', p: '/ws/blocks · /ws/transactions · /ws/mempool', d: 'Simple one-way push channels (PIVX extension; the explorer UI uses these).', ttl: '' },
    ],
  },
  {
    title: 'GOVERNANCE',
    id: 'PIVX extension · DAO budget, PIV numbers',
    endpoints: [
      { m: 'GET', p: '/budgetinfo', d: 'All retained proposals. RemainingPaymentCount > 0 = still in the running.', ttl: '120s' },
      { m: 'GET', p: '/budgetprojection', d: 'FORWARD projection for the NEXT superblock only.', ttl: '120s' },
      { m: 'GET', p: '/finalizedbudgets', d: 'Finalized budgets, keyed "Name (hash)".', ttl: '120s' },
      { m: 'GET', p: '/budgetvotes/{name}', d: 'Per-masternode votes (URL-encode the name). Map order, not time order.', ttl: '120s' },
      { m: 'POST', p: '/mnrawbudgetvote', d: 'Submit a pre-signed budget vote: {mnTxHash, mnTxIndex, proposalHash, vote:"yes"|"no", time, voteSig(base64)}.', ttl: 'none' },
    ],
  },
  {
    title: 'MASTERNODES',
    id: 'PIVX extension · live RPC proxies',
    endpoints: [
      { m: 'GET', p: '/mncount', d: 'Counts: total, enabled, per-transport, queue.', ttl: '60s' },
      { m: 'GET', p: '/mnlist', d: 'Full roster (~2k rows, bare array).', ttl: '60s' },
      { m: 'GET', p: '/relaymnb/{hex}', d: 'Relay a masternode broadcast.', ttl: 'none' },
    ],
  },
  {
    title: 'ANALYTICS',
    id: 'PIVX extension · precomputed on-chain series',
    endpoints: [
      { m: 'GET', p: '/analytics/supply', q: 'range', d: 'Supply split incl. shield pool.', ttl: '300s' },
      { m: 'GET', p: '/analytics/transactions', q: 'range', d: 'Daily tx-type composition, fees, activity, coin-days destroyed.', ttl: '300s' },
      { m: 'GET', p: '/analytics/staking', q: 'range', d: 'APY estimate, participation, dominance.', ttl: '300s' },
      { m: 'GET', p: '/analytics/network', q: 'range', d: 'Difficulty, orphan rate, block cadence.', ttl: '300s' },
      { m: 'GET', p: '/analytics/richlist', q: 'limit', d: 'Top holders. balance = satoshi STRING.', ttl: '300s' },
      { m: 'GET', p: '/analytics/wealth-distribution', d: 'Histogram + Gini + Nakamoto coefficient.', ttl: '300s' },
      { m: 'GET', p: '/analytics/hodl', d: 'Unspent value by coin age.', ttl: '300s' },
      { m: 'GET', p: '/analytics/coldstaking', q: 'range', d: 'P2CS delegation series.', ttl: '300s' },
      { m: 'GET', p: '/analytics/treasury', d: 'Historical superblock payouts.', ttl: '300s' },
      { m: 'GET', p: '/analytics/snapshots', q: 'hours(1..8760)', d: 'Hourly monitor samples: masternode count, mempool, shield supply.', ttl: '300s' },
      { m: 'GET', p: '/moneysupply', d: 'Total/transparent/shield supply.', ttl: '300s' },
      { m: 'GET', p: '/price', d: 'Live PIVX price {usd, eur, btc} (extension; Blockbook clients use /tickers).', ttl: '60s' },
      { m: 'GET', p: '/search/{query}', d: 'Classifier: height, block hash, txid, address, or xpub.', ttl: '' },
      { m: 'GET', p: '/mempool', d: 'Pending tx snapshot with sizes and fees.', ttl: '' },
    ],
  },
]

const UNITS = [
  { eps: '/tx · /address · /utxo · /xpub · /balancehistory', unit: 'satoshi integer STRINGS', note: 'parse with BigInt; Number() corrupts large balances past 2^53' },
  { eps: '/block per-tx values', unit: 'satoshi STRINGS', note: 'same tx objects as /tx' },
  { eps: '/block-detail per-io value', unit: 'satoshi FLOAT', note: 'raw satoshis as a JSON number (explorer-internal endpoint)' },
  { eps: '/block-detail tx aggregates + sapling.value_balance', unit: 'PIV floats', note: 'value_in / value_out / fees / reward' },
  { eps: 'governance (budget amounts)', unit: 'PIV numbers', note: 'MonthlyPayment, TotalPayment etc.' },
  { eps: 'analytics series', unit: 'mostly PIV', note: 'EXCEPT richlist.balance and transactions.avg_value: satoshi strings' },
]

const EX_ADDR = 'DU8gPC5mh4KxWJARQRxoESFark2jAguBr5'
const EXAMPLES = [
  { label: 'blockbook status', cmd: `curl ${BASE}` },
  { label: 'transaction', cmd: `curl ${BASE}/tx/{txid}` },
  { label: 'address ledger (light)', cmd: `curl '${BASE}/address/${EX_ADDR}?details=txslight&pageSize=25'` },
  { label: 'daily balance history + usd', cmd: `curl '${BASE}/balancehistory/${EX_ADDR}?groupBy=86400&fiatcurrency=usd'` },
  { label: 'websocket', cmd: `wscat -c ${WS} then {"id":"1","method":"getInfo","params":{}}` },
]
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">INTEGRATORS · /api/v2</div>
        <h1 class="page-title">API Reference</h1>
      </div>
      <div class="head-live"><span class="pill cyan mono">BLOCKBOOK v2 DROP-IN</span><span class="pill neon mono">+ PIVX EXTENSIONS</span></div>
    </div>

    <HudPanel title="OVERVIEW" id="base URL + conventions" hero>
      <dl class="kv">
        <dt>Base URL</dt><dd><Copyable :value="BASE"><span class="mono">{{ BASE }}</span></Copyable></dd>
        <dt>WebSocket</dt><dd><Copyable :value="WS"><span class="mono">{{ WS }}</span></Copyable></dd>
        <dt>Drop-in</dt><dd>This API implements the Blockbook v2 contract: shapes, paths, params, the {"error":"..."} string errors, and the /websocket protocol. Wallets and scripts already speaking Blockbook can switch by changing the host.</dd>
        <dt>Caching</dt><dd>Server-side TTLs per endpoint (listed below); heavy computes are single-flight. Mempool-derived fields (unconfirmed balances, 0-conf UTXOs) are overlaid live on every request.</dd>
        <dt>Reindex</dt><dd><span class="mono">/address /xpub /utxo /balancehistory</span> return <span class="pill warn mono">503</span> while the address index rebuilds; retry shortly. Never treat a 503 as a zero balance.</dd>
        <dt>Deep links</dt><dd>Path-style page URLs (<span class="mono">/tx/&lt;txid&gt;</span>, <span class="mono">/address/&lt;addr&gt;</span>, <span class="mono">/block/&lt;h&gt;</span> without /api) redirect to the explorer pages, safe to template in wallets.</dd>
        <dt>Fair use</dt><dd>Open and unauthenticated; global concurrency limits apply. For heavy pipelines, run your own instance (open source).</dd>
      </dl>
    </HudPanel>

    <h2 class="section-title">Money units, read this first</h2>
    <HudPanel title="UNIT CONVENTIONS" id="the integrator footguns">
      <div class="scroll">
        <table class="dtable">
          <thead><tr><th>Endpoints</th><th>Unit</th><th>Note</th></tr></thead>
          <tbody>
            <tr v-for="u in UNITS" :key="u.eps">
              <td class="mono">{{ u.eps }}</td>
              <td class="strong">{{ u.unit }}</td>
              <td class="dim">{{ u.note }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </HudPanel>

    <template v-for="g in GROUPS" :key="g.title">
      <h2 class="section-title">{{ g.title.toLowerCase() }}</h2>
      <HudPanel :title="g.title" :id="g.id">
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th></th><th>Endpoint</th><th>Query</th><th>Description</th><th>Cache</th></tr></thead>
            <tbody>
              <tr v-for="e in g.endpoints" :key="e.p + e.m">
                <td><span class="pill mono" :class="e.m === 'GET' ? 'cyan' : (e.m === 'WS' ? 'neon' : 'warn')">{{ e.m }}</span></td>
                <td><Copyable :value="e.m === 'WS' ? WS : BASE + e.p"><span class="mono strong">{{ e.p || '/' }}</span></Copyable></td>
                <td class="mono dim">{{ e.q || '—' }}</td>
                <td class="dim">{{ e.d }}</td>
                <td class="mono dim">{{ e.ttl || '—' }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>
    </template>

    <h2 class="section-title">examples</h2>
    <HudPanel title="QUICKSTART" id="copy + run">
      <div v-for="ex in EXAMPLES" :key="ex.label" class="ex-row">
        <span class="eyebrow ex-label">{{ ex.label }}</span>
        <Copyable :value="ex.cmd"><code class="mono ex-cmd">{{ ex.cmd }}</code></Copyable>
      </div>
      <p class="dim note">Click any command or endpoint to copy it. Values reflect this instance ({{ BASE }}).</p>
    </HudPanel>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.scroll { overflow-x: auto; }
.ex-row { display: flex; align-items: baseline; gap: 14px; padding: 7px 0; border-bottom: 1px solid var(--hud-line); }
.ex-row:last-of-type { border-bottom: none; }
.ex-label { width: 210px; flex: none; }
.ex-cmd { font-size: 12px; color: var(--text); word-break: break-all; }
.note { margin: var(--space-3) 0 0; font-size: 11.5px; }
</style>
