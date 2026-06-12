<template>
  <div class="rich-list">
    <div class="controls">
      <div class="control-group">
        <label>Show Top:</label>
        <select v-model="limit" class="limit-select">
          <option value="25">25</option>
          <option value="50">50</option>
          <option value="100">100</option>
          <option value="250">250</option>
        </select>
      </div>
      <Button variant="ghost" size="sm" @click="exportData">
        💾 Export
      </Button>
    </div>

    <!-- Summary Stats -->
    <div class="stats-grid">
      <StatCard
        label="Total Addresses"
        :value="formatNumber(stats.totalAddresses)"
        icon="📬"
        :loading="loading"
      />
      <StatCard
        label="Top 100 Hold"
        :value="formatPercentage(stats.top100Percentage)"
        suffix="%"
        icon="💎"
        :loading="loading"
      />
      <StatCard
        label="Richest Address"
        :value="formatBalance(stats.richestBalance)"
        suffix="PIV"
        icon="👑"
        :loading="loading"
      />
      <StatCard
        label="Avg Top 100 Balance"
        :value="formatBalance(stats.avgTop100)"
        suffix="PIV"
        icon="💰"
        :loading="loading"
      />
      <StatCard
        label="Gini Coefficient"
        :value="stats.gini !== null ? stats.gini.toFixed(4) : 'N/A'"
        icon="⚖️"
        :loading="loading"
      />
      <StatCard
        label="Nakamoto Coefficient"
        :value="stats.nakamoto !== null ? formatNumber(stats.nakamoto) : 'N/A'"
        icon="🛡️"
        :loading="loading"
      />
    </div>

    <!-- Wealth Distribution Chart -->
    <div class="chart-grid">
      <BaseChart
        title="Wealth Distribution"
        :option="distributionOption"
        :loading="loading"
        :error="error"
        height="400px"
      />

      <BaseChart
        title="Balance Histogram"
        :option="histogramOption"
        :loading="loading"
        :error="error"
        height="400px"
      />
    </div>

    <!-- Rich List Table -->
    <Card class="table-card">
      <div class="table-header">
        <h3>Top {{ limit }} Addresses by Balance</h3>
      </div>

      <div v-if="loading" class="loading-state">
        <LoadingSpinner />
        <p>Loading addresses...</p>
      </div>

      <div v-else-if="error" class="error-state">
        <p>{{ error }}</p>
      </div>

      <div v-else class="table-container">
        <table class="rich-list-table">
          <thead>
            <tr>
              <th class="sortable" @click="sortBy('rank')">
                Rank<span class="sort-indicator">{{ sortIndicator('rank') }}</span>
              </th>
              <th class="sortable" @click="sortBy('address')">
                Address<span class="sort-indicator">{{ sortIndicator('address') }}</span>
              </th>
              <th class="text-right sortable" @click="sortBy('balance')">
                Balance<span class="sort-indicator">{{ sortIndicator('balance') }}</span>
              </th>
              <th class="text-right sortable" @click="sortBy('percentage')">
                % of Supply<span class="sort-indicator">{{ sortIndicator('percentage') }}</span>
              </th>
              <th class="text-right sortable" @click="sortBy('txCount')">
                Transactions<span class="sort-indicator">{{ sortIndicator('txCount') }}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in sortedRichList" :key="item.address" class="table-row">
              <td class="rank">
                <Badge :variant="getRankBadge(item.rank)">
                  #{{ item.rank }}
                </Badge>
              </td>
              <td class="address-cell">
                <HashDisplay :hash="item.address" :short="true" :copyable="true" />
              </td>
              <td class="text-right balance">
                {{ formatBalance(item.balance) }} PIV
              </td>
              <td class="text-right percentage">
                {{ formatPercentage(item.percentage) }}%
              </td>
              <td class="text-right">
                {{ formatNumber(item.txCount) }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- Pagination -->
      <div v-if="richList.length > 0" class="table-footer">
        <p class="footer-text">
          Showing top {{ richList.length }} addresses
        </p>
      </div>
    </Card>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import Button from '@/components/common/Button.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import StatCard from '@/components/common/StatCard.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { getPieChartOption, getBarChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const limit = ref(100)
const loading = ref(false)
const error = ref(null)
const richList = ref([])
const wealthDistribution = ref(null)

// Format a balance that is ALREADY in PIV (converted once at fetch time)
const formatBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

// Sorting state (default: rank ascending)
const sortKey = ref('rank')
const sortDir = ref('asc')

const sortBy = (key) => {
  if (sortKey.value === key) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortKey.value = key
    sortDir.value = 'asc'
  }
}

const sortIndicator = (key) => {
  if (sortKey.value !== key) return ''
  return sortDir.value === 'asc' ? ' ▲' : ' ▼'
}

const sortedRichList = computed(() => {
  const key = sortKey.value
  const dir = sortDir.value === 'asc' ? 1 : -1
  return [...richList.value].sort((a, b) => {
    if (key === 'address') {
      return a.address.localeCompare(b.address) * dir
    }
    return ((a[key] || 0) - (b[key] || 0)) * dir
  })
})

const stats = computed(() => {
  if (!richList.value || richList.value.length === 0) {
    const wdEmpty = wealthDistribution.value
    return {
      totalAddresses: 0,
      top100Percentage: 0,
      richestBalance: 0,
      avgTop100: 0,
      gini: wdEmpty && typeof wdEmpty.gini === 'number' ? wdEmpty.gini : null,
      nakamoto: wdEmpty && typeof wdEmpty.nakamoto_coefficient === 'number' ? wdEmpty.nakamoto_coefficient : null
    }
  }

  const top100 = richList.value.slice(0, 100)
  const top100Total = top100.reduce((sum, addr) => sum + addr.balance, 0)

  const wd = wealthDistribution.value
  const totalAddresses = wd && Array.isArray(wd.histogram)
    ? wd.histogram.reduce((sum, h) => sum + (h.count || 0), 0)
    : 0
  const top100Percentage = wd && typeof wd.top_100 === 'number'
    ? wd.top_100
    : top100.reduce((sum, addr) => sum + (addr.percentage || 0), 0)

  return {
    totalAddresses,
    top100Percentage,
    richestBalance: richList.value[0]?.balance || 0,
    avgTop100: top100Total / top100.length,
    gini: wd && typeof wd.gini === 'number' ? wd.gini : null,
    nakamoto: wd && typeof wd.nakamoto_coefficient === 'number' ? wd.nakamoto_coefficient : null
  }
})

// Wealth Distribution Pie Chart (uses API percentages of supply)
const distributionOption = computed(() => {
  const wd = wealthDistribution.value
  if (wd && typeof wd.top_10 === 'number') {
    const data = [
      { value: wd.top_10, name: 'Top 10' },
      { value: Math.max(wd.top_50 - wd.top_10, 0), name: 'Top 11-50' },
      { value: Math.max(wd.top_100 - wd.top_50, 0), name: 'Top 51-100' },
      { value: Math.max(100 - wd.top_100, 0), name: 'Others' }
    ]
    return getPieChartOption(data, 'Wealth Distribution')
  }

  if (!richList.value || richList.value.length === 0) {
    return getPieChartOption([], 'Wealth Distribution')
  }

  // Fallback: derive from the API-provided per-address supply percentages
  const pct = (list) => list.reduce((sum, a) => sum + (a.percentage || 0), 0)
  const top10 = pct(richList.value.slice(0, 10))
  const top50 = pct(richList.value.slice(10, 50))
  const top100 = pct(richList.value.slice(50, 100))
  const rest = Math.max(100 - top10 - top50 - top100, 0)

  const data = [
    { value: top10, name: 'Top 10' },
    { value: top50, name: 'Top 11-50' },
    { value: top100, name: 'Top 51-100' },
    { value: rest, name: 'Others' }
  ]

  return getPieChartOption(data, 'Wealth Distribution')
})

// Balance Histogram (from wealth-distribution API)
const histogramOption = computed(() => {
  const wd = wealthDistribution.value
  if (!wd || !Array.isArray(wd.histogram) || wd.histogram.length === 0) {
    return getBarChartOption([], [], 'Number of Addresses')
  }

  const ranges = wd.histogram.map(h => h.range)
  const counts = wd.histogram.map(h => h.count)

  return getBarChartOption(ranges, counts, 'Number of Addresses')
})

const getRankBadge = (rank) => {
  if (rank === 1) return 'warning' // Gold
  if (rank <= 10) return 'info' // Top 10
  return 'default'
}

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    // Fetch rich list from backend (wealth distribution is optional enrichment)
    const [richListData, wealthData] = await Promise.all([
      analyticsService.getRichList(parseInt(limit.value)),
      analyticsService.getWealthDistribution().catch(() => null)
    ])

    if (richListData && Array.isArray(richListData)) {
      richList.value = richListData.map((addr, index) => ({
        rank: addr.rank || index + 1,
        address: addr.address,
        // API returns balance as a satoshi string; convert to PIV exactly once here
        balance: parseFloat(addr.balance) / 100000000,
        percentage: addr.percentage || 0,
        txCount: addr.txCount || 0
      }))
    } else {
      // Fallback to empty if API returns unexpected format
      richList.value = []
      error.value = 'No rich list data available'
    }

    wealthDistribution.value = wealthData
  } catch (err) {
    error.value = err.message || 'Failed to load rich list data. The analytics API may not be available yet.'
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (sortedRichList.value && sortedRichList.value.length > 0) {
    const exportData = sortedRichList.value.map((item) => ({
      rank: item.rank,
      address: item.address,
      balance: item.balance,
      percentage: item.percentage,
      txCount: item.txCount
    }))
    exportToCSV(exportData, `rich-list-top-${limit.value}.csv`)
  }
}

watch(limit, () => {
  fetchData()
})

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.rich-list {
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

.control-group {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.control-group label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.limit-select {
  padding: var(--space-2) var(--space-3);
  background: rgba(var(--rgb-purple-darkest), 0.55);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.limit-select:hover {
  border-color: rgba(var(--rgb-purple-accent), 0.45);
}

.limit-select:focus {
  outline: none;
  border-color: var(--border-accent);
  box-shadow: var(--focus-ring-glow);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
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

.table-container {
  overflow-x: auto;
}

.rich-list-table {
  width: 100%;
  border-collapse: collapse;
}

.rich-list-table thead {
  border-bottom: 1px solid var(--border-primary);
}

.rich-list-table th {
  padding: var(--space-4) var(--space-6);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: rgba(var(--rgb-purple-darkest), 0.92);
  position: sticky;
  top: 0;
  z-index: 1;
}

.rich-list-table th.sortable {
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}

.rich-list-table th.sortable:hover {
  color: var(--text-primary);
}

.sort-indicator {
  color: var(--text-accent);
  font-size: var(--text-xs);
}

.rich-list-table td {
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

.rank {
  width: 80px;
}

.address-cell {
  font-family: var(--font-mono);
}

.balance {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.percentage {
  color: var(--text-secondary);
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
  .chart-grid {
    grid-template-columns: 1fr;
  }

  .rich-list-table {
    font-size: var(--text-xs);
  }

  .rich-list-table th,
  .rich-list-table td {
    padding: var(--space-3) var(--space-4);
  }
}
</style>