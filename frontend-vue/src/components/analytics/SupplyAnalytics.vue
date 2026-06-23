<template>
  <div class="supply-analytics">
    <!-- Summary Stats -->
    <div class="stats-grid">
      <StatCard
        label="Total Supply"
        :value="supply ? formatPIV(supply.totalSupply) : ''"
        subtitle="PIV"
        :loading="loading"
      />
      <StatCard
        label="Transparent Supply"
        :value="supply ? formatPIV(supply.transparent) : ''"
        subtitle="PIV"
        :loading="loading"
      />
      <StatCard
        label="Shielded Supply"
        :value="supply ? formatPIV(supply.shielded) : ''"
        subtitle="PIV"
        :loading="loading"
      />
      <StatCard
        label="Shield Adoption"
        :value="supply ? supply.shieldAdoption.toFixed(2) : ''"
        format="percentage"
        :loading="loading"
      />
    </div>

    <div v-if="error" class="error-banner">
      <p>{{ error }}</p>
      <UiButton variant="secondary" @click="fetchData">Try Again</UiButton>
    </div>

    <template v-else>
      <!-- Historical supply (only when the backend provides history) -->
      <BaseChart
        v-if="historical.length > 0"
        title="Money Supply Over Time"
        :option="historyOption"
        :loading="loading"
        height="400px"
      />

      <div class="chart-grid">
        <BaseChart
          title="Current Supply Distribution"
          :option="distributionOption"
          :loading="loading"
          :empty="!loading && !supply"
          height="350px"
        />

        <UiCard>
          <template #header>
            <h3 class="card-title">Supply Snapshot</h3>
          </template>
          <div v-if="loading" class="skeleton metrics-skeleton"></div>
          <div v-else-if="!supply" class="empty-note">No supply data available</div>
          <div v-else class="metrics">
            <div class="metric">
              <span class="metric-label">Total Supply</span>
              <span class="metric-value">{{ formatPIV(supply.totalSupply) }} PIV</span>
            </div>
            <div class="metric">
              <span class="metric-label">Transparent</span>
              <span class="metric-value">{{ formatPIV(supply.transparent) }} PIV</span>
            </div>
            <div class="metric">
              <span class="metric-label">Shielded</span>
              <span class="metric-value">{{ formatPIV(supply.shielded) }} PIV</span>
            </div>
            <div class="metric">
              <span class="metric-label">Shield Adoption</span>
              <span class="metric-value accent">{{ supply.shieldAdoption.toFixed(2) }}%</span>
            </div>
            <p class="metric-note">
              Historical supply tracking will appear here once enough snapshots are collected.
            </p>
          </div>
        </UiCard>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import BaseChart from './BaseChart.vue'
import { analyticsService } from '@/services/analyticsService'
import { lineChartOption, pieChartOption } from './chartOptions'

const loading = ref(true)
const error = ref('')
const supply = ref(null)
const historical = ref([])

const formatPIV = (value) => {
  return Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })
}

const historyOption = computed(() => {
  const dates = historical.value.map((d) => d.date)
  const values = historical.value.map((d) => d.totalSupply)
  return lineChartOption(dates, values, 'Total Supply (PIV)')
})

const distributionOption = computed(() => {
  if (!supply.value) {
    return pieChartOption([], 'Supply Distribution')
  }
  return pieChartOption(
    [
      { value: Math.round(supply.value.transparent), name: 'Transparent' },
      { value: Math.round(supply.value.shielded), name: 'Shielded' }
    ],
    'Supply Distribution'
  )
})

const fetchData = async () => {
  loading.value = true
  error.value = ''

  try {
    const data = await analyticsService.getSupplyAnalytics()

    if (data && data.current) {
      // API returns PIV amounts as decimal strings
      supply.value = {
        totalSupply: parseFloat(data.current.total_supply) || 0,
        transparent: parseFloat(data.current.transparent_supply) || 0,
        shielded: parseFloat(data.current.shielded_supply) || 0,
        shieldAdoption: Number(data.current.shield_adoption_percentage) || 0
      }

      historical.value = Array.isArray(data.historical)
        ? data.historical.map((point) => ({
            date: point.date,
            totalSupply: parseFloat(point.total) || 0
          }))
        : []
    } else {
      supply.value = null
      historical.value = []
      error.value = 'Supply data is not available right now.'
    }
  } catch (err) {
    supply.value = null
    historical.value = []
    error.value = 'Failed to load supply analytics.'
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.supply-analytics {
  display: grid;
  gap: var(--space-6);
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

.metric-note {
  margin: var(--space-2) 0 0;
  font-size: var(--text-xs);
  color: var(--text-tertiary);
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
