<script setup>
/* =====================================================================
   GOVERNANCE — /budgetinfo (all proposals) + /budgetprojection (the
   funded subset that will actually pay). Superblock countdown, treasury
   allocation bar, and a proposal table with diverging Yeas/Nays bars.
   FUNDING GATE: (Yeas - Nays) must exceed 10% of /mncount.total.
   UNITS: budget amounts are PIV f64 NUMBERS (formatPiv, never /1e8).
   ===================================================================== */
import { ref, onMounted, computed, nextTick } from 'vue'
import { useChainStore } from '../store.js'
import { getBudgetInfo, getBudgetProjection, getMnCount, proposalPasses, nextSuperblock, monthlyBudgetCap } from '../api/client.js'
import { formatPiv } from '../lib/money.js'
import { formatCount, percent } from '../lib/format.js'
import { baseOption, palette, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'
import BudgetSimulator from '../components/BudgetSimulator.vue'

const chain = useChainStore()
const proposals = ref([])
const projection = ref([])
const mncount = ref(null)
const loading = ref(true)
const error = ref(null)

/* Overview (read-only) vs Simulator (what-if sandbox). Tablist with roving
   focus + arrow keys, mirroring the nav-rail tab semantics. */
const mode = ref('overview')
const onTabKey = (e) => {
  if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
    e.preventDefault()
    mode.value = mode.value === 'overview' ? 'sim' : 'overview'
    // APG roving tabindex: arrow keys move selection AND focus to the new tab.
    nextTick(() => document.getElementById(`tab-${mode.value}`)?.focus())
  }
}

onMounted(async () => {
  try {
    proposals.value = await getBudgetInfo()
    projection.value = await getBudgetProjection()
    mncount.value = await getMnCount()
  } catch (e) {
    error.value = e.message || 'Failed to load governance telemetry'
  } finally {
    loading.value = false
  }
})

const threshold = computed(() => mncount.value ? 0.1 * mncount.value.total : 211)
const passes = (p) => (p.Yeas - p.Nays) > threshold.value

/* superblock countdown from the chain tip (~60s PoS spacing). Live derives the
   next superblock from the real tip so it advances past each cycle. */
const sbHeight = computed(() => nextSuperblock(chain.height))
const blocksLeft = computed(() => Math.max(0, sbHeight.value - (chain.height || 0)))
const daysLeft = computed(() => (blocksLeft.value * 60) / 86400)

const allotted = computed(() =>
  projection.value.length ? projection.value[projection.value.length - 1].TotalBudgetAllotted : 0)
// Treasury value in fiat: PIV allotted × PIVX/USD (0 when price unavailable).
const allottedUsd = computed(() =>
  chain.price && chain.price.usd > 0 ? allotted.value * chain.price.usd : 0)
// Per-cycle treasury cap (PIV) — Core funds greedily UP TO this and defers the
// rest, so allotted should never exceed it. Surface it so the max is visible.
const cap = computed(() => monthlyBudgetCap())
const capPct = computed(() => (cap.value > 0 ? (allotted.value / cap.value) * 100 : 0))
const overCap = computed(() => allotted.value > cap.value + 1)
const demand = computed(() => proposals.value.reduce((s, p) => s + p.MonthlyPayment, 0))

const PAL = ['#c46bff', '#46e6d0', '#ffcf5c', '#9d4ef0', '#ff6f9c', '#7ad97a']

/* ---------- treasury allocation: 100%-stacked horizontal bar ---------- */
const allocOption = computed(() => {
  const p = palette()
  const rows = projection.value
  if (!rows.length) return baseOption(p)
  return {
    backgroundColor: 'transparent',
    grid: { left: 8, right: 8, top: 8, bottom: 8 },
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (s) => `${s.seriesName}<br/>${formatPiv(s.value, { decimals: 0 })} PIV` },
    xAxis: { type: 'value', show: false },
    yAxis: { type: 'category', data: ['ALLOTTED'], show: false },
    series: rows.map((r, i) => ({
      name: r.Name, type: 'bar', stack: 'a', barWidth: 46,
      data: [r.Allotted],
      label: { show: r.Allotted / allotted.value > 0.12, position: 'inside', color: '#0b0716', fontFamily: 'monospace', fontWeight: 700, fontSize: 10, formatter: () => `${(r.Allotted / 1000).toFixed(0)}k` },
      itemStyle: { color: PAL[i % PAL.length], borderColor: 'rgba(7,4,13,0.5)', borderWidth: 1 },
    })),
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">PIVX TREASURY · /budgetinfo + /budgetprojection</div>
        <h1 class="page-title">Governance</h1>
      </div>
      <div class="head-live">
        <span class="pill neon mono"><span class="dot neon"></span>SUPERBLOCK #{{ formatCount(sbHeight) }}</span>
      </div>
    </div>

    <HudPanel v-if="loading" title="TREASURY TELEMETRY" id="/budgetinfo + /budgetprojection">
      <div class="loading">loading governance telemetry…</div>
    </HudPanel>
    <div v-else-if="error" class="banner bad" style="margin-top: var(--space-4)">{{ error }}</div>
    <template v-else>
    <!-- MODE TABS -->
    <div class="modetabs" role="tablist" aria-label="Governance view">
      <button
        id="tab-overview" role="tab" type="button" class="modetab"
        :class="{ on: mode === 'overview' }" :aria-selected="mode === 'overview'"
        :tabindex="mode === 'overview' ? 0 : -1"
        @click="mode = 'overview'" @keydown="onTabKey"
      >OVERVIEW</button>
      <button
        id="tab-sim" role="tab" type="button" class="modetab"
        :class="{ on: mode === 'sim' }" :aria-selected="mode === 'sim'"
        :tabindex="mode === 'sim' ? 0 : -1"
        @click="mode = 'sim'" @keydown="onTabKey"
      >SIMULATOR</button>
    </div>

    <!-- ================= SIMULATOR ================= -->
    <div v-if="mode === 'sim'" role="tabpanel" aria-labelledby="tab-sim">
      <BudgetSimulator :candidates="proposals" :actual-funded="projection" :threshold="threshold" />
    </div>

    <!-- ================= OVERVIEW (read-only) ================= -->
    <div v-else role="tabpanel" aria-labelledby="tab-overview">
    <!-- HEADLINE -->
    <div class="statgrid cols-4">
      <Stat k="SUPERBLOCK IN" accent live>
        <template #v>{{ formatCount(blocksLeft) }}<span class="unit">blk</span></template>
        <template #s>≈ {{ daysLeft.toFixed(1) }} days at ~60s/block</template>
      </Stat>
      <Stat k="ALLOTTED THIS CYCLE" glow>
        <template #v>{{ formatPiv(allotted, { decimals: 0 }) }}</template>
        <template #s><span v-if="overCap" style="color:var(--warn);font-weight:700">⚠ OVER CAP · </span>{{ capPct.toFixed(0) }}% of the {{ formatPiv(cap, { decimals: 0 }) }} PIV cap{{ allottedUsd ? ' · ≈ $' + Math.round(allottedUsd).toLocaleString('en-US') : '' }}</template>
      </Stat>
      <Stat k="PROPOSALS">
        <template #v>{{ projection.length }}<span class="unit">/ {{ proposals.length }}</span></template>
        <template #s>funded / submitted</template>
      </Stat>
      <Stat k="TREASURY DEMAND">
        <template #v>{{ formatPiv(demand, { decimals: 0 }) }}</template>
        <template #s>PIV/month requested (all proposals)</template>
      </Stat>
    </div>

    <!-- ALLOCATION -->
    <h2 class="section-title">Treasury allocation — funded proposals this cycle</h2>
    <HudPanel title="BUDGET ALLOCATION" id="/budgetprojection · Allotted share" hero>
      <template #head><span class="pill cyan mono">Σ {{ formatPiv(allotted, { decimals: 0 }) }} PIV</span></template>
      <EChart v-if="projection.length" :option="allocOption" height="74px" aria-label="Treasury allocation share across funded proposals" />
      <div class="alloc-legend" v-if="projection.length">
        <div v-for="(r, i) in projection" :key="r.Hash" class="al">
          <span class="al-dot" :style="{ background: PAL[i % PAL.length] }"></span>
          <RouterLink :to="`/proposal/${encodeURIComponent(r.Name)}`" class="al-name">{{ r.Name }}</RouterLink>
          <span class="al-val mono">{{ formatPiv(r.Allotted, { decimals: 0 }) }} PIV</span>
        </div>
      </div>
    </HudPanel>

    <!-- PROPOSAL TABLE -->
    <h2 class="section-title">All proposals ({{ proposals.length }})</h2>
    <HudPanel title="PROPOSALS" id="/budgetinfo · approval = (Yeas−Nays) &gt; 10% MN">
      <div class="scroll">
        <table class="dtable">
          <thead>
            <tr><th>Proposal</th><th>Approval (Yeas / Nays)</th><th class="num">Monthly</th><th class="num">Payments left</th><th>Status</th></tr>
          </thead>
          <tbody>
            <tr v-for="p in proposals" :key="p.Hash">
              <td><RouterLink :to="`/proposal/${encodeURIComponent(p.Name)}`">{{ p.Name }}</RouterLink></td>
              <td>
                <div class="vote">
                  <div class="vbar">
                    <i class="yea" :style="{ width: (p.Yeas / (p.Yeas + p.Nays || 1) * 100) + '%' }"></i>
                    <i class="nay" :style="{ width: (p.Nays / (p.Yeas + p.Nays || 1) * 100) + '%' }"></i>
                  </div>
                  <span class="mono dim vnums">{{ formatCount(p.Yeas) }} / {{ formatCount(p.Nays) }} · {{ percent(p.Ratio * 100, 1) }}</span>
                </div>
              </td>
              <td class="num strong">{{ formatPiv(p.MonthlyPayment, { decimals: 0 }) }}</td>
              <td class="num dim">{{ p.RemainingPaymentCount }} / {{ p.TotalPaymentCount }}</td>
              <td><span class="pill" :class="passes(p) ? 'ok' : 'bad'">{{ passes(p) ? 'PASSING' : 'FAILING' }}</span></td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="note mono dim">
        Funding gate: net approval (Yeas − Nays) must exceed 10% of the masternode count
        ({{ mncount ? formatCount(Math.ceil(threshold)) : '—' }} votes). budgetinfo.Allotted is 0 —
        the authoritative payout list is /budgetprojection.
      </p>
    </HudPanel>
    </div>
    </template>
  </div>
</template>

<style scoped>
.modetabs { display: inline-flex; gap: 4px; padding: 4px; margin-bottom: var(--space-5);
  background: rgba(10,6,20,0.5); border: 1px solid var(--hud-line); border-radius: var(--radius-md); }
.modetab { font-family: var(--font-mono); font-size: 11px; font-weight: 600; letter-spacing: 0.16em;
  padding: 7px 18px; border-radius: var(--radius-sm); cursor: pointer;
  background: transparent; border: 1px solid transparent; color: var(--text-dim); transition: all .15s; }
.modetab:hover { color: var(--text-muted); }
.modetab.on { color: var(--neon); background: rgba(196,107,255,0.12); border-color: var(--glass-edge); text-shadow: var(--glow-xs); }

.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.unit { font-size: 0.5em; color: var(--text-dim); margin-left: 4px; }
.alloc-legend { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 8px; margin-top: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
.al { display: flex; align-items: center; gap: 8px; font-size: 11.5px; }
.al-dot { width: 9px; height: 9px; border-radius: 2px; box-shadow: 0 0 6px currentColor; }
.al-name { color: var(--neon); font-family: var(--font-mono); }
.al-val { margin-left: auto; color: var(--text-muted); }
/* Let the proposals table flow at full height and scroll with the PAGE, not a
   nested 480px box (which is hard to grab). Keep horizontal scroll for narrow screens. */
.scroll { overflow-x: auto; }
.vote { display: flex; flex-direction: column; gap: 4px; min-width: 200px; }
.vbar { display: flex; height: 7px; border-radius: 4px; overflow: hidden; background: rgba(150,90,220,0.12); }
.vbar .yea { background: var(--success); box-shadow: 0 0 6px rgba(92,203,111,0.5); }
.vbar .nay { background: var(--hot); }
.vnums { font-size: 10.5px; }
.note { margin: var(--space-4) 0 0; font-size: 11px; padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
</style>
