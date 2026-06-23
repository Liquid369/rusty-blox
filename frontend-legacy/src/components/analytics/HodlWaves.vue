<template>
  <div class="hodl-waves">
    <div class="controls">
      <p class="controls-note">Supply grouped by time since coins last moved</p>
      <Button variant="ghost" size="sm" @click="exportData">
        <Icon name="download" :size="14" /> Export
      </Button>
    </div>

    <!-- Summary Stats -->
    <div class="stats-grid">
      <StatCard
        label="Unmoved > 1 Year"
        :value="formatPercentage(stats.unmovedOverYear)"
        suffix="%"
        icon="snowflake"
        :loading="loading"
      />
      <StatCard
        label="Unmoved > 2 Years"
        :value="formatPercentage(stats.unmovedOverTwoYears)"
        suffix="%"
        icon="gem"
        :loading="loading"
      />
      <StatCard
        label="Moved < 1 Month"
        :value="formatPercentage(stats.movedUnderMonth)"
        suffix="%"
        icon="flame"
        :loading="loading"
      />
      <StatCard
        label="Tracked Supply"
        :value="formatBalance(stats.total)"
        suffix="PIV"
        icon="box"
        :loading="loading"
      />
    </div>

    <!-- Age Distribution Donut -->
    <BaseChart
      title="Supply by Coin Age"
      :option="donutOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Age Bands Table -->
    <Card class="table-card">
      <div class="table-header">
        <h3>Age Bands</h3>
      </div>

      <div v-if="loading" class="loading-state">
        <LoadingSpinner />
        <p>Loading HODL waves...</p>
      </div>

      <div v-else-if="error" class="error-state">
        <p>{{ error }}</p>
      </div>

      <div v-else-if="bands.length === 0" class="empty-wrapper">
        <EmptyState
          icon="waves"
          title="No HODL Data"
          message="Coin age data is not available yet."
        />
      </div>

      <div v-else class="table-container">
        <table class="hodl-table">
          <thead>
            <tr>
              <th>Age Band</th>
              <th class="text-right">Supply</th>
              <th class="text-right">% of Supply</th>
              <th>Share</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="band in bands" :key="band.band" class="table-row">
              <td>
                <Badge :variant="getBandBadge(band.band)">
                  {{ band.band }}
                </Badge>
              </td>
              <td class="text-right balance">
                {{ formatBalance(band.value) }} PIV
              </td>
              <td class="text-right percentage">
                {{ formatPercentage(band.percentage) }}%
              </td>
              <td class="share-cell">
                <div class="share-track">
                  <div class="share-fill" :style="{ width: `${Math.min(band.percentage, 100)}%` }"></div>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
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
import { formatPercentage } from '@/utils/formatters'

const { getPieChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const loading = ref(false)
const error = ref(null)
const bands = ref([])
const total = ref(0)

// Format a balance that is ALREADY in PIV (API returns PIV decimal strings)
const formatBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

const stats = computed(() => {
  if (!bands.value || bands.value.length === 0) {
    return {
      unmovedOverYear: 0,
      unmovedOverTwoYears: 0,
      movedUnderMonth: 0,
      total: 0
    }
  }

  const pct = (name) => bands.value.find(b => b.band === name)?.percentage || 0

  return {
    unmovedOverYear: pct('1-2y') + pct('>2y'),
    unmovedOverTwoYears: pct('>2y'),
    movedUnderMonth: pct('<1m'),
    total: total.value
  }
})

// Donut chart of supply by age band
const donutOption = computed(() => {
  if (!bands.value || bands.value.length === 0) {
    return getPieChartOption([], 'Supply by Coin Age')
  }

  const data = bands.value.map(b => ({
    name: b.band,
    value: Math.round(b.value * 100) / 100
  }))

  const option = getPieChartOption(data, 'Supply by Coin Age')
  option.tooltip.formatter = (params) =>
    `${params.name}: ${formatBalance(params.value)} PIV (${params.percent}%)`

  return option
})

const getBandBadge = (band) => {
  switch (band) {
    case '<1m': return 'danger'
    case '1-3m': return 'warning'
    case '3-6m': return 'info'
    case '6-12m': return 'default'
    case '1-2y': return 'accent'
    case '>2y': return 'success'
    default: return 'default'
  }
}

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const response = await api.get('/api/v2/analytics/hodl')
    const data = response.data

    if (data && Array.isArray(data.bands)) {
      bands.value = data.bands.map(b => ({
        band: b.band,
        // Already a PIV decimal string — no satoshi conversion
        value: parseFloat(b.value) || 0,
        percentage: b.percentage || 0
      }))
      total.value = parseFloat(data.total) || 0
    } else {
      bands.value = []
      total.value = 0
      error.value = 'No HODL wave data available'
    }
  } catch (err) {
    error.value = 'Failed to load HODL waves. The analytics API may not be available.'
    bands.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (bands.value && bands.value.length > 0) {
    exportToCSV(bands.value, 'hodl-waves.csv')
  }
}

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.hodl-waves {
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

.hodl-table {
  width: 100%;
  border-collapse: collapse;
}

.hodl-table thead {
  border-bottom: 1px solid var(--border-primary);
}

.hodl-table th {
  padding: var(--space-4) var(--space-6);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: rgba(var(--rgb-purple-darkest), 0.92);
}

.hodl-table td {
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

.balance {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.percentage {
  color: var(--text-secondary);
}

.share-cell {
  width: 30%;
  min-width: 140px;
}

.share-track {
  height: 8px;
  border-radius: var(--radius-full);
  background: rgba(var(--rgb-purple-darkest), 0.55);
  border: 1px solid var(--border-subtle);
  overflow: hidden;
}

.share-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: linear-gradient(90deg, var(--pivx-purple-primary), #B3FF78);
  transition: width var(--transition-base);
}

.text-right {
  text-align: right;
}

@media (max-width: 768px) {
  .hodl-table {
    font-size: var(--text-xs);
  }

  .hodl-table th,
  .hodl-table td {
    padding: var(--space-3) var(--space-4);
  }
}
</style>
