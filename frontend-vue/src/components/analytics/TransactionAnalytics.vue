<template>
  <div class="transaction-analytics">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
    </div>

    <div v-if="error" class="error-banner">
      <p>{{ error }}</p>
      <UiButton variant="secondary" @click="fetchData">Try Again</UiButton>
    </div>

    <template v-else>
      <!-- Daily Transaction Count -->
      <BaseChart
        title="Daily Transactions"
        :option="countOption"
        :loading="loading"
        :empty="isEmpty"
        height="400px"
      />

      <div class="chart-grid">
        <BaseChart
          title="Transaction Type Distribution"
          :option="typeDistributionOption"
          :loading="loading"
          :empty="isEmpty"
          height="350px"
        />

        <UiCard>
          <template #header>
            <h3 class="card-title">Transaction Metrics</h3>
          </template>
          <div v-if="loading" class="skeleton metrics-skeleton"></div>
          <div v-else-if="isEmpty" class="empty-note">No data for this range</div>
          <div v-else class="metrics">
            <div class="metric">
              <span class="metric-label">Total Transactions</span>
              <span class="metric-value">{{ formatNumber(metrics.totalTxs) }}</span>
            </div>
            <div class="metric">
              <span class="metric-label">Avg per Day</span>
              <span class="metric-value">{{ formatNumber(metrics.avgPerDay) }}</span>
            </div>
            <div class="metric">
              <span class="metric-label">Total Volume</span>
              <span class="metric-value">{{ formatNumber(metrics.totalVolume) }} PIV</span>
            </div>
            <div class="metric">
              <span class="metric-label">Payment Txs</span>
              <span class="metric-value">{{ metrics.paymentPct.toFixed(1) }}%</span>
            </div>
            <div class="metric">
              <span class="metric-label">Stake Txs</span>
              <span class="metric-value accent">{{ metrics.stakePct.toFixed(1) }}%</span>
            </div>
          </div>
        </UiCard>
      </div>

      <!-- Daily Volume -->
      <BaseChart
        title="Daily Transaction Volume (PIV)"
        :option="volumeOption"
        :loading="loading"
        :empty="isEmpty"
        height="350px"
      />

      <!-- Average Transaction Value -->
      <BaseChart
        title="Average Transaction Value (PIV)"
        :option="avgValueOption"
        :loading="loading"
        :empty="isEmpty"
        height="350px"
      />
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import BaseChart from './BaseChart.vue'
import TimeRangeSelector from './TimeRangeSelector.vue'
import { analyticsService } from '@/services/analyticsService'
import { lineChartOption, barChartOption, pieChartOption } from './chartOptions'

const SATS_PER_PIV = 100000000

const timeRange = ref('30d')
const loading = ref(true)
const error = ref('')
const txData = ref([])

const isEmpty = computed(() => !loading.value && txData.value.length === 0)

const formatNumber = (value) => Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })

const metrics = computed(() => {
  if (txData.value.length === 0) {
    return { totalTxs: 0, avgPerDay: 0, totalVolume: 0, paymentPct: 0, stakePct: 0 }
  }

  const totalTxs = txData.value.reduce((sum, d) => sum + d.count, 0)
  const totalPayment = txData.value.reduce((sum, d) => sum + d.paymentCount, 0)
  const totalStake = txData.value.reduce((sum, d) => sum + d.stakeCount, 0)
  const totalVolume = txData.value.reduce((sum, d) => sum + d.volume, 0)

  return {
    totalTxs,
    avgPerDay: Math.round(totalTxs / txData.value.length),
    totalVolume,
    paymentPct: totalTxs > 0 ? (totalPayment / totalTxs) * 100 : 0,
    stakePct: totalTxs > 0 ? (totalStake / totalTxs) * 100 : 0
  }
})

const countOption = computed(() => {
  const dates = txData.value.map((d) => d.date)
  const values = txData.value.map((d) => d.count)
  return barChartOption(dates, values, 'Transactions')
})

const typeDistributionOption = computed(() => {
  if (txData.value.length === 0) {
    return pieChartOption([], 'Transaction Types')
  }

  const totalPayment = txData.value.reduce((sum, d) => sum + d.paymentCount, 0)
  const totalStake = txData.value.reduce((sum, d) => sum + d.stakeCount, 0)
  const totalOther = txData.value.reduce((sum, d) => sum + d.otherCount, 0)

  return pieChartOption(
    [
      { value: totalPayment, name: 'Payment' },
      { value: totalStake, name: 'Stake' },
      { value: totalOther, name: 'Other' }
    ],
    'Transaction Types'
  )
})

const volumeOption = computed(() => {
  const dates = txData.value.map((d) => d.date)
  const values = txData.value.map((d) => Math.round(d.volume))
  return lineChartOption(dates, values, 'Volume (PIV)')
})

const avgValueOption = computed(() => {
  const dates = txData.value.map((d) => d.date)
  const values = txData.value.map((d) => Number(d.avgValue.toFixed(2)))
  return lineChartOption(dates, values, 'Avg Value (PIV)')
})

const fetchData = async () => {
  loading.value = true
  error.value = ''

  try {
    const data = await analyticsService.getTransactionAnalytics(timeRange.value)

    if (Array.isArray(data)) {
      txData.value = data.map((d) => ({
        date: d.date,
        count: Number(d.count) || 0,
        // volume is a PIV decimal string
        volume: parseFloat(d.volume) || 0,
        paymentCount: Number(d.payment_count) || 0,
        stakeCount: Number(d.stake_count) || 0,
        otherCount: Number(d.other_count) || 0,
        // avg_size / avg_fee are satoshi strings -> convert for display
        avgValue: (parseFloat(d.avg_size) || 0) / SATS_PER_PIV,
        avgFee: (parseFloat(d.avg_fee) || 0) / SATS_PER_PIV
      }))
    } else {
      txData.value = []
    }
  } catch (err) {
    txData.value = []
    error.value = 'Failed to load transaction analytics.'
  } finally {
    loading.value = false
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
  justify-content: flex-end;
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

.metrics {
  display: grid;
  gap: var(--space-3);
}

.metrics-skeleton {
  height: 200px;
}

.metric {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.metric-label {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.metric-value {
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.metric-value.accent {
  color: var(--text-accent);
}

.empty-note {
  color: var(--text-tertiary);
  text-align: center;
  padding: var(--space-8) 0;
}

.error-banner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-8);
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-md);
  color: var(--danger);
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>
