<template>
  <div class="treasury-history">
    <div class="controls">
      <p class="controls-note">Superblock treasury payouts across the full chain history</p>
      <Button variant="ghost" size="sm" @click="exportData">
        <Icon name="download" :size="14" /> Export
      </Button>
    </div>

    <!-- Summary Stats -->
    <div class="stats-grid">
      <StatCard
        label="Total Paid (All Time)"
        :value="formatBalance(stats.totalAllTime)"
        suffix="PIV"
        icon="landmark"
        :loading="loading"
      />
      <StatCard
        label="Last Cycle"
        :value="formatBalance(stats.lastCycle)"
        suffix="PIV"
        :subtitle="stats.lastCycleSubtitle"
        icon="vote"
        :loading="loading"
      />
      <StatCard
        label="Avg per Cycle (Last 12)"
        :value="formatBalance(stats.avgCycle12)"
        suffix="PIV"
        icon="calendar"
        :loading="loading"
      />
      <StatCard
        label="Total Payouts"
        :value="formatNumber(stats.payoutCount)"
        icon="file-text"
        :loading="loading"
      />
    </div>

    <!-- Per-Cycle Spending Chart -->
    <BaseChart
      title="Treasury Spending per Cycle (43,200 blocks)"
      :option="cycleOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Recent Payouts Table -->
    <Card class="table-card">
      <div class="table-header">
        <h3>Recent Treasury Payouts</h3>
      </div>

      <div v-if="loading" class="loading-state">
        <LoadingSpinner />
        <p>Loading treasury history...</p>
      </div>

      <div v-else-if="error" class="error-state">
        <p>{{ error }}</p>
      </div>

      <div v-else-if="payouts.length === 0" class="empty-wrapper">
        <EmptyState
          icon="landmark"
          title="No Treasury Payouts"
          message="No treasury payout history is available yet."
        />
      </div>

      <div v-else class="table-container">
        <table class="treasury-table">
          <thead>
            <tr>
              <th>Block</th>
              <th>Date</th>
              <th class="text-right">Total Paid</th>
              <th class="text-right">Outputs</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="payout in recentPayouts" :key="payout.height" class="table-row">
              <td>
                <router-link :to="`/block/${payout.height}`" class="height-link">
                  #{{ formatNumber(payout.height) }}
                </router-link>
              </td>
              <td class="date-cell">
                {{ payout.date }}
              </td>
              <td class="text-right balance">
                {{ formatBalance(payout.totalPaid) }} PIV
              </td>
              <td class="text-right">
                <Badge variant="default" size="sm">
                  {{ payout.nOutputs }}
                </Badge>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="payouts.length > 0" class="table-footer">
        <p class="footer-text">
          Showing {{ recentPayouts.length }} most recent of {{ formatNumber(payouts.length) }} payouts
        </p>
      </div>
    </Card>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import Button from '@/components/common/Button.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import StatCard from '@/components/common/StatCard.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import api from '@/services/api'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber } from '@/utils/formatters'

const { getBarChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const RECENT_LIMIT = 15

const loading = ref(false)
const error = ref(null)
const payouts = ref([])

// Format a balance that is ALREADY in PIV (API returns PIV decimal strings)
const formatBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

// PIVX budget cycle length in blocks. Each passed proposal is paid in its own
// consecutive payout block at the superblock, so a cycle's total is the SUM of
// all payouts whose height falls in the same 43,200-block cycle.
const CYCLE_BLOCKS = 43200

// Aggregate payouts per budget cycle (floor(height / 43200)), oldest first
const cycleTotals = computed(() => {
  const cycles = new Map()
  for (const payout of payouts.value) {
    const index = Math.floor(payout.height / CYCLE_BLOCKS)
    let entry = cycles.get(index)
    if (!entry) {
      entry = { cycle: index, total: 0, payouts: 0, startDate: '', endDate: '' }
      cycles.set(index, entry)
    }
    entry.total += payout.totalPaid
    entry.payouts += 1
    if (payout.date) {
      if (!entry.startDate || payout.date < entry.startDate) entry.startDate = payout.date
      if (!entry.endDate || payout.date > entry.endDate) entry.endDate = payout.date
    }
  }
  return [...cycles.values()]
    .sort((a, b) => a.cycle - b.cycle)
    .map((c) => ({ ...c, total: Math.round(c.total * 100) / 100 }))
})

const cycleDateRange = (cycle) => {
  if (!cycle || !cycle.startDate) return ''
  return cycle.startDate === cycle.endDate
    ? cycle.startDate
    : `${cycle.startDate} → ${cycle.endDate}`
}

const stats = computed(() => {
  if (!payouts.value || payouts.value.length === 0) {
    return {
      totalAllTime: 0,
      lastCycle: 0,
      lastCycleSubtitle: '',
      avgCycle12: 0,
      payoutCount: 0
    }
  }

  const totalAllTime = payouts.value.reduce((sum, p) => sum + p.totalPaid, 0)
  const lastCycle = cycleTotals.value[cycleTotals.value.length - 1]

  const last12 = cycleTotals.value.slice(-12)
  const avgCycle12 = last12.length > 0
    ? last12.reduce((sum, c) => sum + c.total, 0) / last12.length
    : 0

  return {
    totalAllTime,
    lastCycle: lastCycle ? lastCycle.total : 0,
    lastCycleSubtitle: lastCycle
      ? `${cycleDateRange(lastCycle)} · ${lastCycle.payouts} payout${lastCycle.payouts === 1 ? '' : 's'}`
      : '',
    avgCycle12,
    payoutCount: payouts.value.length
  }
})

// Most recent payouts first for the table
const recentPayouts = computed(() =>
  [...payouts.value].slice(-RECENT_LIMIT).reverse()
)

// Per-cycle treasury spending bar chart over the full history
const cycleOption = computed(() => {
  if (cycleTotals.value.length === 0) {
    return getBarChartOption([], [], 'Treasury Paid (PIV)')
  }

  const labels = cycleTotals.value.map(c => c.startDate || `Cycle ${c.cycle}`)
  const totals = cycleTotals.value.map(c => c.total)

  const option = getBarChartOption(labels, totals, 'Treasury Paid (PIV)')
  option.tooltip.formatter = (params) => {
    const p = Array.isArray(params) ? params[0] : params
    const cycle = cycleTotals.value[p.dataIndex]
    const range = cycleDateRange(cycle)
    const count = cycle ? `${cycle.payouts} payout${cycle.payouts === 1 ? '' : 's'}` : ''
    return `${range}<br/>${p.marker}${p.seriesName}: ${formatBalance(p.value)} PIV<br/>${count}`
  }

  return option
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const response = await api.get('/api/v2/analytics/treasury')
    const data = response.data

    if (data && Array.isArray(data)) {
      payouts.value = data.map(p => ({
        height: p.height,
        date: p.date,
        // Already a PIV decimal string — no satoshi conversion
        totalPaid: parseFloat(p.total_paid) || 0,
        nOutputs: p.n_outputs || 0
      }))
    } else {
      payouts.value = []
      error.value = 'No treasury data available'
    }
  } catch (err) {
    error.value = 'Failed to load treasury history. The analytics API may not be available.'
    payouts.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (cycleTotals.value && cycleTotals.value.length > 0) {
    exportToCSV(cycleTotals.value, 'treasury-cycles.csv')
  }
}

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.treasury-history {
  display: grid;
  gap: var(--space-6);
}

.controls {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.controls-note {
  margin: 0;
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

/* 4 tiles: keep rows balanced (4 / 2x2 / 1) instead of wrapping 3+1 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-4);
}

@media (max-width: 1024px) {
  .stats-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 520px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }
}

.table-card {
  padding: 0;
  overflow: hidden;
}

.table-header {
  padding: var(--space-6);
  border-bottom: 1px solid var(--border-subtle);
}

.table-header h3 {
  margin: 0;
  color: var(--text-primary);
}

.loading-state,
.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-12);
  color: var(--text-secondary);
}

.loading-state p {
  margin-top: var(--space-3);
}

.empty-wrapper {
  padding: var(--space-6);
}

.table-container {
  overflow-x: auto;
}

.treasury-table {
  width: 100%;
  border-collapse: collapse;
}

.treasury-table thead {
  border-bottom: 1px solid var(--border-primary);
}

.treasury-table th {
  padding: var(--space-4) var(--space-6);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: rgba(var(--rgb-purple-darkest), 0.92);
}

.treasury-table td {
  padding: var(--space-4) var(--space-6);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
  border-bottom: 1px solid var(--border-subtle);
}

.table-row {
  transition: background-color var(--transition-fast);
}

.table-row:hover {
  background: var(--bg-hover);
}

.height-link {
  font-family: var(--font-mono);
  color: var(--text-accent);
  text-decoration: none;
  transition: color var(--transition-fast);
}

.height-link:hover {
  color: var(--text-primary);
  text-decoration: underline;
}

.date-cell {
  color: var(--text-secondary);
}

.balance {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.text-right {
  text-align: right;
}

.table-footer {
  padding: var(--space-4) var(--space-6);
  border-top: 1px solid var(--border-subtle);
  background: rgba(var(--rgb-purple-darkest), 0.4);
}

.footer-text {
  margin: 0;
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

@media (max-width: 768px) {
  .treasury-table {
    font-size: var(--text-xs);
  }

  .treasury-table th,
  .treasury-table td {
    padding: var(--space-3) var(--space-4);
  }
}
</style>
