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
import { getBudgetInfo, getBudgetProjection, getMnCount, getFinalizedBudgets, getTx, nextSuperblock, monthlyBudgetCap } from '../api/client.js'
import { formatPiv } from '../lib/money.js'
import { formatCount, percent, formatDateTime, truncateHash, esc } from '../lib/format.js'
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
const finalized = ref([])

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
  // Finalized budget (mnfinalbudget show) — best-effort, never blocks the page. The FeeTX is
  // the OP_RETURN finalization-collateral tx; resolve it to its block for the "finalized when".
  try {
    const fb = await getFinalizedBudgets()
    const rows = Object.entries(fb || {}).map(([key, v]) => {
      const m = key.match(/^(.*?)\s*\(([0-9a-fA-F]+)\)\s*$/)
      return {
        name: m ? m[1] : key, hash: m ? m[2] : '',
        feeTx: v.FeeTX, blockStart: v.BlockStart, blockEnd: v.BlockEnd,
        votes: v.VoteCount, status: v.Status, proposals: v.Proposals,
        atHeight: null, atTime: null,
      }
    })
    // resolve every FeeTX concurrently — no need to wait N serial round-trips
    await Promise.all(rows.map(async (r) => {
      if (!r.feeTx) return
      try { const tx = await getTx(r.feeTx); r.atHeight = tx.blockHeight; r.atTime = tx.blockTime } catch { /* still link the tx */ }
    }))
    finalized.value = rows
  } catch { /* no finalized budget available */ }
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
// USD value of a PIV amount at the live price, rounded + grouped ('' when unavailable).
const usd = (piv) =>
  chain.price && chain.price.usd > 0 ? '$' + Math.round(piv * chain.price.usd).toLocaleString('en-US') : ''
// Per-cycle treasury cap (PIV) — Core funds greedily UP TO this and defers the
// rest, so allotted should never exceed it. Surface it so the max is visible.
const cap = computed(() => monthlyBudgetCap())
const capPct = computed(() => (cap.value > 0 ? (allotted.value / cap.value) * 100 : 0))
const overCap = computed(() => allotted.value > cap.value + 1)
const demand = computed(() => proposals.value.reduce((s, p) => s + p.MonthlyPayment, 0))

/* ---------- funded THIS cycle (most recent superblock) ----------
   /budgetprojection is forward-looking (the NEXT payout), so it can't answer
   "what did the last superblock fund". We reproduce Core's allotment for the most
   recent superblock instead: passing proposals whose payment window covers it,
   ranked by net approval, funded greedily up to the monthly cap (skip-and-keep-
   filling, exactly like CBudgetManager::GetBudget). Verified to match the on-chain
   superblock payout (blocks pay one proposal each, contiguously from the superblock
   height) exactly, without a per-block scan. */
const SUPERBLOCK_CYCLE = 43200 // PIVX consensus: nBudgetCycleBlocks
const lastSbHeight = computed(() =>
  Math.floor((chain.height || 0) / SUPERBLOCK_CYCLE) * SUPERBLOCK_CYCLE)
const fundedThisCycle = computed(() => {
  const sb = lastSbHeight.value
  if (!sb || !proposals.value.length) return { rows: [], total: 0 }
  const cand = proposals.value
    .filter((p) => passes(p) && p.BlockStart <= sb && sb <= p.BlockEnd && p.MonthlyPayment > 0)
    .sort((a, b) => b.Yeas - b.Nays - (a.Yeas - a.Nays))
  const rows = []
  let total = 0
  for (const p of cand) {
    if (total + p.MonthlyPayment > cap.value + 0.5) continue // over cap → deferred, not paid
    rows.push(p)
    total += p.MonthlyPayment
  }
  return { rows, total }
})

const PAL = ['#c46bff', '#46e6d0', '#ffcf5c', '#9d4ef0', '#ff6f9c', '#7ad97a']

/* ---------- treasury allocation: 100%-stacked horizontal bar ---------- */
const allocOption = computed(() => {
  const p = palette()
  const rows = projection.value
  if (!rows.length) return baseOption(p)
  return {
    backgroundColor: 'transparent',
    grid: { left: 8, right: 8, top: 8, bottom: 8 },
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (s) => `${esc(s.seriesName)}<br/>${formatPiv(s.value, { decimals: 0 })} PIV` },
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
      <Stat k="ALLOTTED · NEXT SUPERBLOCK" glow>
        <template #v>{{ formatPiv(allotted, { decimals: 0 }) }}</template>
        <template #s><span v-if="overCap" style="color:var(--warn);font-weight:700">⚠ OVER CAP · </span>{{ capPct.toFixed(0) }}% of the {{ formatPiv(cap, { decimals: 0 }) }} PIV cap{{ allottedUsd ? ' · ≈ $' + Math.round(allottedUsd).toLocaleString('en-US') : '' }}</template>
      </Stat>
      <Stat k="PROPOSALS">
        <template #v>{{ projection.length }}<span class="unit">/ {{ proposals.length }}</span></template>
        <template #s>next payout / submitted</template>
      </Stat>
      <Stat k="TREASURY DEMAND">
        <template #v>{{ formatPiv(demand, { decimals: 0 }) }}</template>
        <template #s>PIV/month requested (all proposals)</template>
      </Stat>
    </div>

    <!-- ALLOCATION -->
    <h2 class="section-title">Treasury allocation — next superblock payout (#{{ formatCount(sbHeight) }})</h2>
    <HudPanel title="BUDGET ALLOCATION" id="/budgetprojection · next-payout Allotted share" hero>
      <template #head><span class="pill cyan mono">Σ {{ formatPiv(allotted, { decimals: 0 }) }} PIV</span></template>
      <EChart v-if="projection.length" :option="allocOption" height="74px" aria-label="Treasury allocation share across next-payout proposals" />
      <div class="alloc-legend" v-if="projection.length">
        <div v-for="(r, i) in projection" :key="r.Hash" class="al">
          <span class="al-dot" :style="{ background: PAL[i % PAL.length] }"></span>
          <RouterLink :to="`/proposal/${encodeURIComponent(r.Name)}`" class="al-name">{{ r.Name }}</RouterLink>
          <span class="al-val mono">{{ formatPiv(r.Allotted, { decimals: 0 }) }} PIV</span>
        </div>
      </div>
    </HudPanel>

    <!-- FUNDED THIS CYCLE — the most recent superblock's actual payout, derived from
         budget status to match the on-chain payment set without a per-block scan. -->
    <h2 class="section-title">Funded this cycle — superblock #{{ formatCount(lastSbHeight) }}</h2>
    <HudPanel title="BUDGET PAID" id="funded set · matches the on-chain superblock payout" hero>
      <template #head><span class="pill ok mono">Σ {{ formatPiv(fundedThisCycle.total, { decimals: 0 }) }} PIV</span></template>
      <div class="alloc-legend" v-if="fundedThisCycle.rows.length">
        <div v-for="(p, i) in fundedThisCycle.rows" :key="p.Hash" class="al">
          <span class="al-dot" :style="{ background: PAL[i % PAL.length] }"></span>
          <RouterLink :to="`/proposal/${encodeURIComponent(p.Name)}`" class="al-name">{{ p.Name }}</RouterLink>
          <span class="al-val mono">{{ formatPiv(p.MonthlyPayment, { decimals: 0 }) }} PIV</span>
        </div>
      </div>
      <p v-else class="note mono dim">No proposals were funded at the most recent superblock.</p>
      <p class="note mono dim">
        {{ fundedThisCycle.rows.length }} passing proposals whose payment window covers superblock
        <RouterLink :to="`/block/${lastSbHeight}`" class="mono">#{{ formatCount(lastSbHeight) }}</RouterLink>,
        ranked by net approval and funded to the {{ formatPiv(cap, { decimals: 0 }) }} PIV cap{{ usd(fundedThisCycle.total) ? ' · ≈ ' + usd(fundedThisCycle.total) : '' }} — the same set paid on-chain across the superblock blocks, one proposal per block.
      </p>
    </HudPanel>

    <!-- FINALIZED BUDGET (mnfinalbudget show) -->
    <template v-if="finalized.length">
      <h2 class="section-title">Finalized budget</h2>
      <HudPanel v-for="f in finalized" :key="f.hash || f.name" title="FINALIZED BUDGET" id="mnfinalbudget · OP_RETURN collateral">
        <template #head><span class="pill mono" :class="f.status === 'OK' ? 'ok' : 'bad'">{{ f.status }}</span></template>
        <div class="fb">
          <div class="fb-row"><span class="fb-k">Finalized</span><span class="fb-v">
            <template v-if="f.atTime">{{ formatDateTime(f.atTime) }} · block <RouterLink :to="`/block/${f.atHeight}`">#{{ formatCount(f.atHeight) }}</RouterLink></template>
            <span v-else class="dim">— (finalization tx unconfirmed)</span>
          </span></div>
          <div class="fb-row"><span class="fb-k">Finalization tx</span><span class="fb-v"><RouterLink :to="`/tx/${f.feeTx}`" class="mono">{{ truncateHash(f.feeTx, 14, 12) }}</RouterLink> <span class="pill warn mono">OP_RETURN</span></span></div>
          <div class="fb-row"><span class="fb-k">Superblock window</span><span class="fb-v mono"><RouterLink :to="`/block/${f.blockStart}`">#{{ formatCount(f.blockStart) }}</RouterLink> → #{{ formatCount(f.blockEnd) }}</span></div>
          <div class="fb-row"><span class="fb-k">Votes</span><span class="fb-v mono">{{ formatCount(f.votes) }}</span></div>
          <div class="fb-row"><span class="fb-k">Proposals</span><span class="fb-v dim">{{ f.proposals }}</span></div>
        </div>
      </HudPanel>
    </template>

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
              <td class="num strong">
                {{ formatPiv(p.MonthlyPayment, { decimals: 0 }) }}
                <div v-if="usd(p.MonthlyPayment)" class="dim mono" style="font-weight:400;font-size:11px;margin-top:2px">≈ {{ usd(p.MonthlyPayment) }}/mo · {{ usd(p.TotalPayment) }} total</div>
              </td>
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

/* Finalized-budget key/value rows */
.fb { display: grid; gap: 9px; }
.fb-row { display: grid; grid-template-columns: 158px 1fr; gap: 14px; align-items: baseline; font-size: 13px; line-height: 1.5; }
.fb-k { color: var(--text-dim); font-family: var(--font-mono); font-size: 10.5px; letter-spacing: 0.06em; text-transform: uppercase; }
.fb-v { color: var(--text); min-width: 0; overflow-wrap: anywhere; }
@media (max-width: 560px) { .fb-row { grid-template-columns: 1fr; gap: 2px; } }
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
