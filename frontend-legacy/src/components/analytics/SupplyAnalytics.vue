<template>
  <div class="supply-analytics">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
      <Button variant="ghost" size="sm" @click="exportData">
        💾 Export
      </Button>
    </div>

    <!-- Money Supply Over Time -->
    <BaseChart
      title="Money Supply Over Time"
      :option="moneySupplyOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Transparent vs Shielded Split -->
    <BaseChart
      title="Transparent vs Shielded Supply"
      :option="supplyTypeOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Supply Distribution -->
    <div class="chart-grid">
      <BaseChart
        title="Current Supply Distribution"
        :option="distributionOption"
        :loading="loading"
        :error="error"
        height="350px"
      />
      
      <Card class="stats-card">
        <h3>Supply Metrics</h3>
        <div v-if="loading" class="loading-state">Loading...</div>
        <div v-else-if="error" class="error-state">{{ error }}</div>
        <div v-else-if="!supplyData" class="empty-state">No data available</div>
        <div v-else class="metrics">
          <div class="metric">
            <span class="label">Total Supply</span>
            <span class="value">{{ formatNumber(supplyData.totalSupply) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Transparent</span>
            <span class="value">{{ formatNumber(supplyData.transparent) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Shielded</span>
            <span class="value">{{ formatNumber(supplyData.shielded) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Shield Adoption</span>
            <span class="value accent">{{ formatPercentage(shieldedPercentage) }}%</span>
          </div>
        </div>
      </Card>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import TimeRangeSelector from '@/components/charts/TimeRangeSelector.vue'
import Button from '@/components/common/Button.vue'
import Card from '@/components/common/Card.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { getLineChartOption, getPieChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const supplyData = ref(null)
const historicalData = ref([])

const shieldedPercentage = computed(() => {
  if (!supplyData.value) return 0
  const total = supplyData.value.totalSupply
  const shielded = supplyData.value.shielded
  return total > 0 ? (shielded / total) * 100 : 0
})

// Money Supply Chart Option
const moneySupplyOption = computed(() => {
  if (!historicalData.value || historicalData.value.length === 0) {
    // Show current value as a single point if no historical data
    if (supplyData.value) {
      const today = new Date().toISOString().split('T')[0]
      return getLineChartOption([today], [supplyData.value.totalSupply], 'Total Supply (PIV)')
    }
    return getLineChartOption([], [], 'Total Supply')
  }

  const dates = historicalData.value.map(d => d.date)
  const values = historicalData.value.map(d => d.totalSupply)

  return getLineChartOption(dates, values, 'Total Supply (PIV)')
})

// Transparent vs Shielded Chart Option
const supplyTypeOption = computed(() => {
  if (!historicalData.value || historicalData.value.length === 0) {
    // Show current values as single points if no historical data
    if (supplyData.value) {
      const today = new Date().toISOString().split('T')[0]
      return {
        tooltip: {
          trigger: 'axis',
          backgroundColor: 'rgba(17, 24, 39, 0.95)',
          borderColor: '#374151',
          textStyle: { color: '#E5E7EB' }
        },
        legend: {
          data: ['Transparent', 'Shielded'],
          textStyle: { color: '#9CA3AF' }
        },
        grid: {
          left: '3%',
          right: '4%',
          bottom: '3%',
          top: '15%',
          containLabel: true
        },
        xAxis: {
          type: 'category',
          data: [today],
          axisLine: { lineStyle: { color: '#374151' } },
          axisLabel: { color: '#9CA3AF' }
        },
        yAxis: {
          type: 'value',
          axisLine: { lineStyle: { color: '#374151' } },
          axisLabel: { color: '#9CA3AF' },
          splitLine: { lineStyle: { color: '#374151', type: 'dashed' } }
        },
        series: [
          {
            name: 'Transparent',
            type: 'bar',
            itemStyle: { color: '#59FCB3' },
            data: [supplyData.value.transparent]
          },
          {
            name: 'Shielded',
            type: 'bar',
            itemStyle: { color: '#662D91' },
            data: [supplyData.value.shielded]
          }
        ]
      }
    }
    return {
      ...getLineChartOption([], [], 'Transparent'),
      series: []
    }
  }

  const dates = historicalData.value.map(d => d.date)

  return {
    tooltip: {
      trigger: 'axis',
      backgroundColor: 'rgba(17, 24, 39, 0.95)',
      borderColor: '#374151',
      textStyle: { color: '#E5E7EB' }
    },
    legend: {
      data: ['Transparent', 'Shielded'],
      textStyle: { color: '#9CA3AF' }
    },
    grid: {
      left: '3%',
      right: '4%',
      bottom: '3%',
      top: '15%',
      containLabel: true
    },
    xAxis: {
      type: 'category',
      data: dates,
      axisLine: { lineStyle: { color: '#374151' } },
      axisLabel: { color: '#9CA3AF' }
    },
    yAxis: {
      type: 'value',
      axisLine: { lineStyle: { color: '#374151' } },
      axisLabel: { color: '#9CA3AF' },
      splitLine: { lineStyle: { color: '#374151', type: 'dashed' } }
    },
    series: [
      {
        name: 'Transparent',
        type: 'line',
        stack: 'Total',
        areaStyle: { color: 'rgba(89, 252, 179, 0.3)' },
        lineStyle: { color: '#59FCB3' },
        data: historicalData.value.map(d => d.transparent)
      },
      {
        name: 'Shielded',
        type: 'line',
        stack: 'Total',
        areaStyle: { color: 'rgba(102, 45, 145, 0.3)' },
        lineStyle: { color: '#662D91' },
        data: historicalData.value.map(d => d.shielded)
      }
    ]
  }
})

// Distribution Pie Chart Option
const distributionOption = computed(() => {
  if (!supplyData.value) {
    return getPieChartOption([], 'Supply Distribution')
  }

  const data = [
    { value: supplyData.value.transparent, name: 'Transparent' },
    { value: supplyData.value.shielded, name: 'Shielded' }
  ]

  return getPieChartOption(data, 'Current Supply Distribution')
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const data = await analyticsService.getSupplyAnalytics(timeRange.value)
    console.log('Supply Analytics API Response:', data)
    
    if (data && data.current) {
      // Parse API response (snake_case with string PIV amounts)
      supplyData.value = {
        totalSupply: parseFloat(data.current.total_supply) || 0,
        transparent: parseFloat(data.current.transparent_supply) || 0,
        shielded: parseFloat(data.current.shielded_supply) || 0,
        shieldAdoption: data.current.shield_adoption_percentage || 0
      }
      
      console.log('Parsed Supply Data:', supplyData.value)
      
      // Parse historical data if available
      if (data.historical && data.historical.length > 0) {
        historicalData.value = data.historical.map(point => ({
          date: point.date,
          totalSupply: parseFloat(point.total) || 0,
          transparent: parseFloat(point.transparent) || 0,
          shielded: parseFloat(point.shielded) || 0
        }))
      } else {
        // No historical data available - use current value for display
        historicalData.value = []
      }
    } else {
      throw new Error('Invalid API response')
    }
  } catch (err) {
    console.error('Failed to fetch supply analytics:', err)
    error.value = 'Failed to load supply data from API.'
    // Set empty data on error
    supplyData.value = null
    historicalData.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (historicalData.value && historicalData.value.length > 0) {
    exportToCSV(historicalData.value, `supply-analytics-${timeRange.value}.csv`)
  } else if (supplyData.value) {
    // Export current snapshot if no historical data
    const today = new Date().toISOString().split('T')[0]
    exportToCSV([{
      date: today,
      totalSupply: supplyData.value.totalSupply,
      transparent: supplyData.value.transparent,
      shielded: supplyData.value.shielded
    }], `supply-snapshot-${today}.csv`)
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
.supply-analytics {
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
