<template>
  <div class="rich-list">
    <div class="controls">
      <label class="control-label" for="richlist-limit">Show Top</label>
      <select id="richlist-limit" v-model="limit" class="limit-select">
        <option :value="25">25</option>
        <option :value="50">50</option>
        <option :value="100">100</option>
        <option :value="250">250</option>
      </select>
    </div>

    <div v-if="error" class="error-banner">
      <p>{{ error }}</p>
      <UiButton variant="secondary" @click="fetchData">Try Again</UiButton>
    </div>

    <template v-else>
      <!-- Summary Stats -->
      <div class="stats-grid">
        <StatCard
          label="Top 10 Hold"
          :value="wealth ? wealth.top10.toFixed(2) : ''"
          format="percentage"
          :loading="loading"
        />
        <StatCard
          label="Top 100 Hold"
          :value="wealth ? wealth.top100.toFixed(2) : ''"
          format="percentage"
          :loading="loading"
        />
        <StatCard
          label="Top 1000 Hold"
          :value="wealth ? wealth.top1000.toFixed(2) : ''"
          format="percentage"
          :loading="loading"
        />
        <StatCard
          label="Richest Address"
          :value="richList.length ? formatPIV(richList[0].balance) : ''"
          subtitle="PIV"
          :loading="loading"
        />
      </div>

      <!-- Charts -->
      <div class="chart-grid">
        <BaseChart
          title="Wealth Distribution (% of Supply)"
          :option="distributionOption"
          :loading="loading"
          :empty="!loading && !wealth"
          height="350px"
        />
        <BaseChart
          title="Address Balance Histogram"
          :option="histogramOption"
          :loading="loading"
          :empty="!loading && (!wealth || wealth.histogram.length === 0)"
          height="350px"
        />
      </div>

      <!-- Rich List Table -->
      <UiCard class="table-card">
        <template #header>
          <h3 class="card-title">Top {{ limit }} Addresses by Balance</h3>
        </template>

        <div v-if="loading" class="loading-state">
          <span class="loading-spinner"></span>
          <p>Loading addresses...</p>
        </div>

        <div v-else-if="richList.length === 0" class="empty-note">No rich list data available</div>

        <div v-else class="table-container">
          <table class="rich-list-table">
            <thead>
              <tr>
                <th>Rank</th>
                <th>Address</th>
                <th class="text-right">Balance (PIV)</th>
                <th class="text-right">% of Supply</th>
                <th class="text-right">Transactions</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in richList" :key="item.address">
                <td class="rank">#{{ item.rank }}</td>
                <td class="address-cell">
                  <router-link :to="`/address/${item.address}`" class="address-link mono">
                    {{ item.address }}
                  </router-link>
                </td>
                <td class="text-right balance">{{ formatPIV(item.balance) }}</td>
                <td class="text-right percentage">{{ item.percentage.toFixed(4) }}%</td>
                <td class="text-right">{{ formatNumber(item.txCount) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </UiCard>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import BaseChart from './BaseChart.vue'
import { analyticsService } from '@/services/analyticsService'
import { pieChartOption, barChartOption } from './chartOptions'

const SATS_PER_PIV = 100000000

const limit = ref(100)
const loading = ref(true)
const error = ref('')
const richList = ref([])
const wealth = ref(null)

const formatNumber = (value) => Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })
const formatPIV = (value) =>
  Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 2 })

const distributionOption = computed(() => {
  if (!wealth.value) {
    return pieChartOption([], 'Wealth Distribution')
  }

  const { top10, top50, top100, top1000 } = wealth.value
  const segments = [
    { value: Number(top10.toFixed(2)), name: 'Top 10' },
    { value: Number(Math.max(top50 - top10, 0).toFixed(2)), name: 'Top 11-50' },
    { value: Number(Math.max(top100 - top50, 0).toFixed(2)), name: 'Top 51-100' },
    { value: Number(Math.max(top1000 - top100, 0).toFixed(2)), name: 'Top 101-1000' },
    { value: Number(Math.max(100 - top1000, 0).toFixed(2)), name: 'Others' }
  ]

  const option = pieChartOption(segments, 'Wealth Distribution')
  option.tooltip.formatter = '{b}: {c}% of supply'
  return option
})

const histogramOption = computed(() => {
  if (!wealth.value || wealth.value.histogram.length === 0) {
    return barChartOption([], [], 'Addresses')
  }

  const ranges = wealth.value.histogram.map((h) => h.range)
  const counts = wealth.value.histogram.map((h) => h.count)
  return barChartOption(ranges, counts, 'Number of Addresses')
})

const fetchData = async () => {
  loading.value = true
  error.value = ''

  try {
    const [richListData, wealthData] = await Promise.all([
      analyticsService.getRichList(limit.value),
      analyticsService.getWealthDistribution()
    ])

    richList.value = Array.isArray(richListData)
      ? richListData.map((addr) => ({
          rank: addr.rank,
          address: addr.address,
          // balance is a satoshi string -> convert for display
          balance: (parseFloat(addr.balance) || 0) / SATS_PER_PIV,
          percentage: Number(addr.percentage) || 0,
          txCount: Number(addr.txCount) || 0
        }))
      : []

    wealth.value = wealthData
      ? {
          top10: Number(wealthData.top_10) || 0,
          top50: Number(wealthData.top_50) || 0,
          top100: Number(wealthData.top_100) || 0,
          top1000: Number(wealthData.top_1000) || 0,
          histogram: Array.isArray(wealthData.histogram) ? wealthData.histogram : []
        }
      : null
  } catch (err) {
    richList.value = []
    wealth.value = null
    error.value = 'Failed to load rich list data.'
  } finally {
    loading.value = false
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
  justify-content: flex-end;
  align-items: center;
  gap: var(--space-3);
}

.control-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
}

.limit-select {
  padding: var(--space-2) var(--space-3);
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-family: var(--font-primary);
  font-size: var(--text-sm);
  cursor: pointer;
}

.limit-select:focus {
  outline: none;
  border-color: var(--border-accent);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--space-4);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
}

.card-title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-12);
  color: var(--text-tertiary);
}

.empty-note {
  color: var(--text-tertiary);
  text-align: center;
  padding: var(--space-8) 0;
}

.table-container {
  overflow-x: auto;
}

.rich-list-table {
  width: 100%;
  border-collapse: collapse;
}

.rich-list-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 2px solid var(--border-secondary);
  background: var(--bg-tertiary);
}

.rich-list-table td {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-sm);
  border-bottom: 1px solid var(--border-subtle);
}

.rich-list-table tbody tr:hover {
  background: var(--bg-tertiary);
}

.rank {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
  white-space: nowrap;
}

.mono {
  font-family: var(--font-mono);
}

.address-cell {
  word-break: break-all;
}

.address-link {
  color: var(--text-accent);
  text-decoration: none;
  font-size: var(--text-sm);
}

.address-link:hover {
  color: var(--pivx-accent-dark);
  text-decoration: underline;
}

.balance {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
  white-space: nowrap;
}

.percentage {
  color: var(--text-secondary);
  white-space: nowrap;
}

.text-right {
  text-align: right;
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }

  .rich-list-table th,
  .rich-list-table td {
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-xs);
  }
}
</style>
