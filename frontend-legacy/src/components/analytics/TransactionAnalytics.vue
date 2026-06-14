<template>
  <div class="transaction-analytics">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
      <Button variant="ghost" size="sm" @click="exportData">
        <Icon name="download" :size="14" /> Export
      </Button>
    </div>

    <!-- Daily Transactions by Type -->
    <div class="typed-chart">
      <div class="series-toggles" role="group" aria-label="Toggle transaction series">
        <button
          v-for="series in SERIES_DEFS"
          :key="series.key"
          type="button"
          :class="['series-pill', { active: visibleSeries[series.key] }]"
          :style="{ '--series-color': series.color }"
          :aria-pressed="visibleSeries[series.key]"
          @click="toggleSeries(series.key)"
        >
          <span class="series-dot" aria-hidden="true"></span>
          {{ series.label }}
        </button>
      </div>
      <BaseChart
        title="Daily Transactions by Type"
        :option="dailyTypeOption"
        :loading="loading"
        :error="error"
        height="400px"
      />
    </div>

    <!-- Daily Transaction Volume -->
    <BaseChart
      title="Daily Transaction Volume"
      :option="volumeChartOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Transaction Type Distribution -->
    <div class="chart-grid">
      <BaseChart
        title="Transaction Type Distribution"
        :option="typeDistributionOption"
        :loading="loading"
        :error="error"
        height="350px"
      />
      
      <Card class="stats-card">
        <h3>Transaction Metrics</h3>
        <div v-if="loading" class="state-message">Loading...</div>
        <div v-else-if="error" class="state-message">{{ error }}</div>
        <div v-else-if="txData.length === 0" class="state-message">No data available</div>
        <div v-else class="metrics">
          <div class="metric">
            <span class="label">Total Transactions</span>
            <span class="value">{{ formatNumber(metrics.totalTxs) }}</span>
          </div>
          <div class="metric">
            <span class="label">Avg per Day</span>
            <span class="value">{{ formatNumber(metrics.avgPerDay) }}</span>
          </div>
          <div class="metric">
            <span class="label">Payment Txs</span>
            <span class="value">{{ formatPercentage(metrics.paymentPct) }}%</span>
          </div>
          <div class="metric">
            <span class="label">Stake Txs</span>
            <span class="value accent">{{ formatPercentage(metrics.stakePct) }}%</span>
          </div>
        </div>
      </Card>
    </div>

    <!-- Average Transaction Size Trend -->
    <BaseChart
      title="Average Transaction Size (PIV)"
      :option="avgSizeOption"
      :loading="loading"
      :error="error"
      height="350px"
    />
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import TimeRangeSelector from '@/components/charts/TimeRangeSelector.vue'
import Button from '@/components/common/Button.vue'
import Card from '@/components/common/Card.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartConfig, useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { colors, getBaseOption } = useChartConfig()
const { getBarChartOption, getLineChartOption, getPieChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const txData = ref([])

// Daily series: distinct brand colors so the types are separable.
// Cold Stake counts coinstakes won by P2CS delegations (subset of Coinstake).
const SERIES_DEFS = [
  { key: 'transparent', label: 'Transparent', field: 'payment', color: colors.info },
  { key: 'coinstake', label: 'Coinstake', field: 'stake', color: colors.primary },
  { key: 'coldstake', label: 'Cold Stake', field: 'coldstake', color: colors.warning },
  { key: 'shield', label: 'Shield', field: 'shield', color: colors.accent }
]

const visibleSeries = ref({ transparent: true, coinstake: true, coldstake: true, shield: true })

const toggleSeries = (key) => {
  visibleSeries.value = { ...visibleSeries.value, [key]: !visibleSeries.value[key] }
}

const metrics = computed(() => {
  if (!txData.value || txData.value.length === 0) {
    return { totalTxs: 0, avgPerDay: 0, paymentPct: 0, stakePct: 0 }
  }

  const totalTxs = txData.value.reduce((sum, d) => sum + d.total, 0)
  const avgPerDay = Math.round(totalTxs / txData.value.length)
  const totalPayment = txData.value.reduce((sum, d) => sum + d.payment, 0)
  const totalStake = txData.value.reduce((sum, d) => sum + d.stake, 0)

  return {
    totalTxs,
    avgPerDay,
    paymentPct: totalTxs > 0 ? (totalPayment / totalTxs) * 100 : 0,
    stakePct: totalTxs > 0 ? (totalStake / totalTxs) * 100 : 0
  }
})

// Daily Transactions by Type: one brand-colored line per series. Hidden
// series keep their slot with empty data so ECharts option merging clears
// the line when a pill is toggled off.
const dailyTypeOption = computed(() => {
  const base = getBaseOption()
  const dates = txData.value.map(d => d.date)

  return {
    ...base,
    xAxis: {
      ...base.xAxis,
      data: dates
    },
    series: SERIES_DEFS.map(s => ({
      name: s.label,
      type: 'line',
      smooth: true,
      showSymbol: false,
      data: visibleSeries.value[s.key] ? txData.value.map(d => d[s.field]) : [],
      lineStyle: { color: s.color, width: 2 },
      itemStyle: { color: s.color },
      emphasis: { focus: 'series' }
    }))
  }
})

// Daily Volume Chart (volume from the API is already in PIV)
const volumeChartOption = computed(() => {
  if (!txData.value || txData.value.length === 0) {
    return getBarChartOption([], [], 'Volume (PIV)')
  }

  const dates = txData.value.map(d => d.date)
  const values = txData.value.map(d => d.volume)

  return getBarChartOption(dates, values, 'Volume (PIV)')
})

// Transaction Type Distribution
const typeDistributionOption = computed(() => {
  if (!txData.value || txData.value.length === 0) {
    return getPieChartOption([], 'Transaction Types')
  }

  const totalPayment = txData.value.reduce((sum, d) => sum + d.payment, 0)
  const totalStake = txData.value.reduce((sum, d) => sum + d.stake, 0)
  const totalOther = txData.value.reduce((sum, d) => sum + d.other, 0)

  const data = [
    { value: totalPayment, name: 'Payment' },
    { value: totalStake, name: 'Stake' },
    { value: totalOther, name: 'Other' }
  ]

  return getPieChartOption(data, 'Transaction Types')
})

// Average Size Trend
const avgSizeOption = computed(() => {
  if (!txData.value || txData.value.length === 0) {
    return getLineChartOption([], [], 'Avg Size')
  }

  const dates = txData.value.map(d => d.date)
  const values = txData.value.map(d => d.avgSize)

  return getLineChartOption(dates, values, 'Average Size (PIV)')
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const data = await analyticsService.getTransactionAnalytics(timeRange.value)

    if (data && Array.isArray(data)) {
      txData.value = data.map(d => ({
        date: d.date,
        total: d.count || 0,
        payment: d.payment_count || 0,
        stake: d.stake_count || 0,
        other: d.other_count || 0,
        shield: d.sapling_txs || 0,
        coldstake: d.coldstake_txs || 0,
        // volume is already a PIV decimal string — no satoshi conversion
        volume: parseFloat(d.volume) || 0,
        // avg_size is a satoshi string — convert to PIV exactly once here
        avgSize: (parseFloat(d.avg_size) || 0) / 100000000
      }))
    } else {
      txData.value = []
      error.value = 'No transaction analytics data available'
    }
  } catch (err) {
    error.value = 'Failed to load transaction analytics. The analytics API may not be available.'
    txData.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (txData.value && txData.value.length > 0) {
    exportToCSV(txData.value, `transaction-analytics-${timeRange.value}.csv`)
  }
}

watch(timeRange, () => {
  fetchData()
})

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.transaction-analytics {
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

.typed-chart {
  display: grid;
  gap: var(--space-3);
}

/* Series toggle pills — consistent with TimeRangeSelector buttons */
.series-toggles {
  display: flex;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.series-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  background: rgba(var(--rgb-purple-dark), 0.5);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-full);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast);
}

.series-pill:hover {
  background: rgba(var(--rgb-purple-mid), 0.6);
  border-color: rgba(var(--rgb-purple-accent), 0.5);
  color: var(--text-primary);
}

.series-pill:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

.series-pill .series-dot {
  width: 10px;
  height: 10px;
  border-radius: var(--radius-full);
  background: var(--text-tertiary);
  transition: background-color var(--transition-fast), box-shadow var(--transition-fast);
}

.series-pill.active {
  border-color: var(--series-color);
  color: var(--text-primary);
  font-weight: var(--weight-bold);
}

.series-pill.active .series-dot {
  background: var(--series-color);
  box-shadow: 0 0 6px var(--series-color);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
}

.stats-card {
  padding: var(--space-6);
}

.stats-card h3 {
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.state-message {
  padding: var(--space-4);
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.metrics {
  display: grid;
  gap: var(--space-4);
}

.metric {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3);
  background: rgba(255, 255, 255, 0.03);
  border-radius: var(--radius-md);
}

.metric .label {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.metric .value {
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.metric .value.accent {
  color: var(--text-accent);
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>