<template>
  <div class="network-health">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
    </div>

    <div v-if="error" class="error-banner">
      <p>{{ error }}</p>
      <UiButton variant="secondary" @click="fetchData">Try Again</UiButton>
    </div>

    <template v-else>
      <!-- Summary Stats (latest day) -->
      <div class="stats-grid">
        <StatCard
          label="Chain Difficulty"
          :value="latest ? formatCompact(latest.difficulty) : ''"
          :loading="loading"
        />
        <StatCard
          label="Orphan Rate"
          :value="latest ? latest.orphanRate.toFixed(2) : ''"
          format="percentage"
          :loading="loading"
        />
        <StatCard
          label="Blocks Per Day"
          :value="latest ? latest.blocksPerDay : ''"
          format="number"
          :loading="loading"
        />
        <StatCard
          label="Days Tracked"
          :value="healthData.length || ''"
          format="number"
          :loading="loading"
        />
      </div>

      <!-- Difficulty -->
      <BaseChart
        title="Chain Difficulty Over Time"
        :option="difficultyOption"
        :loading="loading"
        :empty="isEmpty"
        height="400px"
      />

      <div class="chart-grid">
        <BaseChart
          title="Orphan Rate (%)"
          :option="orphanRateOption"
          :loading="loading"
          :empty="isEmpty"
          height="350px"
        />

        <BaseChart
          title="Blocks Per Day"
          :option="blocksPerDayOption"
          :loading="loading"
          :empty="isEmpty"
          height="350px"
        />
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import StatCard from '@/components/common/StatCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import BaseChart from './BaseChart.vue'
import TimeRangeSelector from './TimeRangeSelector.vue'
import { analyticsService } from '@/services/analyticsService'
import { lineChartOption, barChartOption, referenceLineSeries, chartColors } from './chartOptions'

const timeRange = ref('30d')
const loading = ref(true)
const error = ref('')
const healthData = ref([])

const isEmpty = computed(() => !loading.value && healthData.value.length === 0)
const latest = computed(() => healthData.value[healthData.value.length - 1] || null)

const formatCompact = (value) => {
  const num = Number(value) || 0
  if (num >= 1e9) return `${(num / 1e9).toFixed(2)}B`
  if (num >= 1e6) return `${(num / 1e6).toFixed(2)}M`
  if (num >= 1e3) return `${(num / 1e3).toFixed(2)}K`
  return num.toLocaleString()
}

const difficultyOption = computed(() => {
  const dates = healthData.value.map((d) => d.date)
  const values = healthData.value.map((d) => d.difficulty)

  const option = lineChartOption(dates, values, 'Difficulty')
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: (value) => formatCompact(value)
  }
  return option
})

const orphanRateOption = computed(() => {
  const dates = healthData.value.map((d) => d.date)
  const values = healthData.value.map((d) => Number(d.orphanRate.toFixed(3)))

  const option = lineChartOption(dates, values, 'Orphan Rate (%)')
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: '{value}%'
  }
  if (dates.length > 0) {
    // 2% warning threshold
    option.series.push(referenceLineSeries(dates, 2, 'Warning (2%)'))
  }
  return option
})

const blocksPerDayOption = computed(() => {
  const dates = healthData.value.map((d) => d.date)
  const values = healthData.value.map((d) => d.blocksPerDay)

  const option = barChartOption(dates, values, 'Blocks Per Day')
  if (dates.length > 0) {
    // Expected 1440 blocks/day at 60s block time
    option.series.push(referenceLineSeries(dates, 1440, 'Expected (1440)', chartColors.accentDark))
  }
  return option
})

const fetchData = async () => {
  loading.value = true
  error.value = ''

  try {
    const data = await analyticsService.getNetworkHealth(timeRange.value)

    if (Array.isArray(data)) {
      healthData.value = data.map((d) => ({
        date: d.date,
        difficulty: parseFloat(d.difficulty) || 0,
        orphanRate: Number(d.orphan_rate) || 0,
        blocksPerDay: Number(d.blocks_per_day) || 0
      }))
    } else {
      healthData.value = []
    }
  } catch (err) {
    healthData.value = []
    error.value = 'Failed to load network health data.'
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
.network-health {
  display: grid;
  gap: var(--space-6);
}

.controls {
  display: flex;
  justify-content: flex-end;
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
