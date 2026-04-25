<template>
  <div class="network-health">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
      <Button variant="ghost" size="sm" @click="exportData">
        💾 Export
      </Button>
    </div>

    <!-- Health Metrics -->
    <div class="stats-grid">
      <StatCard
        label="Chain Difficulty"
        :value="formatNumber(metrics.difficulty)"
        icon="⚡"
        :loading="loading"
      />
      <StatCard
        label="Orphan Rate"
        :value="formatPercentage(metrics.orphanRate)"
        suffix="%"
        icon="🔀"
        :loading="loading"
        :valueClass="metrics.orphanRate < 1 ? 'text-success' : 'text-warning'"
      />
      <StatCard
        label="Avg Block Size"
        :value="formatNumber(metrics.avgBlockSize)"
        suffix="KB"
        icon="📦"
        :loading="loading"
      />
      <StatCard
        label="Blocks Today"
        :value="formatNumber(metrics.blocksToday)"
        icon="🎯"
        :loading="loading"
      />
    </div>

    <!-- Chain Difficulty -->
    <BaseChart
      title="Chain Difficulty Over Time (Log Scale)"
      :option="difficultyOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Charts Grid -->
    <div class="chart-grid">
      <!-- Orphan Rate -->
      <BaseChart
        title="Orphan Rate Percentage"
        :option="orphanRateOption"
        :loading="loading"
        :error="error"
        height="350px"
      />

      <!-- Blocks Per Day -->
      <BaseChart
        title="Blocks Per Day"
        :option="blocksPerDayOption"
        :loading="loading"
        :error="error"
        height="350px"
      />
    </div>

    <!-- Average Block Size -->
    <BaseChart
      title="Average Block Size (KB)"
      :option="blockSizeOption"
      :loading="loading"
      :error="error"
      height="350px"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import TimeRangeSelector from '@/components/charts/TimeRangeSelector.vue'
import Button from '@/components/common/Button.vue'
import StatCard from '@/components/common/StatCard.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { getLineChartOption, getBarChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const healthData = ref([])

const metrics = computed(() => {
  if (!healthData.value || healthData.value.length === 0) {
    return {
      difficulty: 0,
      orphanRate: 0,
      avgBlockSize: 0,
      blocksToday: 0
    }
  }

  const latest = healthData.value[healthData.value.length - 1]
  return {
    difficulty: latest.difficulty,
    orphanRate: latest.orphanRate,
    avgBlockSize: latest.avgBlockSize,
    blocksToday: latest.blocksPerDay
  }
})

// Difficulty Chart (Log Scale)
const difficultyOption = computed(() => {
  if (!healthData.value || healthData.value.length === 0) {
    return getLineChartOption([], [], 'Difficulty')
  }

  const dates = healthData.value.map(d => d.date)
  const values = healthData.value.map(d => d.difficulty)

  const option = getLineChartOption(dates, values, 'Difficulty')
  option.yAxis.type = 'log'
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: (value) => {
      if (value >= 1e9) return (value / 1e9).toFixed(1) + 'B'
      if (value >= 1e6) return (value / 1e6).toFixed(1) + 'M'
      if (value >= 1e3) return (value / 1e3).toFixed(1) + 'K'
      return value.toFixed(0)
    }
  }

  return option
})

// Orphan Rate Chart
const orphanRateOption = computed(() => {
  if (!healthData.value || healthData.value.length === 0) {
    return getLineChartOption([], [], 'Orphan Rate')
  }

  const dates = healthData.value.map(d => d.date)
  const values = healthData.value.map(d => d.orphanRate)

  const option = getLineChartOption(dates, values, 'Orphan Rate (%)')
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: '{value}%'
  }

  // Add warning threshold line at 2%
  option.series.push({
    name: 'Warning Threshold',
    type: 'line',
    data: dates.map(() => 2),
    lineStyle: {
      type: 'dashed',
      color: '#F59E0B',
      width: 2
    },
    itemStyle: {
      color: '#F59E0B'
    },
    symbol: 'none'
  })

  return option
})

// Blocks Per Day Chart
const blocksPerDayOption = computed(() => {
  if (!healthData.value || healthData.value.length === 0) {
    return getBarChartOption([], [], 'Blocks')
  }

  const dates = healthData.value.map(d => d.date)
  const values = healthData.value.map(d => d.blocksPerDay)

  const option = getBarChartOption(dates, values, 'Blocks Per Day')
  
  // Add expected line at 1440 (blocks per day with 60s block time)
  option.series.push({
    name: 'Expected (1440)',
    type: 'line',
    data: dates.map(() => 1440),
    lineStyle: {
      type: 'dashed',
      color: '#10B981',
      width: 2
    },
    itemStyle: {
      color: '#10B981'
    },
    symbol: 'none'
  })

  return option
})

// Block Size Chart
const blockSizeOption = computed(() => {
  if (!healthData.value || healthData.value.length === 0) {
    return getLineChartOption([], [], 'Block Size')
  }

  const dates = healthData.value.map(d => d.date)
  const values = healthData.value.map(d => d.avgBlockSize)

  return getLineChartOption(dates, values, 'Avg Block Size (KB)')
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const data = await analyticsService.getNetworkHealth(timeRange.value)
    
    if (data && Array.isArray(data)) {
      healthData.value = data
    } else {
      // Fallback to mock data
      healthData.value = generateMockHealthData(timeRange.value)
    }
  } catch (err) {
    console.error('Failed to fetch network health:', err)
    error.value = 'Network health API not available. Using mock data.'
    healthData.value = generateMockHealthData(timeRange.value)
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (healthData.value && healthData.value.length > 0) {
    exportToCSV(healthData.value, `network-health-${timeRange.value}.csv`)
  }
}

const generateMockHealthData = (range) => {
  const days = range === '24h' ? 1 : range === '7d' ? 7 : range === '30d' ? 30 : range === '90d' ? 90 : 365
  const data = []
  let baseDifficulty = 1250000000
  
  for (let i = days; i >= 0; i--) {
    const date = new Date()
    date.setDate(date.getDate() - i)
    
    baseDifficulty *= (1 + (Math.random() * 0.02 - 0.01)) // Vary by ±1%
    
    data.push({
      date: date.toISOString().split('T')[0],
      difficulty: baseDifficulty,
      orphanRate: Math.random() * 1.5 + 0.2,
      avgBlockSize: Math.random() * 5 + 8,
      blocksPerDay: 1440 + Math.floor(Math.random() * 40 - 20)
    })
  }

  return data
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
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
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

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>