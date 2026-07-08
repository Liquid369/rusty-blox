<script setup>
/* =====================================================================
   PROPOSAL DETAIL — one /budgetinfo proposal + its /budgetvotes campaign.
   Approval gauge (Ratio), YES/NO/ABSTAIN donut (valid votes only), the
   cumulative net-approval timeline crossing the funding threshold, and a
   pass/fail verdict vs 10% of /mncount.total.
   UNITS: TotalPayment/MonthlyPayment are PIV f64 NUMBERS (formatPiv).
   ===================================================================== */
import { ref, onMounted, computed, watch } from 'vue'
import { getBudgetInfo, getBudgetVotes, getMnCount } from '../api/client.js'
import { formatPiv } from '../lib/money.js'
import { formatCount, percent, formatDateTime } from '../lib/format.js'
import { echarts, baseOption, valAxis, palette, areaFill, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ name: { type: String, required: true } })
const proposal = ref(null)
const votes = ref([])
const mncount = ref(null)
const notFound = ref(false)
const err = ref(null)
const loading = ref(true)

async function load() {
  proposal.value = null; votes.value = []; notFound.value = false; err.value = null
  loading.value = true
  try {
    const [all, mc] = await Promise.all([getBudgetInfo(), getMnCount()])
    mncount.value = mc
    const p = all.find((x) => x.Name === props.name)
    if (!p) { notFound.value = true; return }
    proposal.value = p
    votes.value = await getBudgetVotes(p.Name)
  } catch (e) {
    err.value = e.message || 'failed to load proposal'
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(() => props.name, load)

const threshold = computed(() => mncount.value ? 0.1 * mncount.value.total : 211)
const netApproval = computed(() => proposal.value ? proposal.value.Yeas - proposal.value.Nays : 0)
const passes = computed(() => netApproval.value > threshold.value)

// valid-only tallies (array length != Yeas — strip invalid/superseded)
const valid = computed(() => votes.value.filter((v) => v.fValid))
const tally = computed(() => {
  const t = { YES: 0, NO: 0, ABSTAIN: 0 }
  for (const v of valid.value) t[v.Vote] = (t[v.Vote] || 0) + 1
  return t
})

/* ---------- approval gauge (Ratio) ---------- */
const gaugeOption = computed(() => {
  const p = palette()
  const pct = proposal.value ? proposal.value.Ratio * 100 : 0
  const col = passes.value ? p.green : p.hot
  return {
    backgroundColor: 'transparent',
    series: [{
      type: 'gauge', startAngle: 220, endAngle: -40, radius: '96%', center: ['50%', '58%'],
      min: 0, max: 100, splitNumber: 4,
      progress: { show: true, width: 10, roundCap: true, itemStyle: { color: col, shadowColor: hexA(col, 0.7), shadowBlur: 14 } },
      axisLine: { lineStyle: { width: 10, color: [[1, hexA(p.neon, 0.1)]] } },
      axisTick: { show: false },
      splitLine: { length: 8, lineStyle: { color: hexA(p.neon, 0.3), width: 1 } },
      axisLabel: { distance: 14, color: p.axis, fontSize: 9, fontFamily: 'monospace' },
      pointer: { show: false }, anchor: { show: false }, title: { show: false },
      detail: { valueAnimation: true, offsetCenter: [0, '6%'], formatter: (v) => `${v.toFixed(1)}%`, color: col, fontSize: 26, fontFamily: 'monospace', fontWeight: 700 },
      data: [{ value: pct }],
    }],
  }
})

/* ---------- YES / NO / ABSTAIN donut (valid only) ---------- */
const donutOption = computed(() => {
  const p = palette()
  const t = tally.value
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${d.name}<br/>${formatCount(d.value)} votes · ${d.percent}%` },
    series: [{
      type: 'pie', radius: ['56%', '82%'], center: ['50%', '50%'],
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'YES', value: t.YES, itemStyle: { color: p.green } },
        { name: 'NO', value: t.NO, itemStyle: { color: p.hot } },
        { name: 'ABSTAIN', value: t.ABSTAIN, itemStyle: { color: hexA(p.axis, 0.6) } },
      ],
    }],
  }
})

/* ---------- cumulative net-approval timeline ---------- */
const timelineOption = computed(() => {
  const p = palette()
  const rows = valid.value
  if (!rows.length) return baseOption(p)
  let cum = 0
  const pts = rows.map((v) => {
    cum += v.Vote === 'YES' ? 1 : (v.Vote === 'NO' ? -1 : 0)
    return [v.nTime * 1000, cum]
  })
  return {
    ...baseOption(p),
    grid: { left: 50, right: 16, top: 18, bottom: 26, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${new Date(arr[0].value[0]).toISOString().slice(0, 10)}<br/>net approval ${formatCount(arr[0].value[1])}` },
    xAxis: { type: 'time', axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } }, axisLabel: { color: p.axis, fontSize: 9, fontFamily: 'monospace' }, axisTick: { show: false } },
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: (v) => formatCount(v) } }),
    series: [{
      type: 'line', smooth: true, showSymbol: false, data: pts,
      lineStyle: { color: p.neon, width: 2.4, shadowColor: hexA(p.neon, 0.7), shadowBlur: 12 },
      areaStyle: { color: areaFill(echarts, p.neon, 0.36, 0.01) },
      markLine: {
        silent: true, symbol: 'none',
        data: [{ yAxis: Math.ceil(threshold.value), lineStyle: { color: hexA(p.amber, 0.8), type: 'dashed' }, label: { formatter: 'funding threshold', color: p.amber, fontSize: 9, fontFamily: 'monospace', position: 'insideEndTop' } }],
      },
    }],
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">PROPOSAL · /budgetinfo + /budgetvotes</div>
        <h1 class="page-title">{{ proposal ? proposal.Name : 'Proposal' }}</h1>
      </div>
      <div class="head-live" v-if="proposal">
        <span class="pill" :class="passes ? 'ok' : 'bad'"><span class="dot" :class="passes ? 'live' : ''"></span>{{ passes ? 'PASSING' : 'FAILING' }}</span>
      </div>
    </div>

    <div v-if="notFound" class="banner bad">
      No proposal named “{{ name }}” in the current budget cycle.
      <RouterLink to="/governance">Back to governance ›</RouterLink>
    </div>

    <div v-else-if="err" class="banner bad">{{ err }}</div>

    <div v-else-if="loading" class="loading" style="margin-top: var(--space-4)">loading proposal telemetry…</div>

    <template v-else-if="proposal">
      <!-- HEADLINE -->
      <div class="statgrid cols-4">
        <Stat k="MONTHLY PAYMENT" accent>
          <template #v>{{ formatPiv(proposal.MonthlyPayment, { decimals: 0 }) }}</template>
          <template #s>PIV per superblock</template>
        </Stat>
        <Stat k="TOTAL REQUEST" glow>
          <template #v>{{ formatPiv(proposal.TotalPayment, { decimals: 0 }) }}</template>
          <template #s>PIV over {{ proposal.TotalPaymentCount }} payments</template>
        </Stat>
        <Stat k="PAYMENTS LEFT">
          <template #v>{{ proposal.RemainingPaymentCount }}<span class="unit">/ {{ proposal.TotalPaymentCount }}</span></template>
          <template #s>blocks {{ formatCount(proposal.BlockStart) }}–{{ formatCount(proposal.BlockEnd) }}</template>
        </Stat>
        <Stat k="NET APPROVAL">
          <template #v>{{ formatCount(netApproval) }}</template>
          <template #s>need &gt; {{ formatCount(Math.ceil(threshold)) }} to fund</template>
        </Stat>
      </div>

      <!-- VOTE TALLY -->
      <h2 class="section-title">Vote tally</h2>
      <div class="split s-21">
        <HudPanel title="NET-APPROVAL TIMELINE" id="/budgetvotes · cumulative YES−NO (valid)" hero>
          <template #head><span class="pill cyan mono">{{ formatCount(valid.length) }} valid · {{ formatCount(votes.length) }} total</span></template>
          <EChart v-if="valid.length" :option="timelineOption" height="260px" />
          <div v-else class="loading">no votes recorded for this proposal.</div>
        </HudPanel>
        <div class="stack">
          <HudPanel title="APPROVAL RATIO" id="Yeas / (Yeas+Nays)">
            <EChart :option="gaugeOption" height="150px" />
            <div class="verdict" :class="passes ? 'ok' : 'bad'">
              {{ passes ? 'FUNDED — net approval clears the 10% gate' : 'BELOW THRESHOLD — will not be paid this cycle' }}
            </div>
          </HudPanel>
          <HudPanel title="VOTE BREAKDOWN" id="valid votes only">
            <EChart :option="donutOption" height="130px" />
            <div class="tally-cap">
              <span class="mono"><i class="td" style="background:var(--success)"></i>YES {{ formatCount(tally.YES) }}</span>
              <span class="mono"><i class="td" style="background:var(--hot)"></i>NO {{ formatCount(tally.NO) }}</span>
              <span class="mono"><i class="td" style="background:var(--text-dim)"></i>ABS {{ formatCount(tally.ABSTAIN) }}</span>
            </div>
          </HudPanel>
        </div>
      </div>

      <!-- META -->
      <h2 class="section-title">Proposal record</h2>
      <HudPanel title="DETAIL" :id="proposal.IsEstablished ? 'established' : 'new'">
        <dl class="kv">
          <dt>Name</dt><dd>{{ proposal.Name }}</dd>
          <dt>URL</dt><dd><a :href="proposal.URL" target="_blank" rel="noopener noreferrer">{{ proposal.URL }}</a></dd>
          <dt>Payment address</dt><dd><RouterLink :to="`/address/${proposal.PaymentAddress}`">{{ proposal.PaymentAddress }}</RouterLink></dd>
          <dt>Proposal hash</dt><dd>{{ proposal.Hash }}</dd>
          <dt>Fee hash</dt><dd><RouterLink :to="`/tx/${proposal.FeeHash}`">{{ proposal.FeeHash }}</RouterLink></dd>
          <dt>Approval ratio</dt><dd>{{ percent(proposal.Ratio * 100, 2) }} <span class="dim">(excludes abstains)</span></dd>
          <dt>Yeas / Nays / Abstains</dt><dd>{{ formatCount(proposal.Yeas) }} / {{ formatCount(proposal.Nays) }} / {{ formatCount(proposal.Abstains) }}</dd>
          <dt>Established</dt><dd>{{ proposal.IsEstablished ? 'yes' : 'no' }}</dd>
          <dt>Valid</dt><dd>{{ proposal.IsValid ? 'yes' : 'no' }}</dd>
        </dl>
      </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.unit { font-size: 0.5em; color: var(--text-dim); margin-left: 4px; }
.verdict { margin-top: 10px; font-family: var(--font-mono); font-size: 11px; text-align: center; padding: 8px; border-radius: var(--radius-md); }
.verdict.ok { color: var(--success); background: rgba(92,203,111,0.1); border: 1px solid rgba(92,203,111,0.3); }
.verdict.bad { color: var(--hot); background: rgba(255,84,112,0.1); border: 1px solid rgba(255,84,112,0.3); }
.tally-cap { display: flex; justify-content: center; gap: 14px; margin-top: 8px; font-size: 11px; color: var(--text-muted); }
.tally-cap .td { display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 5px; box-shadow: 0 0 5px currentColor; }
</style>
