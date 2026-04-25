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
        :value="formatPIV(stats.richestBalance)"
        suffix="PIV"
        icon="👑"
        :loading="loading"
      />
      <StatCard
        label="Avg Top 100 Balance"
        :value="formatPIV(stats.avgTop100)"
        suffix="PIV"
        icon="💰"
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
              <th>Rank</th>
              <th>Address</th>
              <th class="text-right">Balance</th>
              <th class="text-right">% of Supply</th>
              <th class="text-right">Transactions</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in richList" :key="item.address" class="table-row">
              <td class="rank">
                <Badge :variant="getRankBadge(index + 1)">
                  #{{ index + 1 }}
                </Badge>
              </td>
              <td class="address-cell">
                <HashDisplay :hash="item.address" :short="true" :copyable="true" />
              </td>
              <td class="text-right balance">
                {{ formatPIV(item.balance) }} PIV
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
import { formatNumber, formatPercentage, formatPIV } from '@/utils/formatters'

const { getPieChartOption, getBarChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const limit = ref(100)
const loading = ref(false)
const error = ref(null)
const richList = ref([])
const wealthDistribution = ref(null)

const totalSupply = 70000000 // Will be updated from API

const stats = computed(() => {
  if (!richList.value || richList.value.length === 0) {
    return {
      totalAddresses: 0,
      top100Percentage: 0,
      richestBalance: 0,
      avgTop100: 0
    }
  }

  const top100 = richList.value.slice(0, 100)
  const top100Total = top100.reduce((sum, addr) => sum + addr.balance, 0)

  return {
    totalAddresses: 125000, // Mock value
    top100Percentage: (top100Total / totalSupply) * 100,
    richestBalance: richList.value[0]?.balance || 0,
    avgTop100: top100Total / top100.length
  }
})

// Wealth Distribution Pie Chart
const distributionOption = computed(() => {
  if (!richList.value || richList.value.length === 0) {
    return getPieChartOption([], 'Wealth Distribution')
  }

  const top10 = richList.value.slice(0, 10).reduce((sum, a) => sum + a.balance, 0)
  const top50 = richList.value.slice(10, 50).reduce((sum, a) => sum + a.balance, 0)
  const top100 = richList.value.slice(50, 100).reduce((sum, a) => sum + a.balance, 0)
  const rest = totalSupply - top10 - top50 - top100

  const data = [
    { value: top10, name: 'Top 10' },
    { value: top50, name: 'Top 11-50' },
    { value: top100, name: 'Top 51-100' },
    { value: rest, name: 'Others' }
  ]

  return getPieChartOption(data, 'Wealth Distribution')
})

// Balance Histogram
const histogramOption = computed(() => {
  const ranges = ['0-10K', '10K-50K', '50K-100K', '100K-500K', '500K+']
  const counts = [42000, 18000, 8000, 2500, 450]

  return getBarChartOption(ranges, counts, 'Number of Addresses')
})

const getRankBadge = (rank) => {
  if (rank === 1) return 'warning' // Gold
  if (rank <= 10) return 'info' // Top 10
  if (rank <= 50) return 'secondary'
  return 'secondary'
}

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    // Fetch rich list from backend
    const [richListData, wealthData] = await Promise.all([
      analyticsService.getRichList(parseInt(limit.value)),
      analyticsService.getWealthDistribution()
    ])

    if (richListData && Array.isArray(richListData)) {
      richList.value = richListData.map(addr => ({
        address: addr.address,
        balance: parseFloat(addr.balance) / 100000000, // Convert from satoshis to PIV
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
    console.error('Failed to fetch rich list:', err)
    error.value = err.message || 'Failed to load rich list data. The analytics API may not be available yet.'
    // Don't show empty list, keep any existing data
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (richList.value && richList.value.length > 0) {
    const exportData = richList.value.map((item, index) => ({
      rank: index + 1,
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
  background: var(--card-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: var(--text-sm);
  cursor: pointer;
}

.limit-select:focus {
  outline: none;
  border-color: var(--text-accent);
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
  border-bottom: 1px solid var(--border-color);
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
  background: rgba(255, 255, 255, 0.03);
  border-bottom: 2px solid var(--border-color);
}

.rich-list-table th {
  padding: var(--space-4) var(--space-6);
  text-align: left;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.rich-list-table td {
  padding: var(--space-4) var(--space-6);
  font-size: var(--text-sm);
  border-bottom: 1px solid var(--border-color);
}

.table-row:hover {
  background: rgba(255, 255, 255, 0.03);
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
  border-top: 1px solid var(--border-color);
  background: rgba(255, 255, 255, 0.02);
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