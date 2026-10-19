<script setup>
/* =====================================================================
   GOVERNANCE HISTORY: cycle table ranked by margin over the funding line,
   a swing strip, and a per-proposal view (net votes over time against the
   threshold, signed-time vote batches, change log).
   PASS RULE: net (Yeas - Nays) must be STRICTLY greater than
   floor(enabled / 10) (Core budgetproposal.cpp:211).
   UNITS: governance amounts are PIV f64 numbers (formatPiv, never /1e8).
   Copy states numbers and chain facts only; no adjectives about voters.
   ===================================================================== */
import { ref, computed, onMounted, watch } from 'vue'
import { getGovProposals, getGovHistory, getGovBatches, getGovChanges, getGovSwings, nextSuperblock } from '../api/client.js'
import { useChainStore } from '../store.js'
import { formatPiv } from '../lib/money.js'
import { formatCount, formatDateTime, timeAgo } from '../lib/format.js'
import { baseOption, palette, hexA, valAxis, areaFill, echarts } from '../lib/chart.js'
import EChart from './EChart.vue'
import HudPanel from './HudPanel.vue'
import Stat from './Stat.vue'

const chain = useChainStore()
const meta = ref(null)
const enabled = ref(0)
const downLine = ref(0)
const rows = ref([])
const swings = ref([])
const error = ref(null)
const selected = ref(null)
const history = ref([])
const batches = ref([])
const changes = ref([])
const page = ref(0)
const PAGE = 25

const threshold = computed(() => Math.floor(enabled.value / 10))
const ranked = computed(() => [...rows.value].sort((a, b) => b.margin - a.margin))
const sb = computed(() => nextSuperblock(chain.height))
const sel = computed(() => rows.value.find((r) => r.hash === selected.value) || null)
const byHash = computed(() => Object.fromEntries(rows.value.map((r) => [r.hash, r])))

const REASON = {
  passing: { label: 'passing', cls: 'ok' },
  below_threshold: { label: 'below line', cls: 'warn' },
  out_of_cycle: { label: 'outside next cycle', cls: 'warn' },
  not_established: { label: 'not established', cls: 'warn' },
  invalid: { label: 'invalid', cls: 'bad' },
}
const VOTE = { 0: 'abstain', 1: 'yes', 2: 'no' }

onMounted(async () => {
  try {
    const [p, s] = await Promise.all([getGovProposals(), getGovSwings()])
    meta.value = p.meta
    enabled.value = p.enabled
    downLine.value = p.downvoteInvalidLine
    rows.value = p.rows
    swings.value = s.rows
    const firstWithHistory = ranked.value.find((r) => r.spark.length > 1) || ranked.value[0]
    if (firstWithHistory) selected.value = firstWithHistory.hash
  } catch (e) {
    error.value = e.message || 'Failed to load governance history'
  }
})

watch(selected, async (h) => {
  history.value = []; batches.value = []; changes.value = []; page.value = 0
  if (!h) return
  const [hi, ba, ch] = await Promise.all([getGovHistory(h), getGovBatches(h), getGovChanges(h)])
  history.value = hi.points; batches.value = ba.batches; changes.value = ch.rows
})

/* Inline SVG sparkline: one polyline per row, scaled to its own range, with the
   funding line drawn when it falls inside that range. */
function spark(r) {
  const pts = r.spark.length > 1 ? r.spark : [r.net, r.net]
  const lo = Math.min(...pts, r.threshold), hi = Math.max(...pts, r.threshold)
  const span = hi - lo || 1
  const y = (v) => 18 - ((v - lo) / span) * 16
  const step = 96 / (pts.length - 1)
  return {
    line: pts.map((v, i) => `${(i * step).toFixed(1)},${y(v).toFixed(1)}`).join(' '),
    th: y(r.threshold).toFixed(1),
  }
}

const p = palette()
const histOption = computed(() => {
  const pts = history.value
  const base = baseOption(p)
  return {
    ...base,
    grid: { left: 52, right: 24, top: 18, bottom: 28, containLabel: true },
    tooltip: { ...base.tooltip, trigger: 'axis',
      formatter: (a) => `${formatDateTime(a[0].value[0] / 1000)}<br/>net ${formatCount(a[0].value[1])}` },
    xAxis: { type: 'time', axisLabel: { color: p.axis, fontSize: 10, fontFamily: 'monospace' },
      axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } } },
    yAxis: valAxis(p, { axisLabel: { formatter: (v) => formatCount(v) } }),
    series: [{
      name: 'net votes', type: 'line', step: 'end', showSymbol: false,
      data: pts.map((x) => [x.ts * 1000, x.net]),
      lineStyle: { color: p.cyan, width: 2.2 }, areaStyle: { color: areaFill(echarts, p.cyan, 0.22, 0.01) },
      markLine: { symbol: 'none', silent: true, data: [
        { yAxis: threshold.value, lineStyle: { color: p.amber, type: 'dashed' },
          label: { formatter: `funding line > ${formatCount(threshold.value)}`, color: p.amber, fontSize: 9, fontFamily: 'monospace', position: 'insideEndTop' } },
        { yAxis: -downLine.value, lineStyle: { color: hexA(p.rose, 0.8), type: 'dotted' },
          label: { formatter: `invalid below -${formatCount(downLine.value)}`, color: p.rose, fontSize: 9, fontFamily: 'monospace', position: 'insideEndBottom' } },
      ] },
    }],
  }
})

const batchOption = computed(() => {
  const b = batches.value
  const base = baseOption(p)
  const bar = (name, key, color) => ({
    name, type: 'bar', stack: 'b', barMaxWidth: 14, color,
    data: b.map((x) => [x.start * 1000, x[key]]),
  })
  return {
    ...base,
    legend: { data: ['yes', 'no', 'abstain'], top: 0, right: 4, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 10, itemHeight: 8 },
    grid: { left: 52, right: 24, top: 26, bottom: 28, containLabel: true },
    tooltip: { ...base.tooltip, trigger: 'item', formatter: (it) => {
      const x = b[it.dataIndex]
      const secs = x.end - x.start
      return `${formatDateTime(x.start)}<br/>${formatCount(x.count)} votes within ${formatCount(secs)} s<br/>yes ${x.yes} · no ${x.no} · abstain ${x.abstain}`
    } },
    xAxis: { type: 'time', axisLabel: { color: p.axis, fontSize: 10, fontFamily: 'monospace' },
      axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } } },
    yAxis: valAxis(p, { axisLabel: { formatter: (v) => formatCount(v) } }),
    series: [bar('yes', 'yes', p.green), bar('no', 'no', p.rose), bar('abstain', 'abstain', p.axis)],
  }
})

const pagedChanges = computed(() => changes.value.slice(page.value * PAGE, (page.value + 1) * PAGE))
const pages = computed(() => Math.max(1, Math.ceil(changes.value.length / PAGE)))

function swingText(s) {
  const top = s.batches[0]
  const net = `${s.net_delta > 0 ? '+' : ''}${formatCount(s.net_delta)}`
  const burst = top ? `, ${formatCount(top.count)} ${top.no >= top.yes ? 'NO' : 'YES'} votes within ${formatCount(top.end - top.start)} s on ${formatDateTime(top.start)}` : ''
  return `${s.name} net ${net} in ${s.window}${burst}`
}
</script>

<template>
  <div class="govh">
    <p v-if="error" class="note mono bad">{{ error }}</p>
    <p v-if="meta && meta.prototype" class="proto mono">
      <span class="pill warn">PROTOTYPE DATA</span>
      Live node responses captured {{ formatDateTime(meta.time) }}. Real history starts when the poller ships.
    </p>

    <div v-if="meta" class="statrow">
      <Stat k="FUNDING LINE" :v="`> ${formatCount(threshold)}`" :s="`net votes · floor(${formatCount(enabled)} enabled / 10)`" accent />
      <Stat k="PASSING" :v="`${rows.filter((r) => r.passing).length} / ${rows.length}`" s="live proposals over the line" />
      <Stat k="FUNDED NEXT" :v="String(rows.filter((r) => r.funded).length)" s="in Core's budget projection" />
      <Stat k="NEXT SUPERBLOCK" :v="`#${formatCount(sb)}`" :s="chain.height ? `${formatCount(sb - chain.height)} blocks away` : ''" />
    </div>

    <HudPanel v-if="swings.length" title="SWINGS" id="/gov/swings · net change ≥ 200 within 72h">
      <ul class="swings mono">
        <li v-for="s in swings" :key="s.hash">
          <button type="button" class="linkish" @click="selected = s.hash">{{ swingText(s) }}</button>
        </li>
      </ul>
    </HudPanel>

    <h2 class="section-title">This cycle · ranked by margin over the funding line</h2>
    <HudPanel title="CYCLE" id="/gov/proposals · margin = net - floor(enabled/10)">
      <div class="tablewrap">
        <table class="dtable">
          <thead>
            <tr>
              <th>proposal</th><th>net trend</th><th class="r">net</th><th class="r">margin</th>
              <th>status</th><th class="r">monthly PIV</th><th class="r">payments</th><th class="r">last change</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in ranked" :key="r.hash" :class="{ on: r.hash === selected }" @click="selected = r.hash">
              <td><button type="button" class="linkish name" @click.stop="selected = r.hash">{{ r.name }}</button></td>
              <td>
                <svg class="spark" viewBox="0 0 96 20" preserveAspectRatio="none" aria-hidden="true">
                  <line x1="0" x2="96" :y1="spark(r).th" :y2="spark(r).th" class="spark-th" />
                  <polyline :points="spark(r).line" class="spark-line" :class="{ neg: r.margin <= 0 }" />
                </svg>
              </td>
              <td class="r">{{ formatCount(r.net) }}</td>
              <td class="r" :class="r.margin > 0 ? 'good' : 'badc'">{{ r.margin > 0 ? '+' : '' }}{{ formatCount(r.margin) }}</td>
              <td>
                <span class="pill" :class="REASON[r.reason].cls">{{ REASON[r.reason].label }}</span>
                <span v-if="r.funded" class="pill cyan">funded</span>
                <!-- No remaining payments means nothing left to fund, not a cap miss. -->
                <span v-else-if="r.payments_total && r.payments_done >= r.payments_total" class="pill">paid out</span>
                <span v-else-if="r.passing" class="pill neon">over line, not funded</span>
              </td>
              <td class="r">{{ formatPiv(r.monthly, { decimals: 0 }) }}</td>
              <td class="r">{{ r.payments_done }} / {{ r.payments_total }}</td>
              <td class="r dim">{{ r.last_change_ts ? timeAgo(r.last_change_ts) : 'no history yet' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </HudPanel>

    <template v-if="sel">
      <h2 class="section-title">{{ sel.name }} · history</h2>
      <div class="selmeta mono dim">
        hash {{ sel.hash.slice(0, 16) }}… · payee {{ sel.payee }} · blocks {{ formatCount(sel.start) }} to {{ formatCount(sel.end) }}
        · <a :href="sel.url" target="_blank" rel="noopener noreferrer nofollow">{{ sel.url }}</a>
      </div>

      <div class="grid2">
        <HudPanel title="NET VOTES OVER TIME" id="/gov/proposal/{hash}/history">
          <EChart v-if="history.length > 1" :option="histOption" height="240px" :aria-label="`${sel.name} net votes over time against the funding line`" />
          <p v-else class="note mono dim">No history recorded yet for this proposal. Current net {{ formatCount(sel.net) }}.</p>
        </HudPanel>
        <HudPanel title="VOTE BATCHES" id="/gov/proposal/{hash}/batches · votes ≤ 120 s apart (signed time)">
          <EChart v-if="batches.length" :option="batchOption" height="240px" :aria-label="`${sel.name} vote batches by signed time`" />
          <p v-else class="note mono dim">No votes recorded yet.</p>
        </HudPanel>
      </div>

      <HudPanel title="CHANGE LOG" :id="`/gov/proposal/{hash}/changes · ${formatCount(changes.length)} rows`">
        <div v-if="changes.length" class="tablewrap">
          <table class="dtable">
            <thead><tr><th>signed time</th><th>masternode</th><th>change</th><th>vote</th><th>valid</th></tr></thead>
            <tbody>
              <tr v-for="(c, i) in pagedChanges" :key="i">
                <td>{{ formatDateTime(c.n_time) }}</td>
                <td>{{ c.mn }}</td>
                <td>{{ c.kind }}</td>
                <td>
                  <span v-if="c.from != null">{{ VOTE[c.from] }} → </span>
                  <span :class="c.to === 1 ? 'good' : c.to === 2 ? 'badc' : 'dim'">{{ VOTE[c.to] }}</span>
                </td>
                <td>{{ c.f_valid ? 'yes' : 'no' }}</td>
              </tr>
            </tbody>
          </table>
          <div class="pager mono">
            <button type="button" :disabled="page === 0" @click="page--">prev</button>
            <span>{{ page + 1 }} / {{ pages }}</span>
            <button type="button" :disabled="page + 1 >= pages" @click="page++">next</button>
          </div>
        </div>
        <p v-else class="note mono dim">No changes recorded yet.</p>
      </HudPanel>

      <HudPanel title="PAYOUTS" id="/gov/payouts">
        <p class="note mono dim">
          Payout attribution starts with superblock #{{ formatCount(sb) }}: each paid output is matched to this proposal's payee
          and monthly amount, with a "follow" link into the next transaction.
        </p>
      </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.govh { display: flex; flex-direction: column; gap: var(--space-5); }
.proto { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--text-muted); margin: 0; }
.statrow { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: var(--space-4); }
.swings { list-style: none; margin: 0; padding: var(--space-4) var(--space-5); display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; }
.linkish { background: none; border: 0; padding: 0; color: var(--text); font: inherit; cursor: pointer; text-align: left; }
.linkish:hover, .linkish:focus-visible { color: var(--neon); }
.linkish.name { font-weight: 600; }
.tablewrap { overflow-x: auto; }
.dtable tr { cursor: pointer; }
.dtable tr.on td { background: rgba(196,107,255,0.1); color: var(--text); }
.dtable .r { text-align: right; }
.good { color: var(--success); }
.badc { color: var(--hot); }
.dim { color: var(--text-dim); }
.spark { width: 96px; height: 20px; display: block; }
.spark-line { fill: none; stroke: var(--cyan); stroke-width: 1.4; }
.spark-line.neg { stroke: var(--hot); }
.spark-th { stroke: var(--warn); stroke-width: 0.8; stroke-dasharray: 2 2; opacity: 0.7; }
.selmeta { font-size: 11.5px; margin-top: calc(-1 * var(--space-3)); word-break: break-all; }
.selmeta a { color: var(--text-muted); }
.grid2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: var(--space-5); }
.note { padding: var(--space-4) var(--space-5); margin: 0; font-size: 12px; }
.pager { display: flex; gap: 12px; align-items: center; justify-content: flex-end; padding: 10px 12px; font-size: 12px; }
.pager button { background: rgba(196,107,255,0.1); border: 1px solid var(--glass-edge); color: var(--text-muted); border-radius: 6px; padding: 3px 10px; cursor: pointer; font: inherit; }
.pager button:disabled { opacity: 0.4; cursor: default; }
@media (max-width: 700px) { .grid2 { grid-template-columns: 1fr; } }
</style>
