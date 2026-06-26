<template>
  <div class="budget-simulator">
    <!-- ===== Live result panel (the one glass hero on the page) ===== -->
    <Card class="sim-panel">
      <div class="sim-panel-head">
        <div class="sim-headline">
          <div class="sim-eyebrow">Simulated Monthly Allocation</div>
          <div class="sim-hero" :class="{ 'is-over': simulation.cutCount > 0 }">
            <span class="sim-hero-value">{{ formatNumber(simulation.allocated) }}</span>
            <span class="sim-hero-unit">/ {{ formatNumber(cap) }} PIV</span>
          </div>
        </div>
        <div class="sim-actions">
          <Button variant="ghost" size="sm" @click="resetToActual" title="Re-select today's funded proposals">
            <Icon name="refresh-cw" :size="14" /> Reset to actual
          </Button>
          <Button variant="ghost" size="sm" @click="clear" title="Deselect everything">
            Clear
          </Button>
          <Button variant="accent" size="sm" :disabled="candidates.length === 0" @click="exportCsv">
            <Icon name="download" :size="14" /> Export CSV
          </Button>
        </div>
      </div>

      <!-- Segmented budget bar with the cap as 100% -->
      <div class="sim-bar" role="img"
           :aria-label="`${utilizationPct}% of the ${formatNumber(cap)} PIV cap allocated`">
        <div class="sim-bar-fill" :style="{ width: Math.min(utilizationPct, 100) + '%' }"></div>
      </div>
      <div class="sim-bar-caption">
        <span>{{ utilizationPct }}% of cap used</span>
        <span :class="simulation.remaining > 0 ? 'stat-success' : 'stat-tertiary'">
          {{ formatNumber(simulation.remaining) }} PIV headroom
        </span>
      </div>

      <!-- Live counts + delta vs today -->
      <div class="sim-stats" aria-live="polite">
        <div class="sim-stat"><span class="n">{{ selectedCount }}</span> selected</div>
        <div class="sim-stat"><span class="n stat-success">{{ simulation.fundedCount }}</span> funded</div>
        <div class="sim-stat"><span class="n stat-warning">{{ simulation.cutCount }}</span> cut by cap</div>
        <div class="sim-stat sim-delta">
          vs today
          <span class="stat-success">+{{ delta.added }}</span>
          <span class="stat-danger">&minus;{{ delta.removed }}</span>
        </div>
      </div>

      <!-- Over-cap warning -->
      <div v-if="simulation.cutCount > 0" class="sim-warning" role="status">
        <Icon name="alert-triangle" :size="15" />
        {{ simulation.cutCount }} selected proposal{{ simulation.cutCount > 1 ? 's' : '' }}
        cut by the cap &mdash; {{ formatNumber(simulation.overBy) }} PIV couldn't fit.
      </div>
    </Card>

    <!-- ===== Selectable / sortable candidate table (flat data surface) ===== -->
    <Card variant="data" class="sim-table-card">
      <div class="sim-table-head">
        <h3 class="sim-table-title">Candidate proposals</h3>
        <div class="sim-table-tools">
          <button class="link-btn" type="button" @click="selectAll">Select all</button>
          <button class="link-btn" type="button" @click="clear">Clear</button>
          <span v-if="!fundingOrder" class="sim-hint">
            Sorted by {{ sortLabel }} &mdash; funding order is by net votes
          </span>
        </div>
      </div>

      <div class="sim-table-scroll">
        <table class="sim-table">
          <thead>
            <tr>
              <th class="col-check">
                <input
                  type="checkbox"
                  :checked="allSelected"
                  :indeterminate.prop="someSelected"
                  aria-label="Select all proposals"
                  @change="toggleAll"
                />
              </th>
              <th class="col-name sortable" :aria-sort="ariaSort('name')" @click="setSort('name')">
                Proposal <span class="sort-caret" :class="{ active: sortKey === 'name' }">{{ sortGlyph('name') }}</span>
              </th>
              <th class="num sortable" :aria-sort="ariaSort('netVotes')" @click="setSort('netVotes')">
                Net Votes <span class="sort-caret" :class="{ active: sortKey === 'netVotes' }">{{ sortGlyph('netVotes') }}</span>
              </th>
              <th class="num sortable" :aria-sort="ariaSort('monthly')" @click="setSort('monthly')">
                Monthly PIV <span class="sort-caret" :class="{ active: sortKey === 'monthly' }">{{ sortGlyph('monthly') }}</span>
              </th>
              <th class="num">% Cap</th>
              <th class="num">Cumulative</th>
              <th class="col-status">Status</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="row in sortedRows" :key="row.proposal.Hash">
              <tr v-if="cutLineHash === row.proposal.Hash" class="cut-line-row" aria-hidden="true">
                <td :colspan="7">
                  <span class="cut-line-label">{{ formatNumber(cap) }} PIV monthly cap</span>
                </td>
              </tr>
              <tr :class="rowClass(row)" @click="toggle(row.proposal.Hash)">
                <td class="col-check">
                  <input
                    type="checkbox"
                    :checked="row.selected"
                    :aria-label="`Include ${row.proposal.Name}`"
                    @click.stop
                    @change="toggle(row.proposal.Hash)"
                  />
                </td>
                <td class="col-name">
                  <RouterLink :to="proposalLink(row.proposal)" class="prop-name" @click.stop>
                    {{ row.proposal.Name }}
                  </RouterLink>
                </td>
                <td class="num" :class="{ 'below-threshold': row.netVotes < passingThreshold }"
                    :title="row.netVotes < passingThreshold ? 'Below the 10% passing threshold' : 'Meets the passing threshold'">
                  {{ signed(row.netVotes) }}
                </td>
                <td class="num">{{ formatNumber(row.proposal.MonthlyPayment) }}</td>
                <td class="num">{{ budgetShare(row.proposal) }}%</td>
                <td class="num">{{ row.selected && row.fundedInSim ? formatNumber(row.cumulative) : '—' }}</td>
                <td class="col-status">
                  <span v-if="!row.selected" class="pill pill-muted">Excluded</span>
                  <span v-else-if="row.fundedInSim" class="pill pill-funded">Funded</span>
                  <span v-else class="pill pill-cut">Cut by cap</span>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>

      <EmptyState
        v-if="candidates.length === 0"
        icon="inbox"
        title="No active proposals"
        message="There are no proposals with remaining payments to simulate."
      />
    </Card>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import Card from '@/components/common/Card.vue'
import Button from '@/components/common/Button.vue'
import Icon from '@/components/common/Icon.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import { formatNumber } from '@/utils/formatters'
import { PIVX_GOVERNANCE } from '@/utils/governanceStatus'
import { useBudgetSimulation } from '@/composables/useBudgetSimulation'
import { toCsv, downloadCsv } from '@/utils/csv'

const props = defineProps({
  // Active candidate proposals (valid, not completed, remaining payments > 0)
  candidates: { type: Array, default: () => [] },
  // Proposals funded right now (preload + reset target)
  actualFunded: { type: Array, default: () => [] },
  // 10% passing threshold in net votes (for the below-threshold hint)
  passingThreshold: { type: Number, default: 0 },
})

const cap = PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET

const candidates = computed(() => props.candidates)

const {
  selected, selectedCount, isSelected, toggle, selectAll, clear, resetToActual,
  simulation, simByHash, delta,
} = useBudgetSimulation(candidates, () => props.actualFunded)

// --- Preload today's funded set once the data is available ---
let seeded = false
const trySeed = () => {
  if (seeded) return
  if (candidates.value.length || props.actualFunded.length) {
    resetToActual()
    seeded = true
  }
}
onMounted(trySeed)
watch(() => props.actualFunded, trySeed, { deep: false })

// --- Sorting ---
const sortKey = ref('netVotes')
const sortDir = ref('desc')
const sortLabels = { netVotes: 'net votes', monthly: 'monthly payment', name: 'name' }
const sortLabel = computed(() => sortLabels[sortKey.value] || sortKey.value)
// The cut line only makes sense when the table is in the protocol's funding order.
const fundingOrder = computed(() => sortKey.value === 'netVotes' && sortDir.value === 'desc')

const setSort = (key) => {
  if (sortKey.value === key) {
    sortDir.value = sortDir.value === 'desc' ? 'asc' : 'desc'
  } else {
    sortKey.value = key
    sortDir.value = key === 'name' ? 'asc' : 'desc'
  }
}
const ariaSort = (key) => (sortKey.value !== key ? 'none' : sortDir.value === 'asc' ? 'ascending' : 'descending')

// --- Rows: candidates annotated with their simulation outcome ---
const annotated = computed(() => candidates.value.map((p) => {
  const r = simByHash.value.get(p.Hash)
  return {
    proposal: p,
    selected: isSelected(p.Hash),
    fundedInSim: !!r?.funded,
    cumulative: r?.cumulative ?? 0,
    netVotes: (p.Yeas || 0) - (p.Nays || 0),
  }
}))

const sortedRows = computed(() => {
  const dir = sortDir.value === 'asc' ? 1 : -1
  return [...annotated.value].sort((a, b) => {
    if (sortKey.value === 'name') {
      return dir * String(a.proposal.Name || '').localeCompare(String(b.proposal.Name || ''))
    }
    const av = sortKey.value === 'monthly' ? (a.proposal.MonthlyPayment || 0) : a.netVotes
    const bv = sortKey.value === 'monthly' ? (b.proposal.MonthlyPayment || 0) : b.netVotes
    return dir * (av - bv)
  })
})

// First selected proposal (in vote rank) that the cap cuts -> where we draw the line.
const cutLineHash = computed(() => {
  if (!fundingOrder.value) return null
  const cut = simulation.value.ranked.find((r) => !r.funded)
  return cut ? cut.proposal.Hash : null
})

// --- Select-all checkbox state ---
const allSelected = computed(() => candidates.value.length > 0 && selectedCount.value === candidates.value.length)
const someSelected = computed(() => selectedCount.value > 0 && selectedCount.value < candidates.value.length)
const toggleAll = () => { if (allSelected.value) clear(); else selectAll() }

// --- Display helpers ---
const utilizationPct = computed(() => (cap ? Math.round((simulation.value.allocated / cap) * 1000) / 10 : 0))
const signed = (n) => (n > 0 ? `+${formatNumber(n)}` : formatNumber(n))
const budgetShare = (p) => (cap ? (((p.MonthlyPayment || 0) / cap) * 100).toFixed(1) : '0.0')
const proposalLink = (p) => `/proposal/${encodeURIComponent(p.Name)}`

const rowClass = (row) => ({
  'is-selected': row.selected,
  'is-funded': row.selected && row.fundedInSim,
  'is-cut': row.selected && !row.fundedInSim,
})

// Sort caret glyph for a column header (↕ idle, ▲/▼ when active).
const sortGlyph = (key) => (sortKey.value !== key ? '↕' : sortDir.value === 'asc' ? '▲' : '▼')

// --- CSV export: the selected scenario, in funding (vote-rank) order ---
const CSV_COLUMNS = [
  { key: 'rank', label: 'Rank' },
  { key: 'name', label: 'Name' },
  { key: 'status', label: 'Status' },
  { key: 'netVotes', label: 'NetVotes' },
  { key: 'yeas', label: 'Yeas' },
  { key: 'nays', label: 'Nays' },
  { key: 'monthly', label: 'MonthlyPayment' },
  { key: 'total', label: 'TotalPayment' },
  { key: 'remainingPayments', label: 'RemainingPayments' },
  { key: 'budgetShare', label: 'BudgetSharePct' },
  { key: 'cumulative', label: 'CumulativeAllocated' },
  { key: 'address', label: 'PaymentAddress' },
  { key: 'hash', label: 'Hash' },
  { key: 'url', label: 'URL' },
]

const netVotesOf = (p) => (p.Yeas || 0) - (p.Nays || 0)

const toCsvRow = (p, status, rank, cumulative) => ({
  rank,
  name: p.Name,
  status,
  netVotes: netVotesOf(p),
  yeas: p.Yeas,
  nays: p.Nays,
  monthly: p.MonthlyPayment,
  total: p.TotalPayment,
  remainingPayments: p.RemainingPaymentCount,
  budgetShare: budgetShare(p),
  cumulative,
  address: p.PaymentAddress,
  hash: p.Hash,
  url: p.URL,
})

const exportCsv = () => {
  const sim = simulation.value
  const blank = Object.fromEntries(CSV_COLUMNS.map((c) => [c.key, '']))
  const sectionRow = (label) => ({ ...blank, rank: label })

  // FUNDED: selected proposals that fit the cap, in vote-rank order.
  const fundedRows = sim.ranked
    .filter((r) => r.funded)
    .map((r, i) => toCsvRow(r.proposal, 'Funded', i + 1, r.cumulative))

  // NOT FUNDED: selected-but-over-cap ("Cut by cap"), then never-selected ("Excluded").
  const cutRows = sim.ranked
    .filter((r) => !r.funded)
    .map((r) => toCsvRow(r.proposal, 'Cut by cap', '', ''))
  const excludedRows = candidates.value
    .filter((p) => !isSelected(p.Hash))
    .slice()
    .sort((a, b) => netVotesOf(b) - netVotesOf(a))
    .map((p) => toCsvRow(p, 'Excluded', '', ''))
  const notFundedRows = [...cutRows, ...excludedRows]
  const notFundedMonthly = notFundedRows.reduce((s, r) => s + (r.monthly || 0), 0)

  const rows = [
    sectionRow(`FUNDED (${fundedRows.length})`),
    ...fundedRows,
    blank,
    sectionRow(`NOT FUNDED (${notFundedRows.length})`),
    ...notFundedRows,
    blank,
    {
      ...blank,
      rank: 'TOTAL',
      name: `${fundedRows.length} funded / ${notFundedRows.length} not funded`,
      monthly: sim.allocated,
      total: notFundedMonthly,
      cumulative: `cap ${cap} / remaining ${sim.remaining}`,
    },
  ]

  const date = new Date().toISOString().slice(0, 10)
  downloadCsv(`pivx-budget-sim_${date}.csv`, toCsv(CSV_COLUMNS, rows))
}
</script>

<style scoped>
.budget-simulator {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

/* ===== Live panel ===== */
.sim-panel-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-4);
  flex-wrap: wrap;
}
.sim-eyebrow {
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
  font-weight: var(--weight-bold);
}
.sim-hero {
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
  color: var(--success);
  margin-top: var(--space-1);
  line-height: 1;
}
.sim-hero.is-over { color: var(--warning); }
.sim-hero-value { font-size: var(--text-3xl); }
.sim-hero-unit { font-size: var(--text-md); color: var(--text-tertiary); margin-left: var(--space-2); }
.sim-actions { display: flex; gap: var(--space-2); flex-wrap: wrap; }

.sim-bar {
  position: relative;
  height: 14px;
  margin-top: var(--space-5);
  background: rgba(var(--rgb-purple-darkest), 0.6);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-full);
  overflow: hidden;
}
.sim-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--green-accent-dark), var(--green-accent));
  border-radius: var(--radius-full);
  transition: width var(--transition-slow);
}
.sim-bar-caption {
  display: flex;
  justify-content: space-between;
  margin-top: var(--space-2);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-family: var(--font-mono);
}

.sim-stats {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-5);
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}
.sim-stat .n { font-family: var(--font-mono); font-weight: var(--weight-bold); font-size: var(--text-lg); color: var(--text-primary); margin-right: var(--space-1); }
.sim-delta { margin-left: auto; display: flex; gap: var(--space-2); align-items: baseline; }
.sim-delta span { font-family: var(--font-mono); font-weight: var(--weight-semibold); }

.stat-success { color: var(--success); }
.stat-warning { color: var(--warning); }
.stat-danger { color: var(--danger-text); }
.stat-tertiary { color: var(--text-tertiary); }

.sim-warning {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-4);
  padding: var(--space-3) var(--space-4);
  background: rgba(246, 255, 120, 0.08);
  border: 1px solid rgba(246, 255, 120, 0.3);
  border-radius: var(--radius-sm);
  color: var(--warning);
  font-size: var(--text-sm);
}

/* ===== Table ===== */
.sim-table-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: var(--space-4);
  flex-wrap: wrap;
  margin-bottom: var(--space-3);
}
.sim-table-title { font-size: var(--text-lg); margin: 0; }
.sim-table-tools { display: flex; align-items: center; gap: var(--space-4); }
.link-btn {
  background: none;
  border: none;
  color: var(--text-purple);
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  cursor: pointer;
  padding: var(--space-1) 0;
}
.link-btn:hover { color: var(--text-accent); }
.sim-hint { font-size: var(--text-xs); color: var(--text-tertiary); }

.sim-table-scroll { overflow-x: auto; }
.sim-table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
.sim-table th {
  text-align: left;
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  font-weight: var(--weight-bold);
  color: var(--text-tertiary);
  border-bottom: 1px solid var(--border-subtle);
  white-space: nowrap;
}
.sim-table th.sortable { cursor: pointer; user-select: none; }
.sim-table th.sortable:hover { color: var(--text-secondary); }
.sim-table th.num, .sim-table td.num { text-align: right; font-family: var(--font-mono); font-variant-numeric: tabular-nums; white-space: nowrap; }
.sim-table tbody tr { border-bottom: 1px solid var(--border-subtle); cursor: pointer; transition: background-color var(--transition-fast); }
.sim-table tbody tr:hover { background: var(--bg-hover); }
.sim-table td { padding: var(--space-3) var(--space-4); color: var(--text-secondary); }

/* Row selection states (left accent border) */
.sim-table tbody tr.is-selected { box-shadow: inset 3px 0 0 var(--green-accent); }
.sim-table tbody tr.is-cut { box-shadow: inset 3px 0 0 var(--warning); }
.sim-table tbody tr:not(.is-selected) td { color: var(--text-tertiary); }

.col-check { width: 40px; text-align: center; }
.col-check input { width: 18px; height: 18px; cursor: pointer; accent-color: var(--green-accent-dark); }
.prop-name { color: var(--text-primary); font-weight: var(--weight-medium); text-decoration: none; }
.prop-name:hover { color: var(--text-accent); text-decoration: underline; }
.below-threshold { color: var(--danger-text); }

.sort-caret { color: var(--text-tertiary); font-size: 0.7em; margin-left: 2px; }
.sort-caret.active { color: var(--text-accent); }

/* Cut line */
.cut-line-row td {
  padding: 0;
  height: 0;
  border-bottom: 2px dashed var(--green-accent);
  position: relative;
}
.cut-line-label {
  position: absolute;
  right: var(--space-4);
  top: -10px;
  background: var(--surface-data);
  padding: 0 var(--space-2);
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  font-weight: var(--weight-bold);
  color: var(--green-accent);
  letter-spacing: var(--tracking-wide);
}

/* Status pills */
.pill {
  display: inline-block;
  padding: 2px var(--space-2);
  border-radius: var(--radius-full);
  font-size: var(--text-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  white-space: nowrap;
}
.pill-funded { background: rgba(var(--rgb-green-accent), 0.14); color: var(--success); }
.pill-cut { background: rgba(246, 255, 120, 0.12); color: var(--warning); }
.pill-muted { background: rgba(var(--rgb-purple-mid), 0.18); color: var(--text-tertiary); }

@media (max-width: 640px) {
  .sim-delta { margin-left: 0; }
  .sim-hero-value { font-size: var(--text-2xl); }
}
</style>
