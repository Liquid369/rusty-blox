<script setup>
/* =====================================================================
   ADDRESS DETAIL — account telemetry + reconstructed balance curve +
   UTXO value histogram + coin-age scatter + received/sent split.
   Handles the 503 reindex gate ("catching up, retry").
   UNITS: all /address + /utxo money fields are satoshi STRINGS.
   ===================================================================== */
import { ref, watch, onMounted, computed } from 'vue'
import { getAddress, getUtxo, setAddress503, isMock } from '../api/client.js'
import { formatSats } from '../lib/money.js'
import { timeAgo, truncateHash, formatCount, compactNumber, isUnconfirmedHeight } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, areaFill, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ addr: { type: String, required: true } })
const info = ref(null)
const utxos = ref([])
const reindexing = ref(false)
const err = ref(null)
const demo503 = ref(false)
const loading = ref(true)

async function load() {
  err.value = null; reindexing.value = false; info.value = null; utxos.value = []
  loading.value = true
  try {
    info.value = await getAddress(props.addr, { details: 'txs', pageSize: 25 })
    utxos.value = await getUtxo(props.addr)
  } catch (e) {
    if (e.status === 503) reindexing.value = true
    else err.value = e.message
  } finally {
    loading.value = false
  }
}
function toggle503() {
  demo503.value = !demo503.value
  setAddress503(demo503.value)
  load()
}
onMounted(load)
watch(() => props.addr, load)

const sat2piv = (v) => parseFloat(formatSats(v, { decimals: 8, group: false })) || 0
// Sum UTXO value as BigInt satoshis (the /utxo values are integer satoshi
// strings) so UNSPENT is exact for any address and matches BALANCE — a float
// sum via sat2piv corrupts past 2^53 sats and reads lossily.
const utxoTotal = computed(() => utxos.value.reduce((s, u) => s + BigInt(u.value), 0n).toString())

// Ledger value must be THIS address's delta in each tx (Σ outputs paying it − Σ inputs
// spending it), NOT the tx's grand total — a big exchange batch tx moves far more than this
// address's slice. Satoshi BigInt (values are integer sat strings); memoized (batch txs can
// have hundreds of outputs). toSat guards a stray non-integer so one bad value can't crash.
const toSat = (v) => { try { return BigInt(v || 0) } catch { return 0n } }
const txDeltas = computed(() => {
  const A = props.addr
  const m = {}
  for (const t of (info.value?.transactions || [])) {
    let d = 0n
    for (const o of (t.vout || [])) if ((o.addresses || []).includes(A)) d += toSat(o.value)
    for (const i of (t.vin || [])) if ((i.addresses || []).includes(A)) d -= toSat(i.value)
    const abs = d < 0n ? -d : d
    m[t.txid] = {
      str: (d > 0n ? '+' : d < 0n ? '-' : '') + formatSats(abs, { decimals: 4 }),
      color: d > 0n ? 'var(--cyan)' : d < 0n ? 'var(--rose)' : 'var(--text-muted)',
    }
  }
  return m
})

/* ---------- balance accumulation from the CURRENT UTXO set ----------
   The address tx list (details=txs) is INCOMPLETE for cold-stake owners — it omits the
   P2CS coinstake txs that hold most of the balance — so a forward tx-delta sum
   undercounts badly (it showed ~2.5M for a 34.8M cold-staking account). /utxo carries
   EVERY current coin, incl. cold-staked ones, with its height, so cumulative-by-creation
   ends at the TRUE balance. Timestamps are estimated from confirmation depth (~60s PoS). */
const balanceOption = computed(() => {
  const p = palette()
  const u = utxos.value
  if (!u.length) return baseOption(p)
  const now = Date.now()
  const sorted = [...u].filter((x) => x.height > 0).sort((a, b) => a.height - b.height)
  let bal = 0
  const pts = sorted.map((x) => {
    bal += sat2piv(x.value)
    return [now - (x.confirmations || 0) * 60000, bal]
  })
  return {
    ...baseOption(p),
    grid: { left: 60, right: 16, top: 18, bottom: 26, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${new Date(arr[0].value[0]).toISOString().slice(0,10)}<br/>${compactNumber(arr[0].value[1])} PIV held` },
    xAxis: { type: 'time', axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } }, axisLabel: { color: p.axis, fontSize: 9, fontFamily: 'monospace' }, axisTick: { show: false } },
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
    series: [{
      type: 'line', step: 'end', showSymbol: false, data: pts,
      lineStyle: { color: p.neon, width: 2.4, shadowColor: hexA(p.neon, 0.7), shadowBlur: 12 },
      areaStyle: { color: areaFill(echarts, p.neon, 0.4, 0.01) },
    }],
  }
})

/* ---------- received vs sent donut ---------- */
const flowOption = computed(() => {
  const p = palette()
  if (!info.value) return baseOption(p)
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${d.name}<br/>${compactNumber(d.value)} PIV · ${d.percent}%` },
    series: [{
      type: 'pie', radius: ['56%', '82%'], center: ['50%', '50%'],
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'Received', value: sat2piv(info.value.totalReceived), itemStyle: { color: p.cyan } },
        { name: 'Sent', value: sat2piv(info.value.totalSent), itemStyle: { color: p.rose } },
      ],
    }],
  }
})

/* ---------- UTXO value histogram ---------- */
const BUCKETS = [
  { label: '0–1', lo: 0, hi: 1 }, { label: '1–10', lo: 1, hi: 10 },
  { label: '10–100', lo: 10, hi: 100 }, { label: '100–1k', lo: 100, hi: 1000 },
  { label: '1k–10k', lo: 1000, hi: 10000 }, { label: '10k–100k', lo: 10000, hi: 100000 },
  { label: '>100k', lo: 100000, hi: Infinity },
]
const histOption = computed(() => {
  const p = palette()
  const counts = BUCKETS.map(() => 0)
  for (const u of utxos.value) {
    const v = sat2piv(u.value)
    const idx = BUCKETS.findIndex((b) => v >= b.lo && v < b.hi)
    if (idx >= 0) counts[idx]++
  }
  return {
    ...baseOption(p),
    grid: { left: 36, right: 14, top: 14, bottom: 40, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${arr[0].name} PIV<br/>${arr[0].value} UTXOs` },
    xAxis: catAxis(BUCKETS.map((b) => b.label), p, { boundaryGap: true, axisLabel: { interval: 0, rotate: 30 } }),
    yAxis: valAxis(p, { axisLabel: { formatter: (v) => v } }),
    series: [{
      type: 'bar', data: counts, barWidth: '58%',
      itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [{ offset: 0, color: p.neon }, { offset: 1, color: hexA(p.neon, 0.25) }]), borderRadius: [3, 3, 0, 0] },
    }],
  }
})

/* ---------- coin-age scatter ---------- */
const scatterOption = computed(() => {
  const p = palette()
  const mk = (pred, color, name) => ({
    name, type: 'scatter', symbolSize: (d) => Math.max(7, Math.min(26, Math.sqrt(d[1]) / 6)),
    data: utxos.value.filter(pred).map((u) => [u.confirmations, sat2piv(u.value)]),
    itemStyle: { color: hexA(color, 0.7), borderColor: color, shadowColor: hexA(color, 0.6), shadowBlur: 6 },
  })
  return {
    ...baseOption(p),
    legend: { data: ['stake', 'received'], top: 0, right: 4, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 10, itemHeight: 8 },
    grid: { left: 56, right: 16, top: 30, bottom: 32, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${compactNumber(d.value[1])} PIV<br/>${formatCount(d.value[0])} conf` },
    xAxis: { type: 'value', name: 'confirmations (age →)', nameLocation: 'middle', nameGap: 26, nameTextStyle: { color: p.axis, fontSize: 9, fontFamily: 'monospace' }, splitLine: { lineStyle: { color: 'rgba(150,90,220,0.1)', type: 'dashed' } }, axisLabel: { color: p.axis, fontSize: 9, fontFamily: 'monospace', formatter: (v) => compactNumber(v) }, axisLine: { show: false } },
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
    series: [
      mk((u) => u.coinstake, p.neon, 'stake'),
      mk((u) => !u.coinstake && !u.coinbase, p.cyan, 'received'),
    ],
  }
})

const addrKind = computed(() => {
  const a = props.addr || ''
  if (a.startsWith('S')) return 'COLD-STAKE'
  if (/^[67]/.test(a)) return 'P2SH'
  if (a.startsWith('E')) return 'EXCHANGE'
  if (a.startsWith('xpub')) return 'XPUB'
  return 'TRANSPARENT'
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">ACCOUNT · /address + /utxo</div>
        <h1 class="page-title">Address</h1>
      </div>
      <div class="head-live">
        <span class="pill neon mono">{{ addrKind }}</span>
        <span v-if="info && info.unconfirmedTxs > 0" class="pill warn mono">{{ info.unconfirmedTxs }} PENDING · {{ Number(info.unconfirmedBalance) > 0 ? '+' : '' }}{{ formatSats(info.unconfirmedBalance, { decimals: 2 }) }} PIV</span>
        <button v-if="isMock" class="gbtn" :class="{ on: demo503 }" @click="toggle503">{{ demo503 ? 'DISABLE' : 'DEMO' }} 503</button>
      </div>
    </div>

    <HudPanel>
      <div class="addr-row">
        <span class="mono addr-val">{{ addr }}</span>
      </div>
    </HudPanel>

    <div v-if="reindexing" class="banner warn" style="margin-top: var(--space-4)">
      <span class="dot warn"></span> ADDRESS INDEX REINDEXING — explorer is catching up. Retry shortly.
      This account is never rendered as empty during a rebuild.
    </div>
    <div v-else-if="err" class="banner bad" style="margin-top: var(--space-4)">{{ err }}</div>

    <HudPanel v-else-if="loading" title="ACCOUNT TELEMETRY" id="/address + /utxo" style="margin-top: var(--space-4)">
      <div class="loading">loading account telemetry…</div>
    </HudPanel>

    <template v-else-if="info">
      <div class="statgrid cols-4" style="margin-top: var(--space-4)">
        <Stat k="BALANCE" accent><template #v>{{ formatSats(info.balance, { decimals: 2 }) }}</template><template #s>PIV (incl. immature)</template></Stat>
        <Stat k="TOTAL RECEIVED" glow><template #v>{{ formatSats(info.totalReceived, { decimals: 2 }) }}</template><template #s>PIV lifetime in</template></Stat>
        <Stat k="TOTAL SENT"><template #v>{{ formatSats(info.totalSent, { decimals: 2 }) }}</template><template #s>PIV lifetime out</template></Stat>
        <Stat k="UNSPENT"><template #v>{{ formatSats(utxoTotal, { decimals: 2 }) }}</template><template #s>{{ utxos.length }} UTXOs spendable</template></Stat>
      </div>

      <div class="split s-21" style="margin-top: var(--space-4)">
        <HudPanel title="BALANCE OVER TIME" id="current holdings · by UTXO creation height" hero>
          <EChart :option="balanceOption" height="240px" aria-label="Account balance accumulation by UTXO creation height" />
        </HudPanel>
        <HudPanel title="RECEIVED / SENT" id="totalReceived vs totalSent">
          <EChart :option="flowOption" height="160px" aria-label="Received versus sent split" />
          <div class="flow-cap">
            <span class="mono"><span class="fc-dot" style="background:var(--cyan)"></span>received {{ formatSats(info.totalReceived, { decimals: 2 }) }}</span>
            <span class="mono"><span class="fc-dot" style="background:var(--rose)"></span>sent {{ formatSats(info.totalSent, { decimals: 2 }) }}</span>
          </div>
        </HudPanel>
      </div>

      <h2 class="section-title">UTXO set — fragmentation &amp; coin age</h2>
      <div class="split s-2">
        <HudPanel title="UTXO VALUE HISTOGRAM" id="/utxo · PIV buckets">
          <EChart :option="histOption" height="220px" aria-label="UTXO value histogram by PIV bucket" />
        </HudPanel>
        <HudPanel title="COIN-AGE SCATTER" id="value × confirmations · staking view">
          <EChart :option="scatterOption" height="220px" aria-label="Coin-age scatter of UTXO value versus confirmations" />
        </HudPanel>
      </div>

      <h2 class="section-title">Transactions (page {{ info.page }}/{{ info.totalPages }})</h2>
      <HudPanel title="LEDGER" :id="`${info.txs} total`">
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th>Txid</th><th class="num">Height</th><th>Age</th><th class="num">Amount (PIV)</th><th class="num">Conf.</th></tr></thead>
            <tbody>
              <tr v-for="t in info.transactions" :key="t.txid">
                <td><RouterLink :to="`/tx/${t.txid}`">{{ truncateHash(t.txid, 10, 8) }}</RouterLink></td>
                <td class="num dim"><span v-if="isUnconfirmedHeight(t.blockHeight)" class="pill warn mono">UNCONFIRMED</span><span v-else>{{ formatCount(t.blockHeight) }}</span></td>
                <td class="dim">{{ timeAgo(t.blockTime) }}</td>
                <td class="num strong" :style="{ color: (txDeltas[t.txid] || {}).color }">{{ (txDeltas[t.txid] || {}).str }}</td>
                <td class="num dim">{{ formatCount(t.confirmations) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>

      <h2 class="section-title">Unspent outputs ({{ utxos.length }})</h2>
      <HudPanel title="UTXOS" id="/utxo · newest first">
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th>Outpoint</th><th class="num">Height</th><th class="num">Value (PIV)</th><th class="num">Conf.</th><th>Source</th></tr></thead>
            <tbody>
              <tr v-for="(u, i) in utxos" :key="i">
                <td class="dim">{{ truncateHash(u.txid, 8, 4) }}:{{ u.vout }}</td>
                <td class="num dim">{{ formatCount(u.height) }}</td>
                <td class="num strong">{{ formatSats(u.value, { decimals: 4 }) }}</td>
                <td class="num dim">{{ formatCount(u.confirmations) }}</td>
                <td><span class="pill" :class="u.coinstake ? 'neon' : (u.coinbase ? 'warn' : 'cyan')">{{ u.coinstake ? 'stake' : (u.coinbase ? 'reward' : 'received') }}</span></td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.addr-row { display: flex; align-items: center; gap: 12px; }
.addr-val { font-size: 13px; color: var(--text-muted); word-break: break-all; }
.flow-cap { display: flex; justify-content: center; gap: 18px; margin-top: 8px; font-size: 11px; color: var(--text-muted); }
.fc-dot { display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 5px; box-shadow: 0 0 5px currentColor; }
/* Flow with the PAGE, not a hard-to-grab nested box; keep horizontal scroll for narrow screens. */
.scroll { overflow-x: auto; }
</style>
