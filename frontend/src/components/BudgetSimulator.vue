<script setup>
/* =====================================================================
   BUDGET SIMULATOR — a what-if treasury sandbox for the Governance HUD.
   Toggle proposals in/out and nudge their net votes; the monthly payout
   re-ranks and re-funds live within the budget cap so voters can SEE the
   vote -> funding causality.

   FUNDING RULE (ported faithfully from frontend-legacy):
   rank SELECTED proposals by (possibly adjusted) net votes desc, then walk
   the list funding each whose MonthlyPayment still fits under the cap. This
   is greedy-SKIP, not hard-stop: a small proposal ranked below a skipped
   large one can still be funded if it fits the remaining headroom.

   Outcome per proposal: Funded / Cut by cap (selected, over cap) /
   Excluded (not selected).
   UNITS: MonthlyPayment / cap are PIV f64 NUMBERS -> formatPiv, never /1e8.
   ===================================================================== */
import { reactive, computed, onMounted, watch } from 'vue'
import { monthlyBudgetCap } from '../api/client.js'
import { formatPiv } from '../lib/money.js'
import { formatCount } from '../lib/format.js'
import HudPanel from './HudPanel.vue'

const props = defineProps({
  // All candidate proposals (the same /budgetinfo set the page loaded).
  candidates: { type: Array, default: () => [] },
  // Proposals funded right now (/budgetprojection) — preload + reset target.
  actualFunded: { type: Array, default: () => [] },
  // Net-vote passing threshold (10% of the masternode count).
  threshold: { type: Number, default: 0 },
})

const CAP = monthlyBudgetCap()
const VOTE_STEP = 50

const actualNet = (p) => (p.Yeas || 0) - (p.Nays || 0)

/* --- Interactive state -------------------------------------------------
   reactive Set/object track add/delete mutations in Vue 3, so a toggle or
   a vote nudge re-runs every computed below. */
const selected = reactive(new Set())
const voteAdj = reactive({}) // Hash -> adjusted net votes (absent = use actual)

const effectiveNet = (p) => (p.Hash in voteAdj ? voteAdj[p.Hash] : actualNet(p))
const isAdjusted = (p) => p.Hash in voteAdj && voteAdj[p.Hash] !== actualNet(p)

const isSelected = (hash) => selected.has(hash)
const toggle = (hash) => { selected.has(hash) ? selected.delete(hash) : selected.add(hash) }
const selectAll = () => props.candidates.forEach((p) => selected.add(p.Hash))
const clear = () => selected.clear()
const resetToActual = () => {
  selected.clear()
  props.actualFunded.forEach((p) => selected.add(p.Hash))
  resetVotes() // "actual" means actual votes too, not just the actual selection
}

// Vote adjustment (clamped to a sane 0..MN-count range for the slider/stepper).
const setVotes = (p, v) => { voteAdj[p.Hash] = Math.max(0, Math.min(props.threshold * 10, Math.round(v))) }
const bump = (p, dir) => setVotes(p, effectiveNet(p) + dir * VOTE_STEP)
const resetRow = (p) => { delete voteAdj[p.Hash] }
const resetVotes = () => Object.keys(voteAdj).forEach((h) => delete voteAdj[h])
const votesAdjusted = computed(() => props.candidates.some((p) => isAdjusted(p)))

/* --- The cap-limited, vote-ranked allocation over the selected set ----- */
function simulate(proposals, netOf, cap, threshold) {
  const sorted = [...proposals].sort((a, b) => netOf(b) - netOf(a))
  const ranked = []
  let allocated = 0
  let overBy = 0
  for (const proposal of sorted) {
    const amount = proposal.MonthlyPayment || 0
    // The 10% net-vote gate is a funding ELIGIBILITY criterion, not just a
    // label: a below-gate proposal can never be funded no matter how small
    // (matches real PIVX /budgetprojection). Only eligible proposals compete
    // for the cap.
    const eligible = netOf(proposal) > threshold
    const funded = eligible && allocated + amount <= cap
    if (funded) allocated += amount
    else if (eligible) overBy += amount
    const outcome = funded ? 'funded' : eligible ? 'cut' : 'belowgate'
    ranked.push({ proposal, funded, outcome, amount, cumulative: allocated })
  }
  return {
    ranked, allocated, overBy, cap,
    remaining: cap - allocated,
    fundedCount: ranked.filter((r) => r.outcome === 'funded').length,
    cutCount: ranked.filter((r) => r.outcome === 'cut').length,
    belowGateCount: ranked.filter((r) => r.outcome === 'belowgate').length,
  }
}

const selectedProposals = computed(() => props.candidates.filter((p) => selected.has(p.Hash)))
const sim = computed(() => simulate(selectedProposals.value, effectiveNet, CAP, props.threshold))
const simByHash = computed(() => {
  const m = new Map()
  for (const r of sim.value.ranked) m.set(r.proposal.Hash, r)
  return m
})

/* --- Live panel derivations ------------------------------------------- */
const selectedCount = computed(() => selected.size)
const utilizationPct = computed(() => (CAP ? Math.round((sim.value.allocated / CAP) * 1000) / 10 : 0))
const overCap = computed(() => sim.value.cutCount > 0)
const delta = computed(() => {
  const actual = new Set(props.actualFunded.map((p) => p.Hash))
  let added = 0
  let removed = 0
  selected.forEach((h) => { if (!actual.has(h)) added++ })
  actual.forEach((h) => { if (!selected.has(h)) removed++ })
  return { added, removed }
})

/* --- Sorting --------------------------------------------------------- */
const sortKey = reactive({ key: 'netVotes', dir: 'desc' })
const setSort = (key) => {
  if (sortKey.key === key) sortKey.dir = sortKey.dir === 'desc' ? 'asc' : 'desc'
  else { sortKey.key = key; sortKey.dir = key === 'name' ? 'asc' : 'desc' }
}
const ariaSort = (key) => (sortKey.key !== key ? 'none' : sortKey.dir === 'asc' ? 'ascending' : 'descending')
const sortGlyph = (key) => (sortKey.key !== key ? '↕' : sortKey.dir === 'asc' ? '▲' : '▼')

const rows = computed(() => {
  const list = props.candidates.map((p) => {
    const r = simByHash.value.get(p.Hash)
    return {
      proposal: p,
      selected: selected.has(p.Hash),
      fundedInSim: !!r?.funded,
      outcome: r?.outcome ?? null,
      cumulative: r?.cumulative ?? 0,
      net: effectiveNet(p),
      adjusted: isAdjusted(p),
    }
  })
  const dir = sortKey.dir === 'asc' ? 1 : -1
  return list.sort((a, b) => {
    if (sortKey.key === 'name') return dir * String(a.proposal.Name).localeCompare(String(b.proposal.Name))
    const av = sortKey.key === 'monthly' ? (a.proposal.MonthlyPayment || 0) : a.net
    const bv = sortKey.key === 'monthly' ? (b.proposal.MonthlyPayment || 0) : b.net
    return dir * (av - bv)
  })
})

/* No single "cut line": greedy-skip can fund a row below a skipped larger one,
   so the per-row Funded / Cut by cap / Below-gate status pill is the source of
   truth for each proposal's outcome. */

/* --- Select-all checkbox tri-state ------------------------------------ */
const allSelected = computed(() => props.candidates.length > 0 && selectedCount.value === props.candidates.length)
const someSelected = computed(() => selectedCount.value > 0 && !allSelected.value)
const toggleAll = () => (allSelected.value ? clear() : selectAll())

/* --- Display helpers --------------------------------------------------- */
const piv = (n) => formatPiv(n, { decimals: 0 })
const capShare = (p) => (CAP ? (((p.MonthlyPayment || 0) / CAP) * 100).toFixed(1) : '0.0')
const signed = (n) => (n > 0 ? `+${formatCount(n)}` : formatCount(n))
const proposalLink = (p) => `/proposal/${encodeURIComponent(p.Name)}`
const rowClass = (row) => ({
  on: row.selected,
  funded: row.selected && row.outcome === 'funded',
  cut: row.selected && row.outcome === 'cut',
  gate: row.selected && row.outcome === 'belowgate',
})

/* --- Seed today's funded set once data arrives ------------------------- */
let seeded = false
const trySeed = () => {
  if (seeded) return
  if (props.candidates.length || props.actualFunded.length) { resetToActual(); seeded = true }
}
onMounted(trySeed)
watch(() => props.actualFunded, trySeed)
</script>

<template>
  <div class="sim">
    <!-- ===== LIVE ALLOCATION PANEL ===== -->
    <HudPanel title="SIMULATED MONTHLY ALLOCATION" :id="`cap ${piv(CAP)} PIV · greedy by net votes`" hero>
      <template #head>
        <span class="pill mono" :class="overCap ? 'warn' : 'cyan'">{{ utilizationPct }}% OF CAP</span>
      </template>

      <div class="hero" :class="{ over: overCap }">
        <span class="hero-v mono">{{ piv(sim.allocated) }}</span>
        <span class="hero-u mono">/ {{ piv(CAP) }} PIV</span>
      </div>

      <!-- Budget bar: % of cap used, distinct over-cap state -->
      <div
        class="bar" :class="{ over: overCap }" role="img"
        :aria-label="`${utilizationPct}% of the ${piv(CAP)} PIV monthly cap allocated`"
      >
        <i class="bar-fill" :style="{ width: Math.min(utilizationPct, 100) + '%' }"></i>
      </div>
      <div class="bar-cap mono">
        <span>{{ utilizationPct }}% of cap used</span>
        <span :class="sim.remaining > 0 ? 'good' : 'dim'">{{ piv(sim.remaining) }} PIV headroom</span>
      </div>

      <!-- Live counts + delta vs today (announced) -->
      <div class="counts mono" aria-live="polite">
        <span class="ct"><b>{{ selectedCount }}</b> selected</span>
        <span class="ct"><b class="good">{{ sim.fundedCount }}</b> funded</span>
        <span class="ct"><b class="warn-t">{{ sim.cutCount }}</b> cut by cap</span>
        <span v-if="sim.belowGateCount" class="ct"><b class="hot-t">{{ sim.belowGateCount }}</b> below gate</span>
        <span class="ct delta">
          vs today <b class="good">+{{ delta.added }}</b><b class="hot-t">−{{ delta.removed }}</b>
        </span>
      </div>

      <!-- Over-cap warning -->
      <div v-if="overCap" class="warn-box mono" role="status">
        ⚠ {{ sim.cutCount }} selected proposal{{ sim.cutCount > 1 ? 's' : '' }} cut by the cap —
        {{ piv(sim.overBy) }} PIV of demand couldn't fit.
      </div>

      <!-- Actions -->
      <div class="actions">
        <button class="gbtn" type="button" @click="resetToActual">↺ Reset to actual</button>
        <button class="gbtn" type="button" @click="selectAll">Select all</button>
        <button class="gbtn" type="button" @click="clear">Clear</button>
        <button class="gbtn" type="button" :disabled="!votesAdjusted" @click="resetVotes">Reset votes</button>
      </div>
    </HudPanel>

    <!-- ===== CANDIDATE TABLE ===== -->
    <HudPanel title="CANDIDATE PROPOSALS" id="include · nudge votes · watch the payout re-rank">
      <div class="scroll">
        <table class="dtable sim-table">
          <thead>
            <tr>
              <th class="cx">
                <input
                  type="checkbox" :checked="allSelected" :indeterminate.prop="someSelected"
                  aria-label="Select all proposals" @change="toggleAll"
                />
              </th>
              <th :aria-sort="ariaSort('name')">
                <button class="sorth" type="button" @click="setSort('name')">
                  Proposal <span class="caret" :class="{ on: sortKey.key === 'name' }">{{ sortGlyph('name') }}</span>
                </button>
              </th>
              <th :aria-sort="ariaSort('netVotes')">
                <button class="sorth" type="button" @click="setSort('netVotes')">
                  Net Votes <span class="caret" :class="{ on: sortKey.key === 'netVotes' }">{{ sortGlyph('netVotes') }}</span>
                </button>
              </th>
              <th class="num">
                <button class="sorth num" type="button" @click="setSort('monthly')">
                  Monthly PIV <span class="caret" :class="{ on: sortKey.key === 'monthly' }">{{ sortGlyph('monthly') }}</span>
                </button>
              </th>
              <th class="num">% Cap</th>
              <th class="num">Cumulative</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="row in rows" :key="row.proposal.Hash">
              <tr :class="rowClass(row)" @click="toggle(row.proposal.Hash)">
                <td class="cx">
                  <input
                    type="checkbox" :checked="row.selected" :aria-label="`Include ${row.proposal.Name}`"
                    @click.stop @change="toggle(row.proposal.Hash)"
                  />
                </td>
                <td>
                  <RouterLink :to="proposalLink(row.proposal)" class="strong" @click.stop>{{ row.proposal.Name }}</RouterLink>
                </td>
                <td>
                  <div class="vote" @click.stop>
                    <button class="vstep" type="button" :aria-label="`Decrease ${row.proposal.Name} net votes by ${VOTE_STEP}`" @click="bump(row.proposal, -1)">−</button>
                    <span class="vval mono" :class="{ low: row.net < threshold }">{{ signed(row.net) }}</span>
                    <button class="vstep" type="button" :aria-label="`Increase ${row.proposal.Name} net votes by ${VOTE_STEP}`" @click="bump(row.proposal, 1)">+</button>
                    <button v-if="row.adjusted" class="vreset" type="button" :aria-label="`Reset ${row.proposal.Name} votes to actual`" title="Reset to actual votes" @click="resetRow(row.proposal)">↺</button>
                  </div>
                  <span v-if="row.net < threshold" class="below mono">below 10% gate</span>
                </td>
                <td class="num strong">{{ piv(row.proposal.MonthlyPayment) }}</td>
                <td class="num dim">{{ capShare(row.proposal) }}%</td>
                <td class="num dim">{{ row.selected && row.fundedInSim ? piv(row.cumulative) : '—' }}</td>
                <td>
                  <span v-if="!row.selected" class="pill">Excluded</span>
                  <span v-else-if="row.outcome === 'funded'" class="pill ok">Funded</span>
                  <span v-else-if="row.outcome === 'belowgate'" class="pill gate">Below gate</span>
                  <span v-else class="pill warn">Cut by cap</span>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>
      <p class="note mono dim">
        Funding order is by net votes (desc); each row's Status pill shows whether the cap funds it.
        Nudge any row's votes with −/+ ({{ VOTE_STEP }} each) and the payout re-ranks live.
      </p>
    </HudPanel>
  </div>
</template>

<style scoped>
.sim { display: flex; flex-direction: column; gap: var(--space-5); }

/* ----- hero allocation ----- */
.hero { display: flex; align-items: baseline; gap: 10px; line-height: 1; margin-bottom: var(--space-4); }
.hero-v { font-size: clamp(28px, 4vw, 44px); font-weight: 700; color: var(--success); text-shadow: 0 0 14px rgba(92,203,111,0.3); }
.hero.over .hero-v { color: var(--warn); text-shadow: 0 0 14px rgba(246,211,90,0.3); }
.hero-u { font-size: 15px; color: var(--text-dim); }

.bar { position: relative; height: 14px; border-radius: var(--radius-pill); overflow: hidden; background: rgba(150,90,220,0.14); border: 1px solid var(--hud-line); }
.bar-fill { position: absolute; left: 0; top: 0; bottom: 0; border-radius: var(--radius-pill); background: var(--holo); transition: width .3s ease; }
.bar.over .bar-fill { background: linear-gradient(90deg, var(--amber), var(--hot)); }
.bar-cap { display: flex; justify-content: space-between; margin-top: 6px; font-size: 11px; color: var(--text-dim); }
.bar-cap .good { color: var(--success); }

.counts { display: flex; flex-wrap: wrap; gap: var(--space-5); margin-top: var(--space-4); padding-top: var(--space-4); border-top: 1px solid var(--hud-line); font-size: 12.5px; color: var(--text-muted); }
.ct b { font-size: 16px; color: var(--text); margin-right: 5px; }
.ct b.good { color: var(--success); } .ct b.warn-t { color: var(--warn); } .ct b.hot-t { color: var(--hot); }
.delta { margin-left: auto; display: flex; gap: 8px; align-items: baseline; }
.delta b { font-size: 13px; }

.warn-box { margin-top: var(--space-4); padding: 9px 12px; border-radius: var(--radius-md); font-size: 12px; background: rgba(246,211,90,0.1); border: 1px solid rgba(246,211,90,0.35); color: var(--warn); }

.actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: var(--space-4); }
.actions .gbtn:disabled { opacity: 0.4; cursor: not-allowed; }

/* ----- table ----- */
.scroll { overflow-x: auto; }
.sim-table { min-width: 720px; }
.sim-table tbody tr { cursor: pointer; }
.sim-table .cx { width: 38px; text-align: center; }
.sim-table .cx input { width: 16px; height: 16px; cursor: pointer; accent-color: var(--neon-soft); }
.sorth { background: none; border: 0; font: inherit; color: inherit; text-transform: inherit; letter-spacing: inherit; cursor: pointer; padding: 0; display: inline-flex; align-items: center; gap: 4px; }
.sorth.num { width: 100%; justify-content: flex-end; }
.sorth:hover { color: var(--text); }
.caret { color: var(--text-dim); font-size: 0.85em; }
.caret.on { color: var(--neon); }

/* selection accents (left rail) */
.sim-table tbody tr.on td { color: var(--text); }
.sim-table tbody tr.funded td:first-child { box-shadow: inset 3px 0 0 var(--success); }
.sim-table tbody tr.cut td:first-child { box-shadow: inset 3px 0 0 var(--warn); }
.sim-table tbody tr.gate td:first-child { box-shadow: inset 3px 0 0 var(--hot); }
.pill.gate { background: rgba(255, 90, 140, 0.13); color: var(--hot); border-color: rgba(255, 90, 140, 0.3); }

/* vote stepper */
.vote { display: inline-flex; align-items: center; gap: 6px; }
.vstep { width: 22px; height: 22px; display: grid; place-items: center; border-radius: 5px; cursor: pointer;
  background: var(--glass-2); border: 1px solid var(--hud-line); color: var(--neon); font-size: 14px; line-height: 1; font-family: var(--font-mono); }
.vstep:hover { border-color: var(--glass-edge-strong); box-shadow: var(--glow-xs); color: var(--text); }
.vval { min-width: 56px; text-align: center; font-weight: 600; color: var(--text); }
.vval.low { color: var(--amber); }
.vreset { width: 22px; height: 22px; border-radius: 5px; cursor: pointer; background: transparent; border: 1px solid var(--hud-line); color: var(--cyan); font-size: 12px; }
.vreset:hover { border-color: var(--cyan); box-shadow: var(--glow-cyan); }
.below { display: block; margin-top: 3px; font-size: 9.5px; letter-spacing: 0.1em; text-transform: uppercase; color: var(--amber); }

.note { margin: var(--space-4) 0 0; font-size: 10.5px; padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
</style>
